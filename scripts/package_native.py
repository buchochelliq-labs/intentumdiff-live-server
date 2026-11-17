"""Package the tested native executable and verified components with checksums.

One archive per platform (``intentumdiff-live-server-<platform>.tar.gz``, or ``.zip`` on
Windows) containing the executable, ``wasm/`` beside it, ``provenance.json`` and an inner
``SHA256SUMS``; the output directory gets the archive, an outer ``SHA256SUMS`` and a copy of
``provenance.json``. ``--verify`` re-checks an output directory's checksums (portable
replacement for ``sha256sum --check``).
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform as host_platform
import shutil
import subprocess
import sys
import tarfile
import tomllib
import zipfile

# Platform names follow the VS Code target vocabulary used by the extension (vscode#21).
PLATFORMS = {
    "linux-x64": "x86_64-unknown-linux-gnu",
    "linux-arm64": "aarch64-unknown-linux-gnu",
    "darwin-x64": "x86_64-apple-darwin",
    "darwin-arm64": "aarch64-apple-darwin",
    "win32-x64": "x86_64-pc-windows-msvc",
    "win32-arm64": "aarch64-pc-windows-msvc",
}


def default_platform():
    machine = host_platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "x64"
    system = {"linux": "linux", "darwin": "darwin", "win32": "win32"}.get(sys.platform, "linux")
    return f"{system}-{arch}"


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_sums(root, paths, destination):
    destination.write_text("".join(
        f"{sha256(path)}  {path.relative_to(root).as_posix()}\n" for path in sorted(paths)
    ))


def verify(directory):
    """Check every entry of ``directory/SHA256SUMS``; raise on any mismatch."""
    sums = (directory / "SHA256SUMS").read_text().splitlines()
    if not sums:
        raise ValueError("empty SHA256SUMS")
    for line in sums:
        expected, name = line.split("  ", 1)
        if sha256(directory / name) != expected:
            raise ValueError(f"checksum mismatch: {name}")
    print(f"verified {len(sums)} checksum(s) in {directory}")


def package(binary, root, output, platform, server_commit, rustc):
    manifest = tomllib.loads(Path("Cargo.toml").read_text())
    core = manifest["dependencies"]["intentumdiff-rust-core"]["rev"]
    if len(core) != 40:
        raise ValueError("core must be pinned to a full commit")
    if platform not in PLATFORMS:
        raise ValueError(f"unknown platform {platform!r}; expected one of {sorted(PLATFORMS)}")
    windows = platform.startswith("win32")
    exe_name = "intentumdiff-live-server.exe" if windows else "intentumdiff-live-server"
    components = json.loads((root / "wasm/component-source.json").read_text())
    shutil.copy2(binary, root / exe_name)
    provenance = {
        "server_commit": server_commit,
        # Cargo SemVer prerelease (0.0.3-beta.1) == product PEP 440 version (0.0.3b1).
        "server_version": manifest["package"]["version"],
        "platform": platform,
        "rust_target": PLATFORMS[platform],
        "core_commit": core,
        "rustc": rustc,
        "components": components,
        "protocol_version": 2,
        "transport": "stdio",
    }
    (root / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    paths = [path for path in root.rglob("*") if path.is_file() and path.name != "SHA256SUMS"]
    write_sums(root, paths, root / "SHA256SUMS")
    output.mkdir(parents=True, exist_ok=True)
    stem = f"intentumdiff-live-server-{platform}"
    if windows:
        archive_path = output / f"{stem}.zip"
        with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(root.rglob("*")):
                if path.is_file():
                    archive.write(path, f"intentumdiff-live-server/{path.relative_to(root).as_posix()}")
    else:
        archive_path = output / f"{stem}.tar.gz"
        with tarfile.open(archive_path, "w:gz") as archive:
            archive.add(root, arcname="intentumdiff-live-server")  # keeps the exec bit
    write_sums(output, [archive_path], output / "SHA256SUMS")
    shutil.copy2(root / "provenance.json", output / "provenance.json")
    return archive_path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", default=default_platform(), choices=sorted(PLATFORMS))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--root", type=Path, default=Path("dist"))
    parser.add_argument("--output", type=Path, default=Path("artifacts"))
    parser.add_argument("--verify", type=Path, metavar="DIR",
                        help="only verify DIR/SHA256SUMS and exit")
    args = parser.parse_args()
    if args.verify:
        verify(args.verify)
        return
    binary = args.binary or Path("target/release") / (
        "intentumdiff-live-server.exe" if args.platform.startswith("win32")
        else "intentumdiff-live-server")
    package(
        binary, args.root, args.output, args.platform,
        subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        subprocess.check_output(["rustc", "--version"], text=True).strip(),
    )
    verify(args.output)


if __name__ == "__main__":
    main()
