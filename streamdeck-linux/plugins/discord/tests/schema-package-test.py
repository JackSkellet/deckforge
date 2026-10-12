#!/usr/bin/env python3
"""The package guard rejects absent/stale generated schemas without requiring Node."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

PROJECT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("package_discord", PROJECT / "scripts/build-discord-plugin.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class SchemaPackageTests(unittest.TestCase):
    def test_exact_schema_and_stale_or_missing_generated_file(self):
        with tempfile.TemporaryDirectory(prefix="deckforge-discord-schema-") as temporary:
            source = Path(temporary)
            ui = source / "ui"
            ui.mkdir()
            original = PROJECT / "plugins/discord/ui"
            (ui / "fields.json").write_bytes((original / "fields.json").read_bytes())
            with self.assertRaisesRegex(ValueError, "schema is stale"):
                PACKAGE.validate_inspector_schema(source)
            (ui / "fields.js").write_bytes((original / "fields.js").read_bytes())
            PACKAGE.validate_inspector_schema(source)
            with (ui / "fields.js").open("a") as file:
                file.write("// stale change\n")
            with self.assertRaisesRegex(ValueError, "schema is stale"):
                PACKAGE.validate_inspector_schema(source)


if __name__ == "__main__":
    unittest.main()
