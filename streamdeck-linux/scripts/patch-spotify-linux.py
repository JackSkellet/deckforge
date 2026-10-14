#!/usr/bin/env python3
"""Add a Linux MPRIS controller to the installed Spotify 2.5 bundle, with backup.

No vendor files are redistributed. Refuse unfamiliar bundles rather than guessing.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import re
import subprocess
import tempfile
from pathlib import Path
import shutil

DERIVATIVE_NAME = 'Spotify for Linux'
DERIVATIVE_DESCRIPTION = 'Local Linux MPRIS adaptation of the installed Elgato Spotify plugin.'

PREFIX = 'import {SpotifyLinuxController as DeckforgeSpotifyLinuxController} from "./spotify-linux.mjs";\n'
PATCHES = [
    ('const xe={win32:', 'const xe={linux:()=>i($e),win32:'),
    ('if("darwin"===process.platform){const e=await Promise.resolve().then(function(){return UP})',
     'if("linux"===process.platform){this.osImpl=new DeckforgeSpotifyLinuxController}else if("darwin"===process.platform){const e=await Promise.resolve().then(function(){return UP})'),
    ('isSupported(){return"darwin"===process.platform||"win32"===process.platform}',
     'isSupported(){return"linux"===process.platform||"darwin"===process.platform||"win32"===process.platform}'),
]


def update_localizations(plugin, *, backup_changes=False):
    changes = []
    for path in sorted(plugin.glob('*.json')):
        if not re.fullmatch(r'[a-z]{2}(?:_[A-Z]{2})?\.json', path.name):
            continue
        overlay = json.loads(path.read_text())
        if not isinstance(overlay, dict):
            raise SystemExit('Unexpected localization overlay: ' + str(path))
        if overlay.get('Name') == DERIVATIVE_NAME and overlay.get('Description') == DERIVATIVE_DESCRIPTION:
            continue
        overlay['Name'] = DERIVATIVE_NAME
        overlay['Description'] = DERIVATIVE_DESCRIPTION
        changes.append((path, json.dumps(overlay, ensure_ascii=False, indent=2) + '\n'))
    if changes and backup_changes:
        backup = plugin.parent.parent / 'plugin-backups' / ('spotify-localization-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ'))
        backup.mkdir(parents=True)
        for path, _ in changes:
            shutil.copy2(path, backup / path.name)
        print('Localization backup:', backup)
    for path, content in changes:
        temporary = path.with_suffix('.json.tmp')
        temporary.write_text(content)
        temporary.chmod(path.stat().st_mode & 0o777)
        temporary.replace(path)
    return len(changes)


def authenticated_manifest(plugin, reference, work):
    root = Path(__file__).resolve().parents[1]
    source = work / 'verify.rs'
    source.write_text((root / 'plugins/spotify-linux/verify-manifest.rs').read_text().replace(
        'PROTECTED_MANIFEST_SOURCE', str(root / 'src/protected_manifest.rs')))
    deps = root / 'target/release/deps'
    command = ['rustc', '--edition=2024', str(source), '-L', 'dependency=' + str(deps), '-o', str(work / 'verify')]
    for name in ['aes_gcm', 'anyhow', 'base64', 'blake3', 'ed25519_dalek', 'hkdf', 'minicbor', 'serde_json', 'sha2', 'x25519_dalek', 'zeroize', 'libc']:
        libraries = sorted(deps.glob('lib' + name + '-*.rlib'), key=lambda p: p.stat().st_mtime, reverse=True)
        if not libraries:
            raise SystemExit('Build Deckforge release dependencies first: missing ' + name)
        command.extend(['--extern', name + '=' + str(libraries[0])])
    subprocess.run(command, check=True, capture_output=True)
    output = work / 'authenticated.json'
    subprocess.run([str(work / 'verify'), str(plugin), str(reference), str(output)], check=True)
    return json.loads(output.read_text())


def patch(plugin, reference):
    entry = plugin / 'bin/plugin.js'
    source = entry.read_text()
    shim = Path(__file__).resolve().parents[1] / 'plugins/spotify-linux/spotify-linux.mjs'
    if source.startswith(PREFIX):
        if all(new in source for _, new in PATCHES) and (entry.parent / shim.name).read_bytes() == shim.read_bytes():
            changed = update_localizations(plugin, backup_changes=True)
            print(f'Spotify Linux support already installed; updated {changed} localization overlays.')
            return
        raise SystemExit('Existing Linux patch differs; restore its backup before updating.')
    for old, _ in PATCHES:
        if source.count(old) != 1:
            raise SystemExit('Unsupported Spotify bundle; no files changed: ' + old[:70])
    with tempfile.TemporaryDirectory(prefix='spotify-auth-') as temporary:
        manifest = authenticated_manifest(plugin, reference, Path(temporary))
    if manifest.get('UUID') != 'com.elgato.spotify' or manifest.get('CodePath') != 'bin/plugin.js':
        raise SystemExit('Unexpected Spotify identity or entrypoint; no files changed.')
    backup = plugin.parent.parent / 'plugin-backups' / ('spotify-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ'))
    backup.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix='.spotify-linux-', dir=plugin.parent))
    try:
        shutil.copytree(plugin, stage, dirs_exist_ok=True)
        for old, new in PATCHES:
            source = source.replace(old, new)
        shutil.copy2(shim, stage / 'bin' / shim.name)
        (stage / 'bin/plugin.js').write_text(PREFIX + source)
        # This is an explicitly modified local derivative, not a signed vendor package.
        manifest.pop('Files', None)
        manifest.pop('Marketplace', None)
        manifest['Name'] = DERIVATIVE_NAME
        manifest['Description'] = DERIVATIVE_DESCRIPTION
        manifest['OS'] = [{'Platform': 'linux', 'MinimumVersion': '1'}]
        manifest.setdefault('ApplicationsToMonitor', {})['linux'] = ['spotify']
        (stage / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        update_localizations(stage)
        plugin.rename(backup)
        try:
            stage.rename(plugin)
        except BaseException:
            backup.rename(plugin)
            raise
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    print('Installed Spotify for Linux; authenticated original backup:', backup)
    print('Original bundle SHA256:', hashlib.sha256((backup / 'bin/plugin.js').read_bytes()).hexdigest())


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('plugin', type=Path)
    parser.add_argument('--reference-app', type=Path, required=True)
    args = parser.parse_args()
    patch(args.plugin.resolve(strict=True), args.reference_app.resolve(strict=True))
