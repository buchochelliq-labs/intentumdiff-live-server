"""Extract only verified Wasm components from an immutable reviewed CI wheel.

Python is a build-time archive utility; the resulting native distribution contains
only the Rust executable, Wasm components and provenance, with no Python runtime.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import urllib.request
import urllib.parse
import zipfile

IDENTITY = {
    "python_commit": "d46fd0e8fb3c65bb9d77ab14c95ebcb7c5798d54",
    "component_core_commit": "89c11a806b3f6368a3f502d77aa35698cdf38cba",
    "wheel_sha256": "1919afa822f5e5bc9b3106d88a0e08c1795dbdd78da99662625f61e9d503faf5",
    "artifact_id": 11514272993,
    "archive_sha256": "9f3f63c54cde55b1e7e0cc55073eb20df28753098d215823510ba692a1b991e0",
    "workflow_run": 37689207055,
}


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        redirected = super().redirect_request(request, fp, code, message, headers, new_url)
        if urllib.parse.urlsplit(new_url).scheme != "https":
            raise ValueError("refusing non-HTTPS artifact redirect")
        if redirected is not None:
            redirected.remove_header("Authorization")
        return redirected


def checked(data, expected, name):
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise ValueError(f"{name} checksum mismatch: {actual}")
    return data


def provision(wheel, destination):
    checked(wheel, IDENTITY["wheel_sha256"], "reviewed wheel")
    with zipfile.ZipFile(io.BytesIO(wheel)) as archive:
        members = {
            name.removeprefix("intentumdiff/wasm/"): archive.read(name)
            for name in archive.namelist()
            if name.startswith("intentumdiff/wasm/") and not name.endswith("/")
        }
    if any("/" in name or "\\" in name or name in (".", "..") for name in members):
        raise ValueError("unexpected nested component path")
    manifest = json.loads(members["wasm_provenance.json"])
    artifacts = manifest["artifacts"]
    actual = {name for name in members if name.endswith(".wasm")}
    if actual != set(artifacts) or not actual:
        raise ValueError("component manifest does not match archive")
    for name, item in artifacts.items():
        checked(members[name], item["sha256"], name)
        if len(members[name]) != item["size_bytes"]:
            raise ValueError(f"{name} size mismatch")
    if not json.loads(members["parser_manifest.json"]):
        raise ValueError("empty parser manifest")
    destination.mkdir(parents=True, exist_ok=True)
    if any(destination.iterdir()):
        raise ValueError("component destination must be empty")
    for name, content in members.items():
        (destination / name).write_bytes(content)
    (destination / "component-source.json").write_text(json.dumps(IDENTITY, indent=2) + "\n")
    print(f"Verified and staged {len(artifacts)} components in {destination}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel", type=Path, help="offline copy of the exact reviewed wheel")
    parser.add_argument("--output", type=Path, default=Path("dist/wasm"))
    args = parser.parse_args()
    if args.wheel:
        wheel = args.wheel.read_bytes()
    else:
        token = os.environ["GH_TOKEN"]
        url = ("https://api.github.com/repos/buchochelliq-labs/intentumdiff-python/"
               f"actions/artifacts/{IDENTITY['artifact_id']}/zip")
        request = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}",
                                                      "Accept": "application/vnd.github+json"})
        with urllib.request.build_opener(SafeRedirect()).open(request, timeout=120) as response:
            data = checked(response.read(), IDENTITY["archive_sha256"], "CI artifact")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            names = [name for name in archive.namelist() if name.endswith(".whl")]
            if len(names) != 1:
                raise ValueError("expected exactly one reviewed wheel")
            wheel = archive.read(names[0])
    provision(wheel, args.output)


if __name__ == "__main__":
    main()
