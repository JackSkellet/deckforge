#!/usr/bin/env python3
"""Installer URI registration checks, with all process operations intercepted."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("install_desktop", ROOT / "scripts/install-desktop.py")
INSTALLER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLER)


class InstallerUriTests(unittest.TestCase):
    def install(self, existing, query_code=0):
        with tempfile.TemporaryDirectory(prefix="deckforge-install-uri-") as temporary:
            root = Path(temporary)
            source = root / "source"
            source.write_bytes(b"isolated binary fixture")
            calls = []

            def run(arguments, **_):
                calls.append(arguments)
                if arguments[:3] == ["xdg-mime", "query", "default"]:
                    return subprocess.CompletedProcess(arguments, query_code, existing + "\n", "")
                return subprocess.CompletedProcess(arguments, 0, "", "")

            with patch.dict(os.environ, {"HOME": str(root), "XDG_DATA_HOME": str(root / "data")}), patch("sys.argv", ["install-desktop.py", "--binary", str(source)]), patch.object(INSTALLER.shutil, "which", side_effect=lambda name: "/fake/xdg-mime" if name == "xdg-mime" else None), patch.object(INSTALLER.subprocess, "run", side_effect=run):
                INSTALLER.main()
            entry = (root / "data/applications/deckforge.desktop").read_text()
            self.assertIn("MimeType=x-scheme-handler/streamdeck;", entry)
            self.assertIn(f'Exec="{root}/.local/bin/deckforge" %u\n', entry)
            self.assertEqual((root / ".local/bin/deckforge").read_bytes(), source.read_bytes())
            self.assertFalse((root / "data/deckforge").exists())
            return calls

    def test_registers_when_no_existing_default(self):
        calls = self.install("")
        self.assertIn(["xdg-mime", "default", "deckforge.desktop", "x-scheme-handler/streamdeck"], calls)

    def test_refreshes_its_own_default(self):
        calls = self.install("deckforge.desktop")
        self.assertIn(["xdg-mime", "default", "deckforge.desktop", "x-scheme-handler/streamdeck"], calls)

    def test_preserves_other_default(self):
        calls = self.install("other-streamdeck.desktop")
        self.assertFalse(any(call[:2] == ["xdg-mime", "default"] for call in calls))

    def test_does_not_guess_default_when_query_fails(self):
        calls = self.install("", query_code=1)
        self.assertFalse(any(call[:2] == ["xdg-mime", "default"] for call in calls))


if __name__ == "__main__":
    unittest.main()
