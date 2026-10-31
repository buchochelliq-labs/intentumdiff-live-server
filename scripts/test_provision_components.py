"""Native build-time parser/Wasm provenance tests; no Python runtime dependency."""

import hashlib
import io
import json
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path
from unittest.mock import patch

from provision_components import IDENTITY, EXPECTED_WHEEL_PATH, extract_verified_ci_wheel, provision, require_matching_native_core


class ProvisionComponentTests(unittest.TestCase):
    @staticmethod
    def wheel(*, missing_wasm=False, unknown_extension=False,
              bad_count=False, duplicate=False):
        component = b"synthetic-wasm-content"
        provenance = {
            "schema_version": 1,
            "artifact_count": 2 if bad_count else 1,
            "artifacts": {
                "example_parser.wasm": {
                    "sha256": hashlib.sha256(component).hexdigest(),
                    "size_bytes": len(component),
                },
            },
        }
        parser_manifest = {
            "schema_version": 1,
            "parsers": {
                "example": {
                    "plugin_id": "example",
                    "extensions": [".example"],
                    "wasm": "missing_parser.wasm" if missing_wasm
                            else "example_parser.wasm",
                },
            },
            "extension_index": {".example": "missing" if unknown_extension else "example"},
        }
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            prefix = "intentumdiff/wasm/"
            archive.writestr(prefix + "example_parser.wasm", component)
            archive.writestr(
                prefix + "wasm_provenance.json", json.dumps(provenance)
            )
            archive.writestr(
                prefix + "parser_manifest.json", json.dumps(parser_manifest)
            )
            if duplicate:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    archive.writestr(prefix + "example_parser.wasm", component)
        return output.getvalue()

    def stage(self, wheel):
        with tempfile.TemporaryDirectory() as temp:
            with patch.dict(
                IDENTITY, {"wheel_sha256": hashlib.sha256(wheel).hexdigest()}
            ):
                target = Path(temp) / "wasm"
                provision(wheel, target)
                return {p.name for p in target.iterdir()}

    def test_native_core_must_match_the_verified_python_wheel_core(self):
        require_matching_native_core(IDENTITY["component_core_commit"])
        with self.assertRaisesRegex(ValueError, "Native Rust Core revision differs"):
            require_matching_native_core("0" * 40)
        with self.assertRaisesRegex(ValueError, "Native Rust Core revision differs"):
            require_matching_native_core("")

    def test_valid_pinned_wheel_inventory_stages(self):
        self.assertEqual(
            self.stage(self.wheel()),
            {
                "example_parser.wasm", "wasm_provenance.json",
                "parser_manifest.json", "component-source.json",
            },
        )

    def test_parser_reference_to_absent_component_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "missing Wasm component"):
            self.stage(self.wheel(missing_wasm=True))

    def test_extension_index_target_must_exist(self):
        with self.assertRaisesRegex(ValueError, "unknown parser"):
            self.stage(self.wheel(unknown_extension=True))

    def test_provenance_artifact_count_must_match(self):
        with self.assertRaisesRegex(ValueError, "artifact count"):
            self.stage(self.wheel(bad_count=True))

    def test_duplicate_archive_member_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "duplicate Wasm archive member"):
            self.stage(self.wheel(duplicate=True))


    @staticmethod
    def ci_archive(*, python_commit=None, core_commit=None, profile="release",
                   wheel_path=EXPECTED_WHEEL_PATH, missing=(), duplicate=False):
        """Synthetic archive proving the three source/profile attestations are mandatory."""
        wheel = b"synthetic-wheel"
        evidence = {
            "evidence/python-tested-commit.txt": (
                IDENTITY["python_commit"] if python_commit is None else python_commit
            ),
            "evidence/core-commit.txt": (
                IDENTITY["component_core_commit"] if core_commit is None else core_commit
            ),
            "evidence/wheel-build-profile.txt": profile,
        }
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            archive.writestr(wheel_path, wheel)
            for path, value in evidence.items():
                if path not in missing:
                    archive.writestr(path, value + "\n")
            if duplicate:
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    archive.writestr("evidence/python-tested-commit.txt", evidence[
                        "evidence/python-tested-commit.txt"
                    ])
        return output.getvalue()

    def checked_archive(self, archive):
        with patch.dict(IDENTITY, {
            "archive_sha256": hashlib.sha256(archive).hexdigest(),
            "wheel_sha256": hashlib.sha256(b"synthetic-wheel").hexdigest(),
        }):
            return extract_verified_ci_wheel(archive)

    def test_ci_archive_requires_actual_tested_merge_commit_and_release_profile(self):
        self.assertEqual(self.checked_archive(self.ci_archive()), b"synthetic-wheel")
        invalid = [
            (self.ci_archive(python_commit="53925bea94ad8663ad350c1c8c96a306af57d5fe"), "python-tested-commit"),
            (self.ci_archive(core_commit="wrong-core"), "core-commit"),
            (self.ci_archive(profile="debug"), "wheel-build-profile"),
            (self.ci_archive(wheel_path="dist/other.whl"), "pinned Linux wheel"),
            (self.ci_archive(missing=("evidence/python-tested-commit.txt",)), "lacks valid attestation"),
            (self.ci_archive(duplicate=True), "duplicate CI artifact"),
        ]
        for archive, diagnostic in invalid:
            with self.subTest(diagnostic=diagnostic):
                with self.assertRaisesRegex(ValueError, diagnostic):
                    self.checked_archive(archive)

    def test_ci_archive_rejects_checksum_mismatches_before_any_attestation(self):
        archive = self.ci_archive()
        with self.assertRaisesRegex(ValueError, "CI artifact checksum mismatch"):
            extract_verified_ci_wheel(archive)
        with patch.dict(IDENTITY, {"archive_sha256": hashlib.sha256(archive).hexdigest()}):
            with self.assertRaisesRegex(ValueError, "reviewed wheel checksum mismatch"):
                extract_verified_ci_wheel(archive)


if __name__ == "__main__":
    unittest.main()
