"""Package the tested native executable and verified components with checksums."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/intentumdiff-live-server"))
    parser.add_argument("--root", type=Path, default=Path("dist"))
    parser.add_argument("--output", type=Path, default=Path("artifacts"))
    args = parser.parse_args()
    core = tomllib.loads(Path("Cargo.toml").read_text())["dependencies"]["intentumdiff-rust-core"]["rev"]
    if len(core) != 40:
        raise ValueError("core must be pinned to a full commit")
    components = json.loads((args.root / "wasm/component-source.json").read_text())
    shutil.copy2(args.binary, args.root / "intentumdiff-live-server")
    provenance = {
        "server_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "core_commit": core,
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "components": components,
        "protocol_version": 2,
        "transport": "stdio",
    }
    (args.root / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    checksum = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    paths = sorted(path for path in args.root.rglob("*") if path.is_file() and path.name != "SHA256SUMS")
    (args.root / "SHA256SUMS").write_text("".join(f"{checksum(path)}  {path.relative_to(args.root).as_posix()}\n" for path in paths))
    args.output.mkdir(parents=True, exist_ok=True)
    tar = args.output / "intentumdiff-live-server-linux-x64.tar.gz"
    with tarfile.open(tar, "w:gz") as archive:
        archive.add(args.root, arcname="intentumdiff-live-server")
    (args.output / "SHA256SUMS").write_text(f"{checksum(tar)}  {tar.name}\n")
    shutil.copy2(args.root / "provenance.json", args.output / "provenance.json")


if __name__ == "__main__":
    main()
