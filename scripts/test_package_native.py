import json
import os
from pathlib import Path
import stat
import sys
import tarfile
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
os.chdir(Path(__file__).resolve().parent.parent)  # package() reads ./Cargo.toml

from package_native import PLATFORMS, package, verify


class PackageNativeTests(unittest.TestCase):
    def stage(self, tmp):
        root = tmp / "dist"
        (root / "wasm").mkdir(parents=True)
        (root / "wasm" / "component-source.json").write_text('{"component_core_commit": "x"}')
        (root / "wasm" / "example_parser.wasm").write_bytes(b"\0asm")
        binary = tmp / "bin"
        binary.write_bytes(b"#!/bin/sh\n")
        binary.chmod(0o755)
        return root, binary

    def run_package(self, platform):
        tmp = Path(tempfile.mkdtemp())
        root, binary = self.stage(tmp)
        out = tmp / "artifacts"
        archive = package(binary, root, out, platform, "a" * 40, "rustc 1.95.0")
        return out, archive

    def test_every_platform_packages_with_verifiable_checksums_and_provenance(self):
        self.assertEqual(len(PLATFORMS), 6)
        for platform in PLATFORMS:
            out, archive = self.run_package(platform)
            verify(out)
            provenance = json.loads((out / "provenance.json").read_text())
            self.assertEqual(provenance["platform"], platform)
            self.assertEqual(provenance["rust_target"], PLATFORMS[platform])
            self.assertRegex(provenance["server_version"], r"^\d+\.\d+\.\d+")
            self.assertEqual(len(provenance["core_commit"]), 40)
            self.assertIn(platform, archive.name)

    def test_unix_archive_keeps_exec_bit_and_windows_archive_is_zip_with_exe(self):
        _, archive = self.run_package("linux-arm64")
        with tarfile.open(archive) as tar:
            member = tar.getmember("intentumdiff-live-server/intentumdiff-live-server")
            self.assertTrue(member.mode & stat.S_IXUSR)
            self.assertIn("intentumdiff-live-server/wasm/example_parser.wasm", tar.getnames())
            self.assertIn("intentumdiff-live-server/SHA256SUMS", tar.getnames())
        _, archive = self.run_package("win32-x64")
        self.assertTrue(archive.name.endswith(".zip"))
        with zipfile.ZipFile(archive) as zf:
            self.assertIn("intentumdiff-live-server/intentumdiff-live-server.exe", zf.namelist())

    def test_tampered_archive_fails_verification(self):
        out, archive = self.run_package("darwin-arm64")
        archive.write_bytes(archive.read_bytes() + b"x")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            verify(out)

    def test_unknown_platform_is_rejected(self):
        tmp = Path(tempfile.mkdtemp())
        root, binary = self.stage(tmp)
        with self.assertRaisesRegex(ValueError, "unknown platform"):
            package(binary, root, tmp / "o", "solaris-sparc", "a" * 40, "rustc")


if __name__ == "__main__":
    unittest.main()
