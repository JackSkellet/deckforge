#!/usr/bin/env python3
"""Action Trigger SDK, timing, nesting and profile interoperability in --simulate.

Generated protocol probes run beside the unchanged official Counter sample.
No USB, keyboard injection, audio playback or desktop action is executed.
"""
import argparse
import base64
import io
from PIL import Image, ImageChops
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time
import uuid
import zipfile

from system_interop import Host
from sample_interop import COUNTER, SAMPLE_COMMIT, require, slot

TRIGGER = 'com.elgato.streamdeck.keys.adaptor'
STACK = 'com.elgato.streamdeck.dial.stack'
ROUTINE = 'com.elgato.streamdeck.multiactions.routine'
DELAY = 'com.elgato.streamdeck.multiactions.delay'
PROBE = 'com.deckforge.trigger-probe.key'
MANUAL = 'com.deckforge.trigger-probe.manual'
ENCODER = 'com.deckforge.trigger-probe.encoder'


def make_probe(path, require_root):
    path.mkdir()
    (path / 'plugin.js').write_text(r'''import {createRequire} from 'node:module';
const WebSocket=createRequire(__ROOT__)('ws');const args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);let seq=0;
const send=(event,context,payload)=>ws.send(JSON.stringify({event,context,payload}));
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{const m=JSON.parse(data);if(!m.payload)return;
 if(['willAppear','willDisappear','keyDown','keyUp','dialDown','dialUp','dialRotate','touchTap'].includes(m.event))
 send('logMessage',args['-pluginUUID'],{message:'TRIGGER '+JSON.stringify({seq:++seq,event:m.event,context:m.context,payload:m.payload,at:performance.now()})});
 if(m.event==='keyUp'){const count=(m.payload.settings.count||0)+1;send('setSettings',m.context,{...m.payload.settings,count});send('setTitle',m.context,{title:String(count)});}
});'''.replace('__ROOT__', json.dumps(str(require_root))))
    (path / 'manifest.json').write_text(json.dumps({
        'UUID': 'com.deckforge.trigger-probe', 'Name': 'Trigger probe',
        'CodePath': 'plugin.js', 'Nodejs': {'Version': '20'}, 'SDKVersion': 2,
        'Actions': [
            {'UUID': PROBE, 'Name': 'Key probe', 'Controllers': ['Keypad'],
             'States': [{}, {}], 'MultiActionDelays': {'Delay1': 1, 'Delay2': 1}},
            {'UUID': MANUAL, 'Name': 'Manual state probe', 'Controllers': ['Keypad'],
             'States': [{}, {}], 'DisableAutomaticStates': True},
            {'UUID': ENCODER, 'Name': 'Encoder probe', 'Controllers': ['Encoder'],
             'States': [{}], 'Encoder': {'layout': '$B1'}},
        ],
    }))
    return path


def child(action=PROBE, **extra):
    return {'UUID': action, 'ActionID': str(uuid.uuid4()), 'Name': action,
            'Settings': {}, 'State': 0, 'States': [{}], **extra}


def records(archive):
    with zipfile.ZipFile(archive) as z:
        return [a for name in z.namelist() if name.endswith('manifest.json')
                for controller in json.loads(z.read(name)).get('Controllers', [])
                for a in (controller.get('Actions') or {}).values()]


def events(host, since=0, contexts=None, kinds=None):
    rows = [json.loads(line.split('TRIGGER ', 1)[1])
            for line in host.state()['logs'] if 'TRIGGER ' in line]
    return [r for r in rows if r['seq'] > since
            and (contexts is None or r['context'] in contexts)
            and (kinds is None or r['event'] in kinds)]


def mark(host):
    return max((e['seq'] for e in events(host)), default=0)


def wait_for(host, predicate, description, timeout=5):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        host.check_process()
        if predicate():
            return
        time.sleep(.005)
    raise AssertionError('Timed out: ' + description + '\n' + host.log.read_text()[-5000:])


def input_(host, event, **fields):
    host.post({'command': 'input', 'event': event, 'index': 0, **fields})


def click(host):
    input_(host, 'dialDown')
    input_(host, 'dialUp')


def current(host):
    return slot(host.state(), TRIGGER)


def configure(host, children=None):
    host.post({'command': 'assign', 'controller': 'Encoder', 'index': 0, 'uuid': TRIGGER})
    parent = current(host)
    raw = {'FutureContainer': 27, 'Settings': {}, 'Actions': children or [
        child(Settings={'tag': name}, FutureChild=i,
              MultiActionDelays={'Delay1': 1, 'Delay2': 1})
        for i, name in enumerate(['left', 'center', 'right'])]}
    host.post({'command': 'trigger_save', 'context': parent['context'], 'raw': raw})
    return current(host)


