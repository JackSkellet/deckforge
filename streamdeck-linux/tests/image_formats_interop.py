#!/usr/bin/env python3
"""Official SDK image formats, shared dial decoder and portable artwork. No USB."""
import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import tempfile
import uuid
import zipfile
from PIL import Image
from display_targets_interop import make_plugin, wait, rows, action, decode, KEY, DIAL
from sample_interop import require, COUNTER_LOCK_SHA256
from system_interop import Host


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    sample = project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256,
            'Official SDK lock changed')
    sdk = sample/'node_modules/@elgato/streamdeck'
    require(json.loads((sdk/'package.json').read_text())['version'] == '2.0.4', 'SDK version changed')
    tracked = ['dist/plugin/actions/key.js', 'dist/plugin/actions/dial.js', 'dist/plugin/connection.js']
    hashes = {name: hashlib.sha256((sdk/name).read_bytes()).hexdigest() for name in tracked}
    report = {'usb_access': False, 'sdk': '2.0.4', 'checks': []}
    with tempfile.TemporaryDirectory(prefix='deckforge-image-formats-') as temporary:
        root = Path(temporary)
        plugin, proof = make_plugin(root, sdk)
        color = (210, 40, 70)
        for name in ('art.icon.webp', 'dial.icon@2x.webp'):
            Image.new('RGBA', (144, 144), (*color, 255)).save(plugin/name, lossless=True)
        Image.new('RGB', (144, 144), (20, 170, 80)).save(plugin/'lossy.webp', quality=95)
        Image.new('RGB', (144, 144), (20, 40, 210)).save(plugin/'blue.jpeg', quality=100)
        Image.new('RGB', (144, 144), (20, 40, 210)).save(plugin/'blue.png')
        manifest = json.loads((plugin/'manifest.json').read_text())
        manifest['Actions'][0]['States'][0]['Image'] = 'art.icon'
        manifest['Actions'][1]['Encoder'] = {'Icon': 'dial.icon', 'layout': 'full.json'}
        (plugin/'manifest.json').write_text(json.dumps(manifest))
        (plugin/'full.json').write_text(json.dumps({'id': 'full', 'items': [
            {'key': 'art', 'type': 'pixmap', 'rect': [0, 0, 200, 100], 'value': ''}]}))
        state = root/'state'
        host = Host(binary, [plugin], state, root/'host.log').ready()
        operations = 0
        def call(method, value, controller='Keypad', **options):
            nonlocal operations
            item = action(host, controller)
            nonce = str(uuid.uuid4())
            settings = dict(item['settings'], displayProbe={
                'nonce': nonce, 'method': method, 'value': value, 'options': options})
            host.post({'command': 'settings', 'context': item['context'], 'settings': settings})
            wait(host, proof, lambda: any(r.get('nonce') == nonce and r['kind'] == 'done' for r in rows(proof)),
                 'Image SDK round-trip')
            operations += 1
        def pixel_matches(value, expected, point=(0, 0), tolerance=0):
            actual = decode(value).getpixel(point)
            require(all(abs(a-b) <= tolerance for a, b in zip(actual, expected)),
                    f'Expected {expected}, got {actual}')
        try:
            wait(host, proof, lambda: host.state()['plugins'][0]['connected'], 'SDK registration')
            for controller, action_id in [('Keypad', KEY), ('Encoder', DIAL)]:
                host.post({'command': 'assign', 'controller': controller, 'index': 0, 'uuid': action_id})
            pixel_matches(action(host)['preview'], color)
            pixel_matches(action(host, 'Encoder')['software_icon'], color, (30, 30))
            report['checks'].append('extensionless manifest key and high-resolution dial WebP')
            original = (plugin/'art.icon.webp').read_bytes()
            baseline = root/'manifest.streamDeckProfile'
            host.post({'command': 'export_profile', 'path': str(baseline)})
            with zipfile.ZipFile(baseline) as archive:
                require(any(n.endswith('.webp') and archive.read(n) == original for n in archive.namelist()),
                        'Export flattened or lost extensionless WebP artwork')
            report['checks'].append('manifest export preserves original WebP bytes')
            for file, expected, tolerance in [('blue.png', (20, 40, 210), 0), ('blue.jpeg', (20, 40, 210), 3),
                                               ('lossy.webp', (20, 170, 80), 4), ('art.icon.webp', color, 0)]:
                call('setImage', file)
                pixel_matches(action(host)['preview'], expected, tolerance=tolerance)
            url = 'data:image/webp;base64,' + base64.b64encode(original).decode()
            call('setImage', url, target=2)
            pixel_matches(action(host)['preview'], color)
            report['checks'].append('SDK PNG JPEG lossy/lossless WebP files and data URL')
            previous = action(host)['preview']
            call('setImage', 'data:image/webp;base64,' + base64.b64encode(b'RIFF\x10\0\0\0WEBPbroken').decode())
            require(action(host)['preview'] == previous, 'Malformed WebP changed the current image')
            require(any(line.startswith('SDK command rejected:') for line in host.state()['logs']), 'Decode error not reported')
            report['checks'].append('malformed WebP rejection preserves current display')
            call('setImage', url, 'Encoder')
            pixel_matches(action(host, 'Encoder')['software_icon'], color, (30, 30))
            for value in ('art.icon.webp', url):
                call('setFeedback', {'art': value}, 'Encoder')
                pixel_matches(action(host, 'Encoder')['preview'], color, (100, 50))
            report['checks'].append('SDK dial icon and layout pixmap files/data URL')
            custom = root/'custom.webp'
            Image.new('RGBA', (144, 144), (10, 180, 230, 255)).save(custom, lossless=True)
            custom_bytes = custom.read_bytes()
            host.post({'command': 'appearance', 'context': action(host)['context'], 'expected_state': action(host)['state'], 'image_path': str(custom)})
            pixel_matches(action(host)['preview'], (10, 180, 230))
            exported = root/'custom.streamDeckProfile'
            host.post({'command': 'export_profile', 'path': str(exported)})
            with zipfile.ZipFile(exported) as archive:
                require(any(n.endswith('.webp') and archive.read(n) == custom_bytes for n in archive.namelist()),
                        'User artwork not portable')
            custom.unlink()
        finally:
            host.stop()
        host = Host(binary, [], state, root/'restart.log').ready()
        try:
            pixel_matches(action(host)['preview'], (10, 180, 230))
            host.post({'command': 'import_profile', 'path': str(exported)})
            pixel_matches(action(host)['preview'], (10, 180, 230))
            report['checks'].append('user WebP override survives original deletion, restart and profile import')
        finally:
            host.stop()
    require(all(hashlib.sha256((sdk/name).read_bytes()).hexdigest() == digest for name, digest in hashes.items()),
            'Official SDK was modified')
    report.update(ok=True, operations=operations, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS:', '; '.join(report['checks']))


if __name__ == '__main__':
    main()
