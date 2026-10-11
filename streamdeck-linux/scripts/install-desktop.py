#!/usr/bin/env python3
"""Install a built native executable and desktop launcher for the current user.

Application state is never changed. A replaced executable is kept as .previous.
No root access, shell command interpolation, or startup service is required.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def atomic_copy(source, target, mode):
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as output:
        temporary = Path(output.name)
        with source.open("rb") as source_file:
            shutil.copyfileobj(source_file, output)
    try:
        temporary.chmod(mode)
        if target.exists():
            shutil.copy2(target, target.with_name(target.name + ".previous"))
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)


def desktop_quote(path):
    # Desktop Entry exec quoting is not shell quoting. Escape its two layers.
    value = str(path)
    if any(c in value for c in "\n\r="):
        raise ValueError("Unsupported character in executable path")
    value = value.replace("%", "%%")
    value = "".join("\\" + c if c in '\\"`$' else c for c in value)
    return '"' + value.replace("\\", "\\\\") + '"'


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=root / "target/release/deckforge")
    args = parser.parse_args()
    source = args.binary.resolve(strict=True)
    subprocess.run([str(source), "--version"], check=True)
    home = Path.home()
    data = Path(os.environ.get("XDG_DATA_HOME", home / ".local/share"))
    executable = home / ".local/bin/deckforge"
    icon = data / "icons/hicolor/scalable/apps/deckforge.svg"
    entry = data / "applications/deckforge.desktop"
    atomic_copy(source, executable, 0o755)
    atomic_copy(root / "assets/deckforge.svg", icon, 0o644)
    entry.parent.mkdir(parents=True, exist_ok=True)
    content = (
        "[Desktop Entry]\nType=Application\nName=Deckforge\n"
        "Comment=Configure Stream Deck devices and compatible plugins\n"
        f"Exec={desktop_quote(executable)}\nIcon=deckforge\n"
        "Terminal=false\nCategories=Utility;\nStartupWMClass=deckforge\n"
    )
    if entry.exists():
        shutil.copy2(entry, entry.with_suffix(".desktop.previous"))
    temporary = entry.with_suffix(".desktop.tmp")
    temporary.write_text(content)
    temporary.replace(entry)
    if shutil.which("desktop-file-validate"):
        subprocess.run(["desktop-file-validate", str(entry)], check=True)
    if shutil.which("update-desktop-database"):
        subprocess.run(["update-desktop-database", str(entry.parent)], check=True)
    print(f"Installed native application: {executable}\nDesktop launcher: {entry}")


if __name__ == "__main__":
    main()
