#!/usr/bin/env python3
"""Install the Linux Bluetooth plugin, retaining the previous package for rollback."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
from datetime import datetime, timezone


UUID = "com.flowkits.bluetooth"
STOCK_JS_SHA256 = "cc9175cebf542ef37577c3b8cf52cc4aebeb3231789c89c40248924db38bf9b1"
STOCK_MANIFEST_SHA256 = "6ded7088733af0e44861afe8dd25cf30f59c2a4923f493d621cd02cd6f78908f"
MARKER = ".deckforge-bluetooth-linux.json"
SOURCE = Path(__file__).resolve().parents[1] / "plugins" / "bluetooth-compat"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def files(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Refusing symlink: {path}")
        relative = path.relative_to(root)
        if "__pycache__" in relative.parts or path.name == MARKER:
            continue
        if path.is_file():
            result[str(relative)] = digest(path)
    return result


def validate_source(source):
    manifest = json.loads((source / "manifest.json").read_text())
    if manifest.get("UUID") != UUID or manifest.get("CodePath") != "plugin.mjs":
        raise ValueError("Source manifest must identify the Linux Bluetooth plugin")
    for name in ("plugin.mjs", "fk-bt-helper.py"):
        if not (source / name).is_file():
            raise ValueError(f"Missing source: {name}")
    return files(source)


def install(source, target):
    source, target = source.resolve(), target.absolute()
    source_hashes = validate_source(source)
    if target.is_symlink():
        raise ValueError(f"Refusing symlink target: {target}")
    if target.exists():
        marker = target / MARKER
        if marker.is_file():
            manifest = json.loads((target / "manifest.json").read_text())
            if manifest.get("UUID") != UUID:
                raise ValueError("Target is a different plugin")
            saved = json.loads(marker.read_text())
            if saved.get("files") != files(target):
                raise ValueError("Installed plugin has changed since installation; refusing to overwrite it")
            if source_hashes == saved["files"]:
                return {"status": "already installed", "target": str(target), "backup": saved.get("backup")}
        else:
            stock = target / "bin" / "plugin.js"
            manifest = target / "manifest.json"
            # The original manifest is an ELGATO binary envelope, not UTF-8 JSON.
            if (not stock.is_file() or digest(stock) != STOCK_JS_SHA256
                    or not manifest.is_file() or digest(manifest) != STOCK_MANIFEST_SHA256):
                raise ValueError("Unrecognized existing package; refusing to overwrite it")
            files(target)  # Reject links before moving a package into the backup directory.
    target.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".bluetooth-linux-stage-", dir=target.parent))
    backup = None
    try:
        for relative in source_hashes:
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / relative, destination)
        (stage / "fk-bt-helper.py").chmod(0o755)
        if target.exists():
            backup_dir = target.parent.parent / "plugin-backups"
            backup_dir.mkdir(parents=True, exist_ok=True)
            stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
            backup = backup_dir / f"{target.name}.{stamp}"
        (stage / MARKER).write_text(json.dumps({
            "format": 1, "files": source_hashes,
            "backup": str(backup) if backup else None,
        }, indent=2) + "\n")
        if backup:
            os.rename(target, backup)
        try:
            os.rename(stage, target)
        except OSError:
            if backup:
                os.rename(backup, target)
            raise
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    return {"status": "installed", "target": str(target), "backup": str(backup) if backup else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", type=Path, default=Path.home() / ".local/share/deckforge/plugins/com.flowkits.bluetooth.sdPlugin")
    parser.add_argument("--source", type=Path, default=SOURCE)
    args = parser.parse_args()
    try:
        print(json.dumps(install(args.source, args.target), indent=2))
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Bluetooth installation stopped: {error}\n")


if __name__ == "__main__":
    main()
