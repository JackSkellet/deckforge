#!/usr/bin/env python3
"""The package guard rejects absent/stale generated schemas without requiring Node."""
import importlib.util
from pathlib import Path
import tempfile
import shutil
import unittest

PROJECT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("package_discord", PROJECT / "scripts/build-discord-plugin.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class SchemaPackageTests(unittest.TestCase):
    def test_only_runtime_assets_are_packaged_and_private_decoys_are_excluded(self):
        with tempfile.TemporaryDirectory(prefix="deckforge-discord-package-") as temporary:
            source = Path(temporary)
            original = PROJECT / "plugins/discord"
            shutil.copy2(original / "manifest.json", source / "manifest.json")
            for directory in ("ui", "images"):
                shutil.copytree(original / directory, source / directory)
            binary = source / "target/release/deckforge-discord"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"test-only ELF fixture")
            (source / "discord-auth.json").write_text('{"access_token":"private-decoy"}')
            (source / "ui/discord-auth.json").write_text('{"client_secret":"private-decoy"}')
            (source / "images/unreferenced.svg").write_text("private-decoy")
            _, files = PACKAGE.package_files(source, binary)
            names = {relative for _, relative in files}
            self.assertIn("images/channel.voice.svg", names)
            self.assertIn("bin/deckforge-discord", names)
            self.assertNotIn("ui/discord-auth.json", names)
            self.assertNotIn("images/unreferenced.svg", names)
            self.assertFalse(any(b"private-decoy" in path.read_bytes() for path, _ in files))
            (source / "ui/app.js").unlink()
            (source / "ui/app.js").symlink_to(source / "discord-auth.json")
            with self.assertRaisesRegex(ValueError, "regular file"):
                PACKAGE.package_files(source, binary)

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
