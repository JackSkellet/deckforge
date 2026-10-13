#!/usr/bin/env python3
"""Build and package Deckforge's independent native Rust Discord plugin."""
import argparse
import json
from pathlib import Path
import stat
import subprocess
import tempfile
import zipfile


def validate_inspector_schema(source):
    schema = json.loads((source / 'ui/fields.json').read_text())
    expected = ('// Generated from fields.json by tests/generate-fields.cjs.\n'
                'window.DeckforgeDiscordFields = '
                + json.dumps(schema, indent=2, ensure_ascii=False) + ';\n')
    generated = source / 'ui/fields.js'
    if not generated.is_file() or generated.read_text() != expected:
        raise ValueError('Inspector schema is stale. Run node plugins/discord/tests/generate-fields.cjs before packaging.')


def package_files(source, binary):
    """Only public runtime assets are eligible; never walk state or private data."""
    validate_inspector_schema(source)
    manifest = json.loads((source / 'manifest.json').read_text())
    if manifest.get('UUID') != 'com.elgato.discord' or manifest.get('CodePath') != 'bin/deckforge-discord':
        raise ValueError('Unexpected Discord plugin identity or executable')
    names = {'manifest.json', 'ui/index.html', 'ui/app.js', 'ui/fields.json', 'ui/fields.js'}
    def images(value):
        if isinstance(value, dict):
            for key, entry in value.items():
                if key in ('Icon', 'Image', 'CategoryIcon') and isinstance(entry, str) and entry:
                    path = Path(entry)
                    if path.is_absolute() or '..' in path.parts or path.parts[0] != 'images':
                        raise ValueError('Image must be a public image asset')
                    names.add(str(path if path.suffix == '.svg' else Path(str(path) + '.svg')))
                else:
                    images(entry)
        elif isinstance(value, list):
            for entry in value:
                images(entry)
    images(manifest)
    files = [(source / name, name) for name in sorted(names)] + [(binary, manifest['CodePath'])]
    for path, _ in files:
        if not path.is_file() or path.is_symlink():
            raise ValueError('Package asset must be a regular file')
        relative = path.relative_to(source)
        if any((source.joinpath(*relative.parts[:n])).is_symlink() for n in range(1, len(relative.parts))):
            raise ValueError('Package asset contains a symlink directory')
    return manifest['UUID'], files


def main():
    project = Path(__file__).resolve().parents[1]
    source = project/'plugins/discord'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true', help='Package the existing release binary')
    parser.add_argument('--output', type=Path, default=project/'target/release/discord-for-deckforge.streamDeckPlugin')
    args = parser.parse_args()
    validate_inspector_schema(source)
    if not args.skip_build:
        subprocess.run(['cargo','build','--release','--locked','--manifest-path',str(source/'Cargo.toml')],check=True)
    binary = source/'target/release/deckforge-discord'
    if binary.read_bytes()[:4] != b'\x7fELF':
        raise ValueError('Build did not produce a native Linux ELF executable')
    identifier, files = package_files(source, binary)
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
