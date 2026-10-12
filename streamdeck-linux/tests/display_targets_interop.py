#!/usr/bin/env python3
"""Exercise display targets through unchanged official SDK2.0.4, simulation only.

The generated plugin uses real KeyAction/DialAction methods. getSettings is a
same-socket barrier, not an invented display acknowledgment. Software PNGs are
inspected here; physical Draw frames belong to separate Rust renderer tests.
"""
import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import tempfile
import time
import uuid

from PIL import Image
from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT, require
from system_interop import Host

PLUGIN = 'com.deckforge.display-target-probe'
KEY = PLUGIN + '.key'
DIAL = PLUGIN + '.dial'
BASE = (16, 32, 48)
BLUE = (20, 40, 220)
RED = (220, 30, 40)
GREEN = (20, 200, 50)
CUSTOM = (15, 190, 200)
ORANGE = (230, 140, 20)

JS = r'''import streamDeck from __SDK__;
import {connection} from __CONNECTION__;
import {appendFileSync} from 'node:fs';
const proof=__PROOF__,seen=new Set();let queue=Promise.resolve();
const record=(kind,fields={})=>appendFileSync(proof,JSON.stringify({kind,pid:process.pid,...fields})+'\n');
connection.on('didReceiveSettings',message=>{
 const q=message.payload?.settings?.displayProbe;if(!q?.nonce||seen.has(q.nonce))return;
 seen.add(q.nonce);queue=queue.then(async()=>{
  const a=streamDeck.actions.getActionById(message.context);
  if(!a)throw Error('Missing SDK action instance');
  const options=q.options??{};
  switch(q.method){
   case 'setTitle':await a.setTitle(q.omit?undefined:q.value,options);break;
   case 'setImage':await a.setImage(q.omit?undefined:q.value,options);break;
   case 'setState':await a.setState(q.value);break;
   case 'setFeedback':await a.setFeedback(q.value);break;
   default:throw Error('Unknown generated fixture command');
  }
  const settings=await a.getSettings();
  record('done',{nonce:q.nonce,context:a.id,instance:a.constructor.name,method:q.method,settings});
 }).catch(error=>record('failure',{nonce:q.nonce,message:String(error),stack:error.stack}));
});
process.on('unhandledRejection',error=>record('failure',{message:String(error)}));
await streamDeck.connect();record('registered');
'''


def rows(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines(keepends=True)
            if line.endswith('\n')]


def action(host, controller='Keypad'):
    return next(s for s in host.state()['slots']
                if s['controller'] == controller and s['index'] == 0)


def decode(value):
    require(bool(value), 'Expected nonempty base64 PNG')
    return Image.open(io.BytesIO(base64.b64decode(value, validate=True))).convert('RGB')


def corner(value):
    return decode(value).getpixel((0, 0))


def image_digest(value):
    image = decode(value)
    return hashlib.sha256(image.tobytes()).hexdigest()


def wait(host, proof, predicate, description):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        host.check_process()
        failures = [row for row in rows(proof) if row['kind'] == 'failure']
        require(not failures, 'SDK fixture failure: ' + repr(failures[-2:]))
        if predicate():
            return
        time.sleep(.02)
    raise AssertionError(description + '\n' + host.log.read_text()[-3000:])