def gesture_rows(host, since, ids):
    return events(host, since, ids, ('keyDown', 'keyUp'))


def assert_normal(event):
    payload = event['payload']
    require(payload['controller'] == 'Keypad', 'Trigger child controller is not Keypad')
    require(payload['isInMultiAction'] is False, 'Trigger child marked as Multi Action')
    require(payload['coordinates'] == {'column': 0, 'row': 0}, 'Parent dial coordinates lost')
    require('userDesiredState' not in payload, 'Trigger child received userDesiredState')


def verify_states(host, context, snapshot, fire, automatic):
    """The probe never calls setState: state changes must come from the host."""
    start = mark(host)
    require(snapshot()['state'] == 0, 'State fixture did not start at zero')
    for press in range(2):
        before = press if automatic else 0
        after = (press + 1) % 2 if automatic else 0
        fire()
        wait_for(host, lambda: len(events(host, start, [context], ['keyUp'])) == press + 1,
                 'state probe release')
        wait_for(host, lambda: snapshot()['state'] == after, 'state after release')
        rows = gesture_rows(host, start, [context])
        pair = rows[press * 2:press * 2 + 2]
        require([e['event'] for e in pair] == ['keyDown', 'keyUp'], 'State probe event order changed')
        require([e['payload']['state'] for e in pair] == [before, before],
                f'Gesture did not report its pre-transition state: {pair}')
        require(all(e['payload']['isInMultiAction'] is False for e in pair),
                'Normal state probe became a Multi Action child')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/deckforge'))
    args = parser.parse_args()
    evidence = Path(__file__).resolve().parents[2] / 'evidence'
    sample = evidence / 'official-samples'
    counter = sample / 'counter/com.elgato.counter.sdPlugin'
    require((counter / 'bin/plugin.js').exists(), 'Build the official Counter fixture first')
    require(subprocess.check_output(['git', '-C', str(sample), 'rev-parse', 'HEAD'], text=True).strip()
            == SAMPLE_COMMIT, 'Official sample commit differs from pinned source')
    vendor_paths = [counter / 'manifest.json', counter / 'bin/plugin.js']
    hashes = {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in vendor_paths}
    require(json.loads(vendor_paths[0].read_text())['Actions'][0]['SupportedInMultiActions'] is False,
            'Counter compatibility test requires its unchanged opt-out manifest')
    with tempfile.TemporaryDirectory(prefix='deckforge-trigger-') as directory:
        root = Path(directory)
        state = root / 'state'
        probe = make_probe(root / 'probe.sdPlugin', sample / 'counter/package.json')
        host = Host(args.binary.resolve(), [probe, counter], state, root / 'host.log')
        try:
            host.ready()
            host.wait(lambda s: all(p['connected'] for p in s['plugins']), 'plugin connections')
            parent = configure(host)
            ids = [c['context'] for c in parent['trigger_children']]
            wait_for(host, lambda: len(events(host, contexts=ids, kinds=['willAppear'])) == 3,
                     'all three child appearances')
            for e in events(host, contexts=ids, kinds=['willAppear']):
                assert_normal(e)
            start = mark(host)
            input_(host, 'dialDown')
            wait_for(host, lambda: len(gesture_rows(host, start, ids)) == 1, 'direct center down')
            time.sleep(.09)
            require(len(gesture_rows(host, start, ids)) == 1, 'Physical hold used queued release timer')
            input_(host, 'dialUp')
            wait_for(host, lambda: len(gesture_rows(host, start, ids)) == 2, 'direct center up')
            rows = gesture_rows(host, start, ids)
            require([r['context'] for r in rows] == [ids[1]] * 2, 'Push did not use center child')
            require(rows[1]['at'] - rows[0]['at'] >= 85, 'Physical hold duration lost')
            for row in rows:
                assert_normal(row)
            # Fixed pulse timing must ignore deliberately tiny MultiActionDelays.
            start = mark(host)
            input_(host, 'dialRotate', ticks=10)
            wait_for(host, lambda: len(events(host, start, [ids[2]], ['keyUp'])) == 3, 'three capped pulses')
            time.sleep(.12)
            rows = gesture_rows(host, start, ids)
            require([r['event'] for r in rows] == ['keyDown', 'keyUp'] * 3, 'Initial burst cap is not three total')
            require(all(r['context'] == ids[2] for r in rows), 'CW chose wrong child')
            for i in range(0, 6, 2):
                require(rows[i + 1]['at'] - rows[i]['at'] >= 50, '60ms queued hold compressed')
            for i in [1, 3]:
                require(rows[i + 1]['at'] - rows[i]['at'] >= 10, '15ms queued gap compressed')
            for row in rows:
                assert_normal(row)
            # Touch only the three icon rectangles, including a held touch.
            for index, (x, y) in enumerate([(40, 55), (100, 40), (160, 55)]):
                start = mark(host)
                input_(host, 'touchTap', x=x, y=y, hold=index == 2)
                wait_for(host, lambda: len(events(host, start, [ids[index]], ['keyUp'])) == 1, 'touch pulse')
                require(len(gesture_rows(host, start, ids)) == 2, 'Touch activated multiple children')
            start = mark(host)
            input_(host, 'touchTap', x=5, y=5, hold=False)
            input_(host, 'touchTap', x=70, y=90, hold=True)
            time.sleep(.12)
            require(not gesture_rows(host, start, ids), 'Touch outside icon rectangles fired')
            # Replacing a queued target releases ownership and drops its remaining pulses.
            start = mark(host)
            input_(host, 'dialRotate', ticks=3)
            wait_for(host, lambda: bool(events(host, start, [ids[2]], ['keyDown'])), 'right queue started')
            input_(host, 'dialRotate', ticks=-1)
            wait_for(host, lambda: bool(events(host, start, [ids[0]], ['keyUp'])), 'replacement left pulse')
            time.sleep(.25)
            rows = gesture_rows(host, start, ids)
            first_left = next(i for i, r in enumerate(rows) if r['context'] == ids[0])
            require(not any(r['context'] == ids[2] for r in rows[first_left:]), 'Replaced right queue continued')
            require(sum(r['event'] == 'keyDown' for r in rows if r['context'] == ids[2]) ==
                    sum(r['event'] == 'keyUp' for r in rows if r['context'] == ids[2]), 'Replacement unbalanced key ownership')
            # Page cancellation releases one owned pulse, never starts later work.
            original_page = host.state()['library']['active_page']
            start = mark(host)
            input_(host, 'dialRotate', ticks=3)
            wait_for(host, lambda: bool(events(host, start, [ids[2]], ['keyDown'])), 'queue before page change')
            host.post({'command': 'page_create', 'name': 'Away'})
            wait_for(host, lambda: len(events(host, start, ids, ['willDisappear'])) == 3, 'all children disappeared')
            time.sleep(.25)
            rows = events(host, start, ids)
            disappearing = next(i for i, r in enumerate(rows) if r['event'] == 'willDisappear')
            require(not any(r['event'] == 'keyDown' for r in rows[disappearing:]), 'Hidden trigger started work')
            host.post({'command': 'page_select', 'id': original_page})
            require([c['context'] for c in current(host)['trigger_children']] == ids, 'Page cycle changed child identities')
            # Blanks retain positions; a valid third child stays the CW target.
            parent = configure(host, [{'Actions': []}, {'Actions': []}, child(Settings={'tag': 'only-right'})])
            require(parent['trigger_children'][:2] == [None, None], 'Blank positions shifted')
            right = parent['trigger_children'][2]['context']
            start = mark(host)
            click(host)
            input_(host, 'dialRotate', ticks=-1)
            time.sleep(.1)
            require(not gesture_rows(host, start, [right]), 'Blank slot activated right child')
            input_(host, 'dialRotate', ticks=1)
            wait_for(host, lambda: bool(events(host, start, [right], ['keyUp'])), 'CW retained third position')
            # A nested Multi Action uses its own child semantics and cancellation.
            nested = child(ROUTINE, Actions=[{'Actions': [
                child(Settings={'tag': 'nested-first'}, OverrideState=1),
                child(DELAY, Settings={'delay': 1500}),
                child(Settings={'tag': 'nested-last'})]}, {'Actions': []}])
            parent = configure(host, [{'Actions': []}, nested, {'Actions': []}])
            routine = parent['trigger_children'][1]
            first = routine['multi_children'][0][0]['context']
            last = routine['multi_children'][0][2]['context']
            start = mark(host)
            click(host)
            wait_for(host, lambda: bool(events(host, start, [first], ['keyUp'])), 'nested routine dispatch')
            e = events(host, start, [first], ['keyDown'])[0]
            require(e['payload']['isInMultiAction'] is True and e['payload']['userDesiredState'] == 1
                    and 'coordinates' not in e['payload'], 'Nested Multi Action payload lost its own semantics')
            require(current(host)['trigger_children'][1]['multi_children'][0][0]['state'] == 0,
                    'Multi Action child received ordinary automatic state advancement')
            host.post({'command': 'page_create', 'name': 'Cancel nested'})
            host.wait(lambda s: not s['multi_runs'], 'nested routine cancelled')
            require(not events(host, start, [last], ['keyDown']), 'Cancelled nested final step ran')
            # An Action Trigger may be the selected child of a Dial Stack.
            host.post({'command': 'assign', 'controller': 'Encoder', 'index': 0, 'uuid': STACK})
            stack = slot(host.state(), STACK)
            trigger = child(TRIGGER, Actions=[child(), child(), child()])
            host.post({'command': 'stack_save', 'context': stack['context'], 'raw': {
                'Settings': {'CurrentIdx': 0, 'FastNavigation': False, 'ShowStackIcon': True},
                'Actions': [trigger, child(ENCODER)]}})
            nested_parent = slot(host.state(), STACK)['stack_children'][0]
            stack_ids = [c['context'] for c in nested_parent['trigger_children']]
            wait_for(host, lambda: len(events(host, contexts=stack_ids, kinds=['willAppear'])) == 3,
                     'stack selected trigger children')
            start = mark(host)
            input_(host, 'dialRotate', ticks=3)
            wait_for(host, lambda: bool(events(host, start, [stack_ids[2]], ['keyDown'])), 'stack trigger rotation')
            click(host)
            host.wait(lambda s: slot(s, STACK)['settings']['CurrentIdx'] == 1, 'stack selected another action')
            wait_for(host, lambda: len(events(host, start, stack_ids, ['willDisappear'])) == 3,
                     'stack deselection child lifecycle')
            time.sleep(.25)
            require(not events(host, start, [stack_ids[1]], ['keyDown']), 'Stack cycling leaked center push')
            # Direct and queued ordinary plugin keys advance after keyUp unless opted out.
            parent = configure(host, [child(MANUAL, States=[{}, {}]),
                                      child(States=[{}, {}]), child(States=[{}, {}])])
            state_ids = [c['context'] for c in parent['trigger_children']]
            verify_states(host, state_ids[1], lambda: current(host)['trigger_children'][1],
                          lambda: click(host), automatic=True)
            verify_states(host, state_ids[2], lambda: current(host)['trigger_children'][2],
                          lambda: input_(host, 'dialRotate', ticks=1), automatic=True)
            verify_states(host, state_ids[0], lambda: current(host)['trigger_children'][0],
                          lambda: input_(host, 'dialRotate', ticks=-1), automatic=False)
            # The same contract applies to ordinary device keys, not only Trigger children.
            for index, action, automatic in [(4, PROBE, True), (5, MANUAL, False)]:
                host.post({'command': 'assign', 'controller': 'Keypad', 'index': index, 'uuid': action})
                key = next(s for s in host.state()['slots']
                           if s['controller'] == 'Keypad' and s['index'] == index)
                context = key['context']
                def snapshot(context=context):
                    return next(s for s in host.state()['slots'] if s['context'] == context)
                def fire(index=index):
                    host.post({'command': 'input', 'event': 'keyDown', 'index': index})
                    host.post({'command': 'input', 'event': 'keyUp', 'index': index})
                verify_states(host, context, snapshot, fire, automatic)
            # Unchanged Counter explicitly opts out of Multi Actions but works here.
            parent = configure(host, [child(COUNTER, Settings={'count': 4, 'incrementBy': 2}),
                                      {'Actions': []}, child(COUNTER, Settings={'count': 10})])
            host.wait(lambda s: current(host)['trigger_children'][0]['title'] == '4', 'official Counter appeared')
            before_image = Image.open(io.BytesIO(base64.b64decode(current(host)['preview']))).convert('RGB')
            input_(host, 'dialRotate', ticks=-2)
            host.wait(lambda s: current(host)['trigger_children'][0]['settings'].get('count') == 8,
                      'unchanged Counter two queued presses')
            require(current(host)['trigger_children'][2]['settings']['count'] == 10, 'Counter instances interfered')
            after_image = Image.open(io.BytesIO(base64.b64decode(current(host)['preview']))).convert('RGB')
            box = ImageChops.difference(before_image, after_image).getbbox()
            require(box and box[0] >= 12 and box[1] >= 29 and box[2] <= 68 and box[3] <= 85,
                    f'Counter pixels changed outside the left icon, or did not change: {box}')
            exported = root / 'trigger.streamDeckProfile'
            host.post({'command': 'export_profile', 'path': str(exported)})
            original_export = next(a for a in records(exported) if a.get('UUID') == TRIGGER
                                   and a.get('Actions', [{}])[0].get('UUID') == COUNTER)
            require(original_export['Actions'][1] == {'Actions': []}, 'Export lost exact blank placeholder')
            require(original_export['Actions'][0]['Settings']['count'] == 8, 'Export lost child settings')
            require(original_export['FutureContainer'] == 27, 'Export lost unknown parent field')
            retained_ids = [c['context'] if c else None for c in current(host)['trigger_children']]
        finally:
            host.stop()
        restart = Host(args.binary.resolve(), [], state, root / 'restart.log')
        try:
            restart.ready()
            restart.wait(lambda s: all(p['connected'] for p in s['plugins']), 'restart plugin connections')
            require([c['context'] if c else None for c in current(restart)['trigger_children']] == retained_ids,
                    'Restart changed child identities or blanks')
            require(current(restart)['trigger_children'][0]['settings']['count'] == 8, 'Restart lost Counter settings')
            restart.post({'command': 'import_profile', 'path': str(exported)})
            require(current(restart)['trigger_children'][1] is None, 'Reimport shifted blank child')
            require(current(restart)['trigger_children'][0]['settings']['count'] == 8, 'Reimport lost Counter settings')
            # Original guide's page-control adaptor, including pinned context ownership.
            restart.post({'command': 'profile_create', 'name': 'Navigation'})
            navigation = configure(restart, [child('com.elgato.streamdeck.page.previous'),
                child('com.elgato.streamdeck.page.goto', Settings={'PageIndex': 1}),
                child('com.elgato.streamdeck.page.next')])
            restart.post({'command': 'slot_pin', 'context': navigation['context'], 'pinned': True})
            first_page = restart.state()['library']['active_page']
            restart.post({'command': 'page_create', 'name': 'Go to page target'})
            second_page = restart.state()['library']['active_page']
            click(restart)
            restart.wait(lambda s:s['library']['active_page'] == first_page, 'center goto page one')
            input_(restart, 'dialRotate', ticks=1)
            restart.wait(lambda s:s['library']['active_page'] == second_page, 'right next page')
            input_(restart, 'dialRotate', ticks=-1)
            restart.wait(lambda s:s['library']['active_page'] == first_page, 'left previous page')
            require(current(restart)['context'] == navigation['context'], 'Pinned navigation context changed')
            # Original Windows fixtures are imported/exported without activating their actions.
            contract = json.loads((evidence / 'action-trigger-contracts.json').read_text())
            groups = {}
            for fixture in contract['fixtures']:
                groups.setdefault(fixture['archive'], []).append(fixture)
            total = 0
            for i, (path, fixtures) in enumerate(groups.items()):
                source = evidence.parent / path
                require(hashlib.sha256(source.read_bytes()).hexdigest() == fixtures[0]['archive_sha256'],
                        'Original Windows source archive changed')
                # A foreign-model import must not replace the Plus active profile.
                # Run each original fixture on its own declared model instead.
                from models_interop import MODELS
                with zipfile.ZipFile(source) as archive:
                    declared = next(json.loads(archive.read(name))['Device']['Model']
                        for name in archive.namelist() if name.endswith('manifest.json')
                        and isinstance(json.loads(archive.read(name)).get('Device'),dict))
                model = next(name for name,values in MODELS.items() if values[4]==declared)
                fixture_host = Host(args.binary.resolve(), [], root/f'original-{i}', root/f'original-{i}.log', model=model)
                output = root / f'original-trigger-{i}.streamDeckProfile'
                try:
                    fixture_host.ready()
                    fixture_host.post({'command': 'import_profile', 'path': str(source)})
                    require(fixture_host.state()['device_model']==model,'Original fixture model changed')
                    fixture_host.post({'command': 'export_profile', 'path': str(output)})
                finally:
                    fixture_host.stop()
                exported_records = {a.get('ActionID'): a for a in records(output)}
                for fixture in fixtures:
                    original = fixture['action']
                    match = exported_records[original['ActionID']]
                    require(match['Actions'] == original['Actions'] and match['Settings'] == original['Settings'],
                            'Original Action Trigger child tree/settings changed on export')
                    total += 1
            require(total == 11, f'Expected all 11 original adaptors, checked {total}')
        finally:
            restart.stop()
    require(all(hashlib.sha256(p.read_bytes()).hexdigest() == digest for p, digest in hashes.items()),
            'Original Counter fixture files were modified')
    print('PASS: ordinary child SDK/lifecycle, physical push, capped 60/15ms pulses, touch rectangles, replacement/cancellation, blanks, nested Multi Action, Dial Stack, automatic/opt-out states on Trigger and normal keys, unchanged Counter, export/reimport/restart, all 11 original Windows adaptors. No hardware or desktop actions executed.')


if __name__ == '__main__':
    main()
