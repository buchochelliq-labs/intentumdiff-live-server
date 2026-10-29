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
    "python_commit": "ce67e79d444c0d4d70f57ec96b04b312c0154272",
    "component_core_commit": "e766f9b8ae842cb2168cd9142f8a3019a8961255",
    "wheel_sha256": "46ee9a9e326a76d27ede9891d7fe4501c6e2f25ac0728c0fd4cd3c7c4350e9d9",
    "artifact_id": 12912183858,
    "archive_sha256": "b8c0b724966393c07488196c7887c4a34849862f4cc12ba276b93b7b1fdecefe",
    "workflow_run": 40900875287,
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
        names = [
            name for name in archive.namelist()
            if name.startswith("intentumdiff/wasm/") and not name.endswith("/")
        ]
        # A dict comprehension would silently discard duplicate archive members.
        if len(names) != len(set(names)):
            raise ValueError("duplicate Wasm archive member")
        members = {
            name.removeprefix("intentumdiff/wasm/"): archive.read(name)
            for name in names
        }
    if any("/" in name or "\\" in name or name in (".", "..") for name in members):
        raise ValueError("unexpected nested component path")
    manifest = json.loads(members["wasm_provenance.json"])
    artifacts = manifest["artifacts"]
    actual = {name for name in members if name.endswith(".wasm")}
    if (not isinstance(artifacts, dict) or not artifacts
            or manifest.get("artifact_count") != len(artifacts)):
        raise ValueError("invalid component provenance artifact count")
    if actual != set(artifacts):
        raise ValueError("component manifest does not match archive")
    for name, item in artifacts.items():
        checked(members[name], item["sha256"], name)
        if len(members[name]) != item["size_bytes"]:
            raise ValueError(f"{name} size mismatch")
    # The checksums above establish component-byte integrity, but not that
    # the parser manifest can resolve its declared languages. Every parser
    # reference and extension-index target must exist in the SAME pinned wheel.
    parser_manifest = json.loads(members["parser_manifest.json"])
    parsers = parser_manifest.get("parsers")
    extension_index = parser_manifest.get("extension_index")
    if (parser_manifest.get("schema_version") != 1
            or not isinstance(parsers, dict) or not parsers
            or not isinstance(extension_index, dict)):
        raise ValueError("invalid parser manifest structure")
    for language, entry in parsers.items():
        if (not isinstance(entry, dict)
                or not isinstance(entry.get("wasm"), str)
                or entry["wasm"] not in actual):
            raise ValueError(f"parser {language!r} references missing Wasm component")
    for extension, language in extension_index.items():
        if (not isinstance(extension, str) or not isinstance(language, str)
                or language not in parsers):
            raise ValueError(
                f"extension {extension!r} references unknown parser {language!r}"
            )
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