def make_plugin(root, sdk):
    plugin = root / (PLUGIN + '.sdPlugin')
    plugin.mkdir(mode=0o700)
    proof = plugin / 'proof.jsonl'
    script = (JS.replace('__SDK__', json.dumps((sdk / 'dist/plugin/index.js').as_uri()))
              .replace('__CONNECTION__', json.dumps((sdk / 'dist/plugin/connection.js').as_uri()))
              .replace('__PROOF__', json.dumps(str(proof))))
    (plugin / 'plugin.mjs').write_text(script)
    for name, color in {'base': BASE, 'blue': BLUE, 'red': RED, 'green': GREEN,
                        'encoder-base': ORANGE, 'encoder-blue': BLUE,
                        'lcd-feedback': GREEN}.items():
        (plugin / (name + '.svg')).write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120">'
            f'<rect width="120" height="120" fill="rgb{color}"/></svg>')
    states = [{'Image': 'base.svg', 'Title': f'Baseline {i}', 'FontSize': 12,
               'FontFamily': 'DejaVu Sans', 'TitleAlignment': 'middle'} for i in range(2)]
    manifest = {'UUID': PLUGIN, 'Name': 'Display target probe', 'Version': '1.0.0',
                'CodePath': 'plugin.mjs', 'Nodejs': {'Version': '20'}, 'SDKVersion': 2,
                'Software': {'MinimumVersion': '7.1'},
                'Actions': [
                    {'UUID': KEY, 'Name': 'Two-state key', 'Controllers': ['Keypad'],
                     'States': states, 'DisableAutomaticStates': True},
                    {'UUID': DIAL, 'Name': 'Separate encoder icon', 'Controllers': ['Encoder'],
                     'States': [{'Image': 'base.svg'}],
                     'Encoder': {'Icon': 'encoder-base', 'layout': '$B1'}}]}
    (plugin / 'manifest.json').write_text(json.dumps(manifest))
    return plugin, proof


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/debug/deckforge')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    sample = project.parent / 'evidence/official-samples/counter'
    require(hashlib.sha256((sample / 'package-lock.json').read_bytes()).hexdigest()
            == COUNTER_LOCK_SHA256, 'Retained SDK dependency lock changed')
    sdk = sample / 'node_modules/@elgato/streamdeck'
    require(json.loads((sdk / 'package.json').read_text())['version'] == '2.0.4',
            'Expected unchanged SDK2.0.4')
    tracked = ['dist/plugin/actions/key.js', 'dist/plugin/actions/dial.js', 'dist/plugin/connection.js']
    hashes = {name: hashlib.sha256((sdk / name).read_bytes()).hexdigest() for name in tracked}
    with tempfile.TemporaryDirectory(prefix='deckforge-display-targets-') as temporary:
        root = Path(temporary)
        plugin, proof = make_plugin(root, sdk)
        custom = root / 'custom.png'
        Image.new('RGB', (120, 120), CUSTOM).save(custom)
        state = root / 'state'
        host = Host(binary, [plugin], state, root / 'host.log').ready()
        operation_count = 0
        try:
            wait(host, proof, lambda: host.state()['plugins'][0]['connected'], 'SDK registration')
            host.post({'command': 'assign', 'controller': 'Keypad', 'index': 0, 'uuid': KEY})
            host.post({'command': 'assign', 'controller': 'Encoder', 'index': 0, 'uuid': DIAL})
            context = action(host)['context']
            host.post({'command': 'settings', 'context': context,
                       'settings': {'preserved': {'future': 73}}})

            def sdk_call(method, value=None, *, target=None, state=None, omit=False,
                         controller='Keypad'):
                nonlocal operation_count
                slot = action(host, controller)
                nonce = str(uuid.uuid4())
                options = {}
                if target is not None:
                    options['target'] = target
                if state is not None:
                    options['state'] = state
                settings = dict(slot['settings'])
                settings['displayProbe'] = {'nonce': nonce, 'method': method, 'value': value,
                                            'omit': omit, 'options': options}
                host.post({'command': 'settings', 'context': slot['context'], 'settings': settings})
                wait(host, proof, lambda: any(row.get('nonce') == nonce and row['kind'] == 'done'
                                             for row in rows(proof)), method + ' SDK barrier')
                done = next(row for row in rows(proof) if row.get('nonce') == nonce and row['kind'] == 'done')
                require(done['instance'] == ('KeyAction' if controller == 'Keypad' else 'DialAction'),
                        'Fixture did not use the official contextualized action class')
                operation_count += 1

            require(corner(action(host)['preview']) == BASE, 'Baseline key PNG mismatch')
            sdk_call('setImage', 'red.svg', target=0)
            sdk_call('setTitle', 'Both states', target=0)
            sdk_call('setImage', 'blue.svg', target=2, state=0)
            sdk_call('setTitle', 'Software zero', target=2, state=0)
            require(action(host)['title'] == 'Software zero', 'Software title not exposed')
            software_zero = image_digest(action(host)['preview'])
            require(corner(action(host)['preview']) == BLUE, 'Software image was not updated')
            sdk_call('setImage', 'green.svg', target=1, state=0)
            sdk_call('setTitle', 'Hardware zero', target=1, state=0)
            require(image_digest(action(host)['preview']) == software_zero,
                    'Hardware-only update changed software pixels')
            require(action(host)['title'] == 'Software zero', 'Hardware-only update changed software title')
            sdk_call('setState', 1)
            require(action(host)['title'] == 'Both states' and corner(action(host)['preview']) == RED,
                    'Omitted state did not update both key states')
            sdk_call('setTitle', 'Software one', target=2, state=1)
            sdk_call('setState', 0)
            require(image_digest(action(host)['preview']) == software_zero, 'State switch lost software state zero')
            sdk_call('setTitle', target=1, state=0, omit=True)
            sdk_call('setImage', target=1, state=0, omit=True)
            require(image_digest(action(host)['preview']) == software_zero,
                    'Hardware reset cleared the independent software display')
            sdk_call('setTitle', target=2, state=0, omit=True)
            sdk_call('setImage', target=2, state=0, omit=True)
            require(action(host)['title'] == 'Baseline 0' and corner(action(host)['preview']) == BASE,
                    'Software reset must reveal baseline, not an older shared runtime value')
            sdk_call('setState', 1)
            require(action(host)['title'] == 'Software one' and corner(action(host)['preview']) == RED,
                    'State zero reset changed state one')
            sdk_call('setTitle', 'Omitted target')
            sdk_call('setImage', 'blue.svg')
            sdk_call('setState', 0)
            require(action(host)['title'] == 'Omitted target' and corner(action(host)['preview']) == BLUE,
                    'Omitted target/state default did not reach software state zero')

            unchanged = image_digest(action(host)['preview'])
            for method, value in [('setTitle', 'Invalid target'), ('setImage', 'green.svg')]:
                before_logs = host.state()['logs']
                sdk_call(method, value, target=99)
                require(image_digest(action(host)['preview']) == unchanged, 'Invalid target changed pixels')
                require(action(host)['title'] == 'Omitted target', 'Invalid target changed title')
                require(host.state()['logs'] != before_logs and any('SDK command rejected' in line for line in host.state()['logs']),
                        'Invalid target was silently accepted')

            encoder = action(host, 'Encoder')
            require(corner(encoder['software_icon']) == ORANGE, 'Encoder.Icon default did not populate separate software icon')
            lcd_before = image_digest(encoder['preview'])
            sdk_call('setImage', 'encoder-blue.svg', controller='Encoder')
            require(corner(action(host, 'Encoder')['software_icon']) == BLUE, 'DialAction.setImage did not change app icon')
            require(image_digest(action(host, 'Encoder')['preview']) == lcd_before,
                    'App-only encoder setImage changed LCD preview')
            sdk_call('setFeedback', {'title': 'LCD feedback', 'value': '42', 'indicator': 42,
                                     'icon': 'lcd-feedback.svg'}, controller='Encoder')
            encoder = action(host, 'Encoder')
            require(image_digest(encoder['preview']) != lcd_before, 'setFeedback did not change LCD')
            require(decode(encoder['preview']).getpixel((40, 65)) == GREEN, 'Feedback icon missing from LCD pixmap')
            require(corner(encoder['software_icon']) == BLUE, 'Feedback changed independent app icon')
            lcd_feedback = image_digest(encoder['preview'])
            sdk_call('setImage', controller='Encoder', omit=True)
            require(corner(action(host, 'Encoder')['software_icon']) == ORANGE, 'Encoder reset did not restore Encoder.Icon')
            require(image_digest(action(host, 'Encoder')['preview']) == lcd_feedback,
                    'Encoder app-icon reset cleared LCD feedback')

            host.post({'command': 'appearance', 'context': context, 'expected_state': 0,
                       'title': 'User title', 'image_path': str(custom), 'title_parameters': {'showTitle': False}})
            for target in [0, 1, 2]:
                sdk_call('setTitle', f'Hidden SDK {target}', target=target)
                sdk_call('setImage', 'red.svg', target=target)
                require(action(host)['title'] == 'User title', 'User title lost precedence')
                require(corner(action(host)['preview']) == CUSTOM, 'User image lost precedence')
            sdk_call('setTitle', target=2, state=0, omit=True)
            sdk_call('setImage', target=2, state=0, omit=True)
            require(action(host)['title'] == 'User title' and corner(action(host)['preview']) == CUSTOM,
                    'Target reset erased user overrides')
            require(action(host)['settings']['preserved'] == {'future': 73}, 'Display commands changed saved settings')
            host.stop()
            host = Host(binary, [], state, root / 'restart.log').ready()
            wait(host, proof, lambda: host.state()['plugins'][0]['connected'], 'SDK restart registration')
            require(action(host)['context'] == context, 'Restart changed saved action context')
            require(action(host)['title'] == 'User title' and corner(action(host)['preview']) == CUSTOM,
                    'Restart lost authored appearance')
            require(action(host)['settings']['preserved'] == {'future': 73}, 'Restart lost settings')
            require(all(hashlib.sha256((sdk / name).read_bytes()).hexdigest() == digest
                        for name, digest in hashes.items()), 'Official SDK source was modified')
            report = {'passed': True, 'host_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                      'sdk_version': '2.0.4', 'sample_commit': SAMPLE_COMMIT, 'sdk_hashes': hashes,
                      'sdk_operations': operation_count, 'key_target_isolation_software_pixels': True,
                      'omitted_state_both': True, 'independent_resets': True, 'invalid_target_atomic': True,
                      'encoder_software_icon_separate_from_lcd': True,
                      'user_overrides_settings_restart': True,
                      'boundary': 'Real simulated Rust host and unchanged official SDK calls; software PNG inspection. No USB or physical-display acceptance. Hardware Draw isolation is tested separately in Rust. Raw encoder target flags and empty-image native semantics are intentionally not inferred.'}
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2) + '\n')
        except Exception:
            print(host.log.read_text()[-4000:])
            raise
        finally:
            host.stop()
    print('PASS: unchanged SDK display target isolation, resets/states, encoder app icon/LCD separation and user persistence; simulation only.')


if __name__ == '__main__':
    main()
