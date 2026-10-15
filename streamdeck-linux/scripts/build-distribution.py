#!/usr/bin/env python3
"""Build a native Deckforge distribution with its unauthenticated Discord plugin."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
import tarfile
import tempfile

PROJECT = Path(__file__).resolve().parents[1]


def architecture(binary):
    with binary.open('rb') as file:
        header = file.read(20)
    if len(header) != 20 or header[:5] != b'\x7fELF\x02' or header[5] != 1:
        raise ValueError('Expected a 64-bit little-endian Linux ELF executable')
    machine = struct.unpack_from('<H', header, 18)[0]
    if machine not in (62, 183):
        raise ValueError('Unsupported Linux architecture')
    return {62: 'x86_64', 183: 'aarch64'}[machine]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true', help='Use the existing host and Discord release binaries')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--with-obs-bridge', action='store_true', help='Include the GPL native OBS bridge and corresponding source, built against local OBS/Qt6')
    args = parser.parse_args()
    if not args.skip_build:
        subprocess.run(['npm', '--prefix', str(PROJECT / 'frontend'), 'ci'], check=True)
        subprocess.run(['npm', '--prefix', str(PROJECT / 'frontend'), 'run', 'build'], check=True)
        subprocess.run(['cargo', 'build', '--release', '--locked', '--bin', 'deckforge'], cwd=PROJECT, check=True)
    build = ['python3', str(PROJECT / 'scripts/build-discord-plugin.py')]
    if args.skip_build:
        build.append('--skip-build')
    subprocess.run(build, check=True)
    if args.with_obs_bridge and not args.skip_build:
        subprocess.run(['python3', str(PROJECT / 'scripts/build-obs-bridge.py')], check=True)
    binary = PROJECT / 'target/release/deckforge'
    plugin = PROJECT / 'plugins/discord/target/release/deckforge-discord'
    arch = architecture(binary)
    if architecture(plugin) != arch:
        raise ValueError('Host and Discord plugin architectures differ')
    package = PROJECT / 'target/release/discord-for-deckforge.streamDeckPlugin'
    output = (args.output or PROJECT / f'target/release/deckforge-linux-{arch}.tar.gz').resolve()
    entries = {
        'deckforge': (binary.read_bytes(), 0o755),
        'bundled-plugins/discord-for-deckforge.streamDeckPlugin': (package.read_bytes(), 0o644),
        'scripts/install-desktop.py': ((PROJECT / 'scripts/install-desktop.py').read_bytes(), 0o755),
        'assets/deckforge.svg': ((PROJECT / 'assets/deckforge.svg').read_bytes(), 0o644),
        'assets/deckforge-mime.xml': ((PROJECT / 'assets/deckforge-mime.xml').read_bytes(), 0o644),
        'assets/deckforge.png': ((PROJECT / 'assets/deckforge.png').read_bytes(), 0o644),
        'LICENSE': ((PROJECT.parent / 'LICENSE').read_bytes(), 0o644),
        'README.md': (b'''# Deckforge for Linux

Made with AI because Elgato was not making a Linux port, and its owner did not
want an expensive paperweight on the table.

Run ./deckforge, or install for your user with:

    python3 scripts/install-desktop.py

GTK3, WebKitGTK 4.1, PulseAudio/PipeWire and the other native dependencies listed
at https://github.com/JackSkellet/deckforge must be installed on the system.
This preview targets the Linux architecture named in its filename and uses the
native libraries of its build system. Build from source on distributions with
incompatible native libraries. Physical device validation currently covers Plus.

Plugins > Included with Deckforge installs the native Discord plugin. Existing
plugin installations are not replaced. Set up your own Discord application and
connect in its Plugin settings. No account settings, client secrets, connection
tokens, browser sessions, or user profiles are included.

Configuration and authorization remain in the user's private data directory.
Source and compatibility limits: https://github.com/JackSkellet/deckforge
''', 0o644),
    }
    if args.with_obs_bridge:
        bridge = PROJECT / 'target/release/obs-bridge'
        if not (bridge / 'source.tar.gz').is_file():
            raise SystemExit('Build the OBS bridge first; complete corresponding source is required.')
        for item in bridge.rglob('*'):
            if item.is_file():
                entries['obs-bridge/' + item.relative_to(bridge).as_posix()] = (item.read_bytes(), 0o644)
        entries['docs/OBS_BRIDGE.md'] = ((PROJECT / 'docs/OBS_BRIDGE.md').read_bytes(), 0o644)
    manifest = {name: hashlib.sha256(data).hexdigest() for name, (data, _) in entries.items()}
    entries['SHA256SUMS.json'] = (json.dumps(manifest, indent=2).encode() + b'\n', 0o644)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=output.parent, suffix='.tar.gz', delete=False) as pending:
        temporary = Path(pending.name)
    try:
        with tarfile.open(temporary, 'w:gz') as archive:
            for name, (data, mode) in entries.items():
                entry = tarfile.TarInfo(f'deckforge-linux-{arch}/{name}')
                entry.size, entry.mode, entry.mtime = len(data), mode, 0
                archive.addfile(entry, io.BytesIO(data))
        temporary.replace(output)
    finally:
        temporary.unlink(missing_ok=True)
    print(output)


if __name__ == '__main__':
    main()
