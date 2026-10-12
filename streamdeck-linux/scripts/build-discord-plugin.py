#!/usr/bin/env python3
"""Build and package Deckforge's independent native Rust Discord plugin."""
import argparse
import json
from pathlib import Path
import stat
import subprocess
import tempfile
import zipfile


def main():
    project = Path(__file__).resolve().parents[1]
    source = project/'plugins/discord'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true', help='Package the existing release binary')
    parser.add_argument('--output', type=Path, default=project/'target/release/discord-for-deckforge.streamDeckPlugin')
    args = parser.parse_args()
    if not args.skip_build:
        subprocess.run(['cargo','build','--release','--locked','--manifest-path',str(source/'Cargo.toml')],check=True)
    binary = source/'target/release/deckforge-discord'
    if binary.read_bytes()[:4] != b'\x7fELF':
        raise ValueError('Build did not produce a native Linux ELF executable')
    manifest = json.loads((source/'manifest.json').read_text())
    identifier = manifest['UUID']
    if identifier != 'com.elgato.discord':
        raise ValueError('Unexpected compatibility identity')
    files = [(source/'manifest.json', 'manifest.json'), (binary, manifest['CodePath'])]
    for directory in ('images','ui'):
        for path in sorted((source/directory).rglob('*')):
            if path.is_symlink():
                raise ValueError('Package source contains a symlink')
            if path.is_file():
                files.append((path,str(path.relative_to(source))))
    args.output.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=args.output.parent,suffix='.zip',delete=False) as pending:
        temporary = Path(pending.name)
    try:
        with zipfile.ZipFile(temporary,'w',zipfile.ZIP_DEFLATED) as archive:
            for path, relative in files:
                info = zipfile.ZipInfo(f'{identifier}.sdPlugin/{relative}')
                info.create_system = 3
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = (stat.S_IFREG | (0o755 if path == binary else 0o644)) << 16
                archive.writestr(info,path.read_bytes())
        temporary.replace(args.output)
    finally:
        temporary.unlink(missing_ok=True)
    print(args.output)


if __name__ == '__main__':
    main()
