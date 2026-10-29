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

from provision_components import IDENTITY, provision


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


if __name__ == "__main__":
    unittest.main()
