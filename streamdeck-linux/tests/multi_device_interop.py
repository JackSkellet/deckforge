#!/usr/bin/env python3
"""One unchanged official Counter process across multiple simulated devices.

The generated probe exchanges only documented loopback SDK frames. It has no
Discord, USB, keyboard, audio, browser, or external-network behavior. All profiles,
resources, logs and command files belong to a temporary test directory.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import time
import zipfile
import zlib

from bundled_profiles_interop import png_corner
from sample_interop import COUNTER, COUNTER_LOCK_SHA256, SAMPLE_COMMIT, require
from system_interop import Host

PLUGIN = 'com.deckforge.multi-device-probe'
ACTION = PLUGIN + '.action'
ROUTINE = 'com.elgato.streamdeck.multiactions.routine'
DELAY = 'com.elgato.streamdeck.multiactions.delay'
B, C = 'fixture-mini-b', 'fixture-mini-c'


def rows(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines(keepends=True) if line.endswith('\n')]


def wait(host, predicate, description, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        host.check_process()
        value = predicate()
        if value:
            return value
        time.sleep(.025)
    raise AssertionError(f'Timed out: {description}\n{host.log.read_text()[-3000:]}')


def selected(host, device):
    host.post({'command': 'device_select', 'device': device})
    state = host.state()
    require(state['device'] == device, 'Device selection did not change editor projection')
    return state


def inspect(host, device):
    previous = host.state()['device']
    try:
        return selected(host, device)
    finally:
        if previous != device:
            host.post({'command': 'device_select', 'device': previous})


def at(state, index, controller='Keypad'):
    return next((slot for slot in state['slots'] if slot['controller'] == controller and slot['index'] == index), None)


def post(host, device, command, **fields):
    host.post({'command': command, 'device': device, **fields})


def assign(host, device, index, action, settings=None):
    post(host, device, 'assign', controller='Keypad', index=index, uuid=action)
    slot = at(inspect(host, device), index)
    if settings is not None:
        post(host, device, 'settings', context=slot['context'], settings=settings)
    return slot['context']


def press(host, device, index):
    for event in ('keyDown', 'keyUp'):
        post(host, device, 'input', event=event, index=index)


def pid_map(host):
    return {p['id']: p['pid'] for p in host.state()['plugins']}


def solid_png(red, green, blue):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes([0, red, green, blue, red, green, blue]) * 2)) + chunk(b'IEND', b'')


def create_probe(root, counter):
    root.mkdir()
    events, control = root/'events.jsonl', root/'commands.json'
    control.write_text(json.dumps({'seq': 0, 'frames': []}))
    source = r'''import {createRequire} from 'node:module';
import {appendFileSync,readFileSync} from 'node:fs';
const WebSocket=createRequire(__PACKAGE__)('ws'),args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const write=value=>appendFileSync(__EVENTS__,JSON.stringify(value)+'\n');
write({event:'fixtureLaunch',pid:process.pid,info:JSON.parse(args['-info'])});
let last=JSON.parse(readFileSync(__CONTROL__,'utf8')).seq;
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
ws.on('open',()=>{
 ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']}));
 setInterval(()=>{
  let request;try{request=JSON.parse(readFileSync(__CONTROL__,'utf8'));}catch{return;}
  if(request.seq<=last)return;last=request.seq;
  for(const original of request.frames){const frame=structuredClone(original);if(frame.context==='@registration')frame.context=args['-pluginUUID'];ws.send(JSON.stringify(frame));}
  ws.send(JSON.stringify({event:'getGlobalSettings',context:args['-pluginUUID'],id:'fixture-barrier-'+last}));
 },15);
});
ws.on('message',data=>write(JSON.parse(data)));
'''
    source = source.replace('__PACKAGE__', json.dumps(str(counter.parent/'package.json'))).replace('__EVENTS__', json.dumps(str(events))).replace('__CONTROL__', json.dumps(str(control)))
    (root/'plugin.mjs').write_text(source)
    (root/'manifest.json').write_text(json.dumps({
        'UUID': PLUGIN, 'Name': 'Multi-device contract probe', 'Version': '1.0.0', 'SDKVersion': 2,
        'CodePath': 'plugin.mjs', 'Nodejs': {'Version': '20'},
        'Actions': [{'UUID': ACTION, 'Name': 'Device probe', 'Controllers': ['Keypad'], 'States': [{}]}],
        'Profiles': [{'Name': 'mini-bundle', 'DeviceType': 1, 'AutoInstall': False, 'Readonly': False, 'DontAutoSwitchWhenInstalled': True}],
    }))
    with zipfile.ZipFile(root/'mini-bundle.streamDeckProfile', 'w') as archive:
        archive.writestr('manifest.json', json.dumps({
            'Name': 'Independent Mini bundle', 'Device': {'Model': '20GAI9901', 'UUID': ''},
            'Controllers': [{'Type': 'Keypad', 'Actions': {'0,0': {'UUID': ACTION, 'Name': 'Bundle probe', 'Settings': {'bundle': 'initial'}, 'States': [{}]}}}],
        }))
    return events, control, hashlib.sha256(source.encode()).hexdigest()


class SDK:
    def __init__(self, host, events, control):
        self.host, self.events, self.control = host, events, control
        self.seq = json.loads(control.read_text())['seq']

    def send(self, *frames):
        self.seq += 1
        pending = self.control.with_suffix('.tmp')
        pending.write_text(json.dumps({'seq': self.seq, 'frames': list(frames)}))
        pending.replace(self.control)
        marker = 'fixture-barrier-' + str(self.seq)
        wait(self.host, lambda: any(r.get('id') == marker for r in rows(self.events)), 'SDK frame ordering barrier')

    def request(self, event, context, **extra):
        request_id = f'fixture-request-{self.seq + 1}'
        self.send({'event': event, 'context': context, 'id': request_id, **extra})
        return next(r for r in reversed(rows(self.events)) if r.get('id') == request_id)

    def switch(self, device, profile=None, **payload):
        if profile is not None:
            payload['profile'] = profile
        self.send({'event': 'switchToProfile', 'context': '@registration', 'device': device, 'payload': payload})


def lifecycle(events):
    return [e for e in events if e.get('event') in ('willAppear', 'willDisappear')]


def check_counter(host, device, expected):
    return wait(host, lambda: (slot if (slot := at(inspect(host, device), 0)) and slot['settings'].get('count') == expected else None), f'Counter {device} count {expected}')


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/debug/deckforge')
    parser.add_argument('--samples', type=Path, default=project.parent/'evidence/official-samples')
    parser.add_argument('--report', type=Path)
    parser.add_argument('--skip-profile-copy', action='store_true', help='Development-only partial run; report explicitly records omitted acceptance.')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    counter = args.samples.resolve()/'counter/com.elgato.counter.sdPlugin'
    require(hashlib.sha256((counter.parent/'package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256, 'Official Counter lockfile source pin changed')
    manifest = json.loads((counter/'manifest.json').read_text())
    counter_entry_sha256 = hashlib.sha256((counter/manifest['CodePath']).read_bytes()).hexdigest()
    report = {'counter_entry_sha256': counter_entry_sha256, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'sample_commit': SAMPLE_COMMIT,
              'counter_lock_sha256': COUNTER_LOCK_SHA256, 'boundary': 'Loopback processes and simulated devices only; no USB, OS input injection, audio or external requests.',
              'profile_copy_tested': not args.skip_profile_copy}
    with tempfile.TemporaryDirectory(prefix='deckforge-multi-device-') as directory:
        root = Path(directory)
        probe = root/(PLUGIN+'.sdPlugin')
        proof, control, report['probe_sha256'] = create_probe(probe, counter)
        state_dir = root/'state'
        host = Host(binary, [counter, probe], state_dir, root/'host.log')
        try:
            host.ready()
            host.wait(lambda s: len(s['plugins']) == 2 and all(p['connected'] for p in s['plugins']), 'Counter and probe registration')
            A = host.state()['device']
            report['device_ids'] = [A, B, C]
            pids = pid_map(host)
            sdk = SDK(host, proof, control)
            host.post({'command': 'device_add', 'device': B, 'model': 'mini'})
            require(host.state()['device'] == A, 'Adding B changed editor selection')
            require(pid_map(host) == pids, 'Adding B restarted a plugin')
            require(len([r for r in rows(proof) if r['event'] == 'fixtureLaunch']) == 1, 'A second probe process was launched')
            a_counter = assign(host, A, 0, COUNTER, {'count': 1, 'incrementBy': 2})
            b_counter = assign(host, B, 0, COUNTER, {'count': 20, 'incrementBy': 7})
            a_probe = assign(host, A, 5, ACTION, {'owner': 'A'})
            b_probe = assign(host, B, 5, ACTION, {'owner': 'B'})
            require(len({a_counter, b_counter, a_probe, b_probe}) == 4, 'Device action contexts collide')
            initial_a, initial_b = inspect(host, A), inspect(host, B)
            sdk.send()
            name_offset = len(rows(proof))
            post(host, A, 'device_rename', name='Editing desk')
            post(host, B, 'device_rename', name='  Studio 🎛  ')
            sdk.send()
            changes = [e for e in rows(proof)[name_offset:] if e.get('event') == 'deviceDidChange']
            require([(e['device'], e['deviceInfo']['name']) for e in changes] == [(A, 'Editing desk'), (B, 'Studio 🎛')], 'Names did not reach the plugin as deviceDidChange')
            require(not lifecycle(rows(proof)[name_offset:]), 'Renaming recreated actions or connections')
            require(pid_map(host) == pids and host.state()['device'] == A, 'Renaming changed plugin process or editor selection')
            for device, before in [(A, initial_a), (B, initial_b)]:
                require([s['context'] for s in inspect(host, device)['slots']] == [s['context'] for s in before['slots']], 'Renaming changed action identities')
            host.post({'command': 'device_rename', 'device': B, 'name': 'x' * 65}, expected=400)
            post(host, B, 'device_rename', name='')
            require(inspect(host, B)['device_name'] == 'Stream Deck Mini', 'Reset did not restore model name')
            post(host, B, 'device_rename', name='Studio 🎛')
            report['device_names_and_change_events'] = True
            a_profile, a_page = initial_a['library']['active_profile'], initial_a['library']['active_page']
            b_profile, b_page = initial_b['library']['active_profile'], initial_b['library']['active_page']
            sdk.send()
            recorded = rows(proof)
            for device, context, coordinates in [(A, a_probe, {'column': 1, 'row': 1}), (B, b_probe, {'column': 2, 'row': 1})]:
                appear = next(e for e in recorded if e.get('event') == 'willAppear' and e.get('context') == context)
                require(appear['device'] == device and appear['payload']['coordinates'] == coordinates, 'Wrong device/geometry in action appearance')
            connected_b = next(i for i, e in enumerate(recorded) if e.get('event') == 'deviceDidConnect' and e.get('device') == B)
            appeared_b = next(i for i, e in enumerate(recorded) if e.get('event') == 'willAppear' and e.get('device') == B)
            require(connected_b < appeared_b, 'Device B was not announced before its actions')
            require(recorded[connected_b]['deviceInfo']['type'] == 1 and recorded[connected_b]['deviceInfo']['size'] == {'columns': 3, 'rows': 2}, 'B connection metadata used another model')

            # Editor selection changes neither SDK visibility nor plugin processes.
            begin = len(recorded)
            for device in (B, A, B):
                selected(host, device)
            sdk.send()
            require(not lifecycle(rows(proof)[begin:]), 'Editor selection emitted action lifecycle')
            press(host, A, 0)
            check_counter(host, A, 3)
            press(host, B, 0)
            check_counter(host, B, 27)
            require(host.state()['device'] == B and pid_map(host) == pids, 'Background input changed editor/process selection')
            report['counter_two_devices_one_process'] = True

            resource = root/'probe-resource.bin'
            resource.write_bytes(b'test-owned portable resource')
            b_before = at(inspect(host, B), 5)
            color = (11, 29, 233)
            sdk.send({'event': 'setSettings', 'context': a_probe, 'payload': {'owner': 'A', 'sdk': 41}},
                     {'event': 'setResources', 'context': a_probe, 'payload': {'file': str(resource)}},
                     {'event': 'setTitle', 'context': a_probe, 'payload': {'title': 'Owner A'}},
                     {'event': 'setImage', 'context': a_probe, 'payload': {'image': 'data:image/png;base64,'+base64.b64encode(solid_png(*color)).decode()}})
            changed = at(inspect(host, A), 5)
            require(changed['settings']['sdk'] == 41 and changed['title'] == 'Owner A', 'SDK updates missed parked A')
            require(changed['resources']['file'] == str(resource), 'Resource update missed A')
            require(png_corner(changed['preview'])[0] == color, 'Raster update missed A rendered pixels')
            unchanged = at(inspect(host, B), 5)
            require((unchanged['settings'], unchanged['title'], unchanged['preview'], unchanged['resources']) == (b_before['settings'], b_before['title'], b_before['preview'], b_before['resources']), 'A updates changed B')
            for event in ('getSettings', 'getResources'):
                response = sdk.request(event, a_probe)
                require(response['device'] == A and response['context'] == a_probe and response['payload']['coordinates'] == {'column': 1, 'row': 1}, 'SDK response used selected B ownership')
            host.post({'command': 'settings', 'device': B, 'context': a_probe, 'settings': {'bad': True}}, expected=400)
            sdk.send({'event': 'setSettings', 'context': a_counter, 'payload': {'count': 999}})
            require(at(inspect(host, A), 0)['settings']['count'] == 3, 'Probe modified another plugin context')
            stale_settings = at(inspect(host, A), 0)['settings']
            press(host, A, 0)
            check_counter(host, A, 5)
            post(host, A, 'settings_patch', context=a_counter, base=stale_settings, settings={**stale_settings, 'incrementBy': 4})
            patched = at(inspect(host, A), 0)['settings']
            require(patched['count'] == 5 and patched['incrementBy'] == 4, 'Stale editor step change rolled back live Counter state')
            press(host, A, 0)
            check_counter(host, A, 9)
            require(host.state()['device'] == B, 'Background settings patch changed editor selection')
            report['atomic_background_settings'] = True
            sdk.send({'event': 'setGlobalSettings', 'context': '@registration', 'payload': {'shared': 71}})
            for context in (a_probe, b_probe):
                require(sdk.request('getGlobalSettings', context)['payload']['settings'] == {'shared': 71}, 'Global settings became device-local')
            report['sdk_owner_routing'] = True

            # Only A departs; B's physical hold and timed sequence remain owned by B.
            routine = assign(host, B, 2, ROUTINE)
            children = [{'UUID': ACTION, 'ActionID': 'before', 'Settings': {}, 'States': [{}]},
                        {'UUID': DELAY, 'ActionID': 'delay', 'Settings': {'delay': 500}, 'States': [{}]},
                        {'UUID': ACTION, 'ActionID': 'after', 'Settings': {}, 'States': [{}]}]
            post(host, B, 'multi_save', context=routine, raw={'Actions': [{'Actions': children}, {'Actions': []}]})
            macro = at(inspect(host, B), 2)
            first_step, last_step = macro['multi_children'][0][0]['context'], macro['multi_children'][0][2]['context']
            sdk.send()
            begin = len(rows(proof))
            for device in (A, B):
                post(host, device, 'input', event='keyDown', index=5)
            press(host, B, 2)
            wait(host, lambda: any(e.get('event') == 'keyDown' and e.get('context') == first_step for e in rows(proof)[begin:]), 'B sequence first step')
            post(host, A, 'page_create', name='A second page')
            a_second = inspect(host, A)['library']['active_page']
            sdk.send()
            events = rows(proof)[begin:]
            require(any(e.get('event') == 'keyUp' and e.get('context') == a_probe for e in events), 'A held control was not released on page change')
            require(any(e.get('event') == 'willDisappear' and e.get('context') == a_probe for e in events), 'A page did not disappear')
            require(not any((e.get('event') == 'keyUp' and e.get('context') == b_probe) or (e.get('event') == 'willDisappear' and e.get('device') == B) for e in events), 'A page change cancelled B')
            wait(host, lambda: any(e.get('event') == 'keyUp' and e.get('context') == last_step and e.get('device') == B for e in rows(proof)[begin:]), 'B sequence continues after A page change')
            post(host, A, 'input', event='keyUp', index=5)
            post(host, B, 'input', event='keyUp', index=5)
            sdk.send()
            for context in (a_probe, b_probe):
                require(sum(e.get('event') == 'keyUp' and e.get('context') == context for e in rows(proof)[begin:]) == 1, 'Held release duplicated or missing')
            require(inspect(host, B)['library']['active_page'] == b_page, 'A navigation moved B')
            require(sdk.request('getSettings', a_probe)['device'] == A, 'Hidden A context lost its owner')
            post(host, A, 'page_select', id=a_page)
            report['page_and_hold_isolation'] = True

            # Disconnect preserves visible contexts and other-device operation.
            sdk.send()
            begin = len(rows(proof))
            post(host, A, 'device_event', event='disconnected', reason='owned test')
            sdk.send()
            require(not lifecycle(rows(proof)[begin:]), 'Disconnect incorrectly removed visible actions')
            online = {d['id']: d['online'] for d in host.state()['devices']}
            require(not online[A] and online[B], 'A disconnect changed another device online state')
            require(any(e.get('event') == 'deviceDidDisconnect' and e.get('device') == A for e in rows(proof)[begin:]), 'Wrong disconnect owner')
            press(host, B, 0)
            check_counter(host, B, 34)
            post(host, A, 'device_event', event='connected', serial=A, firmware='fixture', model='plus')
            sdk.send()
            require(not lifecycle(rows(proof)[begin:]), 'Same-identity reconnect recreated action contexts')
            require(at(inspect(host, A), 5)['context'] == a_probe and pid_map(host) == pids, 'Reconnect replaced actions or plugin process')
            report['disconnect_isolation'] = True

            # The command originates in one plugin but explicitly targets B/C.
            selected(host, A)
            before_a = inspect(host, A)['library']['active_profile']
            sdk.switch(B, 'mini-bundle')
            bundled_b = inspect(host, B)
            b_bundle_profile = bundled_b['library']['active_profile']
            b_bundle = at(bundled_b, 0)['context']
            require(b_bundle_profile != b_profile and at(bundled_b, 0)['uuid'] == ACTION, 'SDK failed to switch target B')
            require(host.state()['device'] == A and inspect(host, A)['library']['active_profile'] == before_a, 'Target-B switch changed A/editor')
            host.post({'command': 'device_add', 'device': C, 'model': 'mini'})
            sdk.switch(C, 'mini-bundle')
            bundled_c = inspect(host, C)
            require(bundled_c['library']['active_profile'] != b_bundle_profile and at(bundled_c, 0)['context'] != b_bundle, 'Same-model devices share a bundle instance')
            sdk.send({'event': 'setSettings', 'context': b_bundle, 'payload': {'bundle': 'B changed'}})
            require(at(inspect(host, C), 0)['settings'] == {'bundle': 'initial'}, 'Bundle settings leaked to C')
            sdk.switch(B)
            returned_b = inspect(host, B)
            require(returned_b['library']['active_profile'] == b_profile and returned_b['library']['active_page'] == b_page and at(returned_b, 0)['context'] == b_counter, 'Profile return used another device history')
            for device, payload in [(B, {'profile': 'undeclared'}), (B, {'profile': 'mini-bundle', 'page': 99}), (A, {'profile': 'mini-bundle'}), ('unknown-device', {'profile': 'mini-bundle'})]:
                before = [(d, inspect(host, d)['library']['active_profile']) for d in (A, B, C)]
                sdk.send({'event': 'switchToProfile', 'context': '@registration', 'device': device, 'payload': payload})
                require(before == [(d, inspect(host, d)['library']['active_profile']) for d in (A, B, C)], 'Invalid SDK switch changed a device')
            require(pid_map(host) == pids, 'Bundled profiles launched duplicate plugins')
            report['targeted_bundles'] = True

            if not args.skip_profile_copy:
                before = {p['id'] for p in inspect(host, C)['library']['profiles']}
                editor_before = host.state()['device']
                sdk.send()
                copy_begin = len(rows(proof))
                post(host, B, 'profile_copy_to_device', profile=b_profile, target_device=C)
                sdk.send()
                require(host.state()['device'] == editor_before, 'Profile copy changed editor selection')
                require(not any(e.get('device') in (A, B) for e in lifecycle(rows(proof)[copy_begin:])), 'Destination profile copy changed source/other lifecycle')
                require(inspect(host, B)['library']['active_profile'] == b_profile and inspect(host, B)['library']['active_page'] == b_page, 'Profile copy navigated source B')
                copied = inspect(host, C)
                copies = [p for p in copied['library']['profiles'] if p['id'] not in before and p['device'] == C]
                require(len(copies) == 1, 'Cross-device profile copy missing or ambiguous')
                require(copied['library']['active_profile'] == copies[0]['id'], 'Profile copy did not activate the destination copy')
                require(at(copied, 0)['context'] not in (a_counter, b_counter), 'Profile copy reused an action context')
                def contexts(slots):
                    return {slot['context'] for slot in slots} | set().union(*(contexts(group) for slot in slots for group in slot.get('multi_children', [])))
                require(contexts(copied['slots']).isdisjoint(contexts(inspect(host, B)['slots'])), 'Profile copy reused nested runtime contexts')
                require(at(copied, 0)['settings']['count'] == 34, 'Profile copy lost live settings')
                press(host, C, 0)
                check_counter(host, C, 41)
                require(at(inspect(host, B), 0)['settings']['count'] == 34, 'Copied Counter changed source B')
                require(pid_map(host) == pids, 'Profile copy launched another plugin process')

            # Persist distinct pages and restore editor B without a --model flag.
            post(host, A, 'page_select', id=a_second)
            a_second_counter = assign(host, A, 0, COUNTER, {'count': 100, 'incrementBy': 3})
            press(host, A, 0)
            check_counter(host, A, 103)
            selected(host, B)
            sdk.send()
            final = {d: inspect(host, d) for d in (A, B, C)}
            # Last disk mutation belongs to hidden A while editor B stays selected.
            # Do not device_select again before restart: that could conceal a
            # persistence bug which saves the temporary execution scope as editor.
            sdk.send({'event': 'setSettings', 'context': a_probe, 'payload': {'owner': 'A', 'sdk': 41, 'late': True}})
            require(host.state()['device'] == B, 'Late background mutation changed selected B')
            host.stop()
            require(host.process.returncode == 0, 'First host failed to shut down cleanly')
            # A newly declared bundle at startup must reach every matching
            # restored device, including C while the editor remains on B.
            # Use a distinct name so the manually installed mini-bundle above
            # cannot conceal a missing per-device AutoInstall pass.
            restart_bundle = 'restart-mini-bundle'
            probe_manifest = json.loads((probe/'manifest.json').read_text())
            probe_manifest['Profiles'].append({'Name': restart_bundle, 'DeviceType': 1,
                                              'AutoInstall': True, 'Readonly': False,
                                              'DontAutoSwitchWhenInstalled': True})
            (probe/'manifest.json').write_text(json.dumps(probe_manifest))
            with zipfile.ZipFile(probe/'mini-bundle.streamDeckProfile') as archive:
                bundled = json.loads(archive.read('manifest.json'))
            bundled['Name'] = 'Startup Mini bundle'
            with zipfile.ZipFile(probe/(restart_bundle+'.streamDeckProfile'), 'w') as archive:
                archive.writestr('manifest.json', json.dumps(bundled))
            launch_offset = len(rows(proof))
            host = Host(binary, [counter, probe], state_dir, root/'restart.log')
            host.ready()
            host.wait(lambda s: len(s['plugins']) == 2 and all(p['connected'] for p in s['plugins']), 'one Counter/probe reconnect after host restart')
            sdk = SDK(host, proof, control)
            sdk.send()
            require(host.state()['device'] == B, 'Restart lost selected editor device B')
            launches = [e for e in rows(proof)[launch_offset:] if e['event'] == 'fixtureLaunch']
            require(len(launches) == 1, 'Restart spawned duplicate probe processes')
            require({d['id'] for d in launches[0]['info']['devices']} == {A, B, C}, 'Restart registration omitted an associated device')
            metadata = {d['id']: d for d in launches[0]['info']['devices']}
            require(metadata[A]['name'] == 'Editing desk' and metadata[B]['name'] == 'Studio 🎛', 'Restart registration lost custom device names')
            for device, columns, rows_count, kind in [(A, 4, 2, 7), (B, 3, 2, 1), (C, 3, 2, 1)]:
                require(metadata[device]['type'] == kind and metadata[device]['size'] == {'columns': columns, 'rows': rows_count}, 'Restart metadata used selected-device geometry for every device')
            reconnects = [e for e in rows(proof)[launch_offset:] if e.get('event') == 'deviceDidConnect']
            require(all(e['deviceInfo']['name'] == metadata[e['device']]['name'] for e in reconnects), 'Reconnect metadata lost a custom name')
            require(sorted(e['device'] for e in reconnects) == sorted([A, B, C]), 'Restart must announce each online device once')
            for device in (A, B, C):
                restored = inspect(host, device)
                require(restored['library']['active_profile'] == final[device]['library']['active_profile'] and restored['library']['active_page'] == final[device]['library']['active_page'], f'{device} navigation lost on restart')
                require([s['context'] for s in restored['slots']] == [s['context'] for s in final[device]['slots']], f'{device} contexts changed on restart')
            bundle_ids = {}
            for device in (A, B, C):
                matching = [p for p in inspect(host, device)['library']['profiles'] if p['name'] == 'Startup Mini bundle']
                require(len(matching) == (0 if device == A else 1), f'Startup AutoInstall ignored {device} or installed the wrong geometry')
                if matching:
                    bundle_ids[device] = matching[0]['id']
            require(bundle_ids[B] != bundle_ids[C], 'Startup AutoInstall reused one profile across devices')
            saved = json.loads((state_dir/'session.json').read_text())
            for record in saved['devices']:
                if record['device'] in bundle_ids:
                    require(record['navigation']['bundles'][PLUGIN][restart_bundle] == bundle_ids[record['device']], 'Startup bundle binding belongs to another device')
            report['startup_bundle_all_devices'] = True
            require(at(inspect(host, A), 0)['context'] == a_second_counter, 'A second page lost its Counter')
            check_counter(host, A, 103)
            check_counter(host, B, 34)
            require(sdk.request('getGlobalSettings', '@registration')['payload']['settings'] == {'shared': 71}, 'Global settings did not persist')
            post(host, A, 'page_select', id=a_page)
            restored_probe = at(inspect(host, A), 5)
            require(restored_probe['context'] == a_probe and restored_probe['resources']['file'] == str(resource), 'Hidden A resources/context lost on restart')
            require(restored_probe['settings'].get('late') is True, 'Last background mutation was lost on restart')
            require(restored_probe['title'] == 'Owner A' and png_corner(restored_probe['preview'])[0] == color, 'Hidden A SDK appearance cache lost on restart')
            require(host.state()['device'] == B, 'Background page restore changed selected B')
            report['persistence_and_registration'] = True
            report['plugin_processes_per_host'] = 2  # One official Counter, one generated probe.
        finally:
            host.stop()
            require(host.process.returncode == 0, 'Host did not shut down cleanly')
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: one unchanged Counter process across devices; SDK ownership, lifecycle/holds, profile targeting/bundles, globals and restart; simulation only.' + (' PARTIAL: profile-copy acceptance skipped.' if args.skip_profile_copy else ' Cross-device profile copy passed.'))


if __name__ == '__main__':
    main()
