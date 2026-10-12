#!/usr/bin/env python3
"""Device lifecycle through unchanged Elgato SDK 2.0.4; no attached USB access.

Generated fixture uses the official Counter installation without editing it.
Simulation exercises injected link events. Native absent startup uses only an
explicit nonmatching serial, verified against read-only device enumeration.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time
import uuid
from urllib.request import urlopen

from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT, require
from system_interop import Host

PLUGIN = 'com.deckforge.device-probe'
ACTION = PLUGIN + '.key'
TRIGGER = 'com.elgato.streamdeck.keys.adaptor'
MULTI = 'com.elgato.streamdeck.multiactions.routine'
DELAY = 'com.elgato.streamdeck.multiactions.delay'
ABSENT_SERIAL = 'deckforge-absent-fixture'

JS = r'''import streamDeck from __SDK__;
import {connection} from __CONNECTION__;
import {appendFileSync} from 'node:fs';
const proof=__PROOF__;let seq=0,queue=Promise.resolve();const seen=new Set();
function snapshot(){const devices=[],actions=[];
 streamDeck.devices.forEach(d=>devices.push({id:d.id,connected:d.isConnected}));
 streamDeck.actions.forEach(a=>actions.push({context:a.id,device:a.device.id}));
 return {devices,actions,pid:process.pid};}
const record=(kind,fields={})=>appendFileSync(proof,JSON.stringify({seq:++seq,kind,...fields,...snapshot()})+'\n');
for(const event of ['deviceDidConnect','deviceDidDisconnect','willAppear','willDisappear','keyDown','keyUp'])
 connection.on(event,m=>record(event,{device:m.device,context:m.context,payload:m.payload}));
connection.on('didReceiveSettings',m=>{const q=m.payload.settings?.deviceProbe;
 if(!q?.nonce||seen.has(q.nonce))return;seen.add(q.nonce);
 queue=queue.then(async()=>{const a=streamDeck.actions.getActionById(m.context);
  if(!a)throw Error('Action missing from SDK store '+m.context);
  if(q.resources!==undefined)await a.setResources(q.resources);
  record('query',{nonce:q.nonce,context:a.id,resources:await a.getResources(),settings:await a.getSettings()});
 }).catch(error=>record('failure',{message:String(error),stack:error.stack}));
});
process.on('unhandledRejection',error=>record('failure',{message:String(error)}));
await streamDeck.connect();record('registered');
'''


class LifecycleHost(Host):
    def __init__(self, binary, plugins, state, log, *, simulate=True):
        self.log, self.output = log, log.open('w')
        self.simulate = simulate
        flags = ['--simulate'] if simulate else ['--serial', ABSENT_SERIAL]
        self.process = subprocess.Popen(
            [str(binary), 'serve', *flags, '--port', '0', '--state', str(state),
             *[x for plugin in plugins for x in ('--plugin', str(plugin))]],
            stdout=self.output, stderr=subprocess.STDOUT, start_new_session=True)
        self.base = self.token = None

    def state(self):
        self.check_process()
        with urlopen(self.base + '/api/state', timeout=3) as response:
            value = json.load(response)
        require(value['simulate'] is self.simulate, 'Host mode changed')
        require(value['connected'] is False, 'Fixture unexpectedly opened physical hardware')
        return value


def make_plugin(root, sdk):
    root.mkdir()
    proof = root / 'proof.jsonl'
    (root / 'plugin.mjs').write_text(JS
        .replace('__SDK__', json.dumps((sdk / 'dist/plugin/index.js').as_uri()))
        .replace('__CONNECTION__', json.dumps((sdk / 'dist/plugin/connection.js').as_uri()))
        .replace('__PROOF__', json.dumps(str(proof))))
    (root / 'manifest.json').write_text(json.dumps({
        'UUID': PLUGIN, 'Name': 'Device lifecycle probe', 'Version': '1.0.0',
        'SDKVersion': 2, 'Software': {'MinimumVersion': '7.1'},
        'CodePath': 'plugin.mjs', 'Nodejs': {'Version': '20'},
        'Actions': [{'UUID': ACTION, 'Name': 'Device probe', 'Controllers': ['Keypad'],
                     'States': [{}], 'SupportedInMultiActions': True}]}))
    return root, proof


def rows(proof):
    if not proof.exists():
        return []
    return [json.loads(line) for line in proof.read_text().splitlines(keepends=True)
            if line.endswith('\n')]


def wait(host, proof, predicate, description, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        host.check_process()
        require(not [r for r in rows(proof) if r['kind'] == 'failure'],
                'SDK fixture failure: ' + repr(rows(proof)[-5:]))
        result = predicate()
        if result:
            return result
        time.sleep(.01)
    raise AssertionError(description + '\n' + host.log.read_text()[-4000:]
                         + '\nSDK: ' + repr(rows(proof)[-12:]))


def key(host, index=0):
    return next(s for s in host.state()['slots']
                if s['controller'] == 'Keypad' and s['index'] == index)


def query(host, proof, context, resources=None):
    target = next(s for s in host.state()['slots'] if s['context'] == context)
    settings = copy.deepcopy(target['settings'])
    nonce = uuid.uuid4().hex
    settings['deviceProbe'] = {'nonce': nonce}
    if resources is not None:
        settings['deviceProbe']['resources'] = resources
    host.post({'command': 'settings', 'context': context, 'settings': settings})
    return wait(host, proof, lambda: next((r for r in rows(proof)
                if r['kind'] == 'query' and r['nonce'] == nonce), None), 'SDK query barrier')


def inject(host, event, **fields):
    host.post({'command': 'device_event', 'event': event, **fields})


def inputs(host, event, index=0, **fields):
    host.post({'command': 'input', 'event': event, 'index': index, **fields})


def child(action=ACTION, **settings):
    return {'UUID': action, 'ActionID': str(uuid.uuid4()), 'Settings': settings,
            'States': [{}], 'MultiActionDelays': {'Delay1': 100, 'Delay2': 100}}


def stable(host):
    return [(s['context'], s['settings'], s['resources']) for s in host.state()['slots']]


def lifecycle(proof, start):
    return [r for r in rows(proof)[start:] if r['kind'] in ('willAppear', 'willDisappear')]


def simulated(binary, root, plugin, proof):
    host = LifecycleHost(binary, [plugin], root / 'simulation', root / 'simulation.log')
    try:
        host.ready()
        host.wait(lambda s: all(p['connected'] for p in s['plugins']), 'SDK plugin registration')
        context = key(host)['context']
        media = root / 'resource.txt'; media.write_text('device lifecycle resource')
        before_query = query(host, proof, context, {'file': str(media)})
        pid, device = before_query['pid'], host.state()['device']
        require(host.state()['device_online'] is True, 'Simulated device must initially be online')
        before, start = stable(host), len(rows(proof))
        inject(host, 'disconnected', reason='test disconnect')
        down = wait(host, proof, lambda: next((r for r in rows(proof)[start:]
                    if r['kind'] == 'deviceDidDisconnect'), None), 'SDK disconnect')
        require(any(d['id'] == device and not d['connected'] for d in down['devices']),
                'SDK Device.isConnected did not become false')
        require(any(a['context'] == context for a in down['actions']), 'Disconnect erased visible SDK action')
        require(host.state()['device_online'] is False and host.state()['connection_error'],
                'Offline state/error not reported')
        require(stable(host) == before, 'Disconnect changed contexts/settings/resources')
        inject(host, 'connected', serial=device, firmware='test')
        up = wait(host, proof, lambda: next((r for r in rows(proof)[start:]
                  if r['kind'] == 'deviceDidConnect'), None), 'SDK reconnect')
        require(any(d['id'] == device and d['connected'] for d in up['devices']),
                'SDK Device.isConnected did not recover')
        require(stable(host) == before and not lifecycle(proof, start), 'Same-device reconnect churned actions')
        require(up['pid'] == pid and host.state()['connection_error'] is None, 'Plugin restarted or error remained')
        require(query(host, proof, context)['resources'] == {'file': str(media)}, 'SDK resource binding lost')

        # Physical hold cancellation is balanced once, with no late release after reconnect.
        start = len(rows(proof)); inputs(host, 'keyDown')
        wait(host, proof, lambda: any(r['kind'] == 'keyDown' for r in rows(proof)[start:]), 'held key')
        inject(host, 'disconnected', reason='held key')
        wait(host, proof, lambda: any(r['kind'] == 'keyUp' for r in rows(proof)[start:]), 'cancelled held key release')
        inject(host, 'connected', serial=device, firmware='test')
        inputs(host, 'keyUp'); query(host, proof, context)
        gestures = [r['kind'] for r in rows(proof)[start:] if r['kind'] in ('keyDown', 'keyUp')]
        require(gestures == ['keyDown', 'keyUp'], 'Held key produced stuck or phantom release')

        # A completed click already started timed work; USB loss alone does not hide it.
        host.post({'command': 'assign', 'controller': 'Keypad', 'index': 1, 'uuid': MULTI})
        parent = key(host, 1)
        host.post({'command': 'multi_save', 'context': parent['context'], 'raw': {
            'Actions': [{'Actions': [child(), child(DELAY, delay=350), child(tag='must-not-run')]}, {'Actions': []}]}})
        parent = key(host, 1); first, last = [parent['multi_children'][0][i]['context'] for i in (0, 2)]
        start = len(rows(proof)); inputs(host, 'keyDown', 1); inputs(host, 'keyUp', 1)
        wait(host, proof, lambda: any(r['kind'] == 'keyDown' and r['context'] == first
                                    for r in rows(proof)[start:]), 'Multi child hold')
        inject(host, 'disconnected', reason='multi remains visible')
        inject(host, 'connected', serial=device, firmware='test')
        host.wait(lambda s: not s['multi_runs'] and not s['multi_pending'], 'Visible Multi completion')
        query(host, proof, context)
        events = rows(proof)[start:]
        require(any(r['kind'] == 'keyDown' and r.get('context') == last for r in events), 'USB loss cancelled visible timed work')
        require([r['kind'] for r in events if r.get('context') == first and r['kind'] in ('keyDown', 'keyUp')]
                == ['keyDown', 'keyUp'], 'Multi hold not balanced')

        # Timed Trigger pulses likewise continue independently of physical holds.
        host.post({'command': 'assign', 'controller': 'Encoder', 'index': 0, 'uuid': TRIGGER})
        parent = next(s for s in host.state()['slots'] if s['uuid'] == TRIGGER)
        host.post({'command': 'trigger_save', 'context': parent['context'],
                   'raw': {'Actions': [child(), child(), child()]}})
        parent = next(s for s in host.state()['slots'] if s['uuid'] == TRIGGER)
        right = parent['trigger_children'][2]['context']; start = len(rows(proof))
        inputs(host, 'dialRotate', ticks=3)
        wait(host, proof, lambda: any(r['kind'] == 'keyDown' and r.get('context') == right
                                    for r in rows(proof)[start:]), 'Trigger pulse')
        inject(host, 'disconnected', reason='trigger remains visible')
        inject(host, 'connected', serial=device, firmware='test')
        time.sleep(.3); query(host, proof, context)
        require([r['kind'] for r in rows(proof)[start:] if r.get('context') == right
                 and r['kind'] in ('keyDown', 'keyUp')] == ['keyDown', 'keyUp'] * 3, 'Visible Trigger queue lost or duplicated pulses')

        # A physical dial press uses DIRECT ownership, unlike those timed pulses.
        center = parent['trigger_children'][1]['context']; start = len(rows(proof))
        inputs(host, 'dialDown')
        wait(host, proof, lambda: any(r['kind'] == 'keyDown' and r.get('context') == center
                                    for r in rows(proof)[start:]), 'held dial center')
        inject(host, 'disconnected', reason='held dial')
        wait(host, proof, lambda: any(r['kind'] == 'keyUp' and r.get('context') == center
                                    for r in rows(proof)[start:]), 'cancelled direct dial hold')
        inject(host, 'connected', serial=device, firmware='test')
        inputs(host, 'dialUp'); query(host, proof, context)
        require([r['kind'] for r in rows(proof)[start:] if r.get('context') == center
                 and r['kind'] in ('keyDown', 'keyUp')] == ['keyDown', 'keyUp'], 'Dial hold produced phantom release')

        # Identity change must retire old SDK actions before announcing their replacement.
        old_sdk = query(host, proof, context)
        old = copy.deepcopy(host.state()); start = len(rows(proof))
        inputs(host, 'keyDown', 1); inputs(host, 'keyUp', 1)
        wait(host, proof, lambda: any(r['kind'] == 'keyDown' and r.get('context') == first
                                    for r in rows(proof)[start:]), 'Multi before identity change')
        inputs(host, 'dialRotate', ticks=3)
        wait(host, proof, lambda: any(r['kind'] == 'keyDown' and r.get('context') == right
                                    for r in rows(proof)[start:]), 'Trigger before identity change')
        replacement = 'deckforge-other-fixture'
        inject(host, 'connected', serial=replacement, firmware='other-test')
        changed = query(host, proof, key(host)['context'])
        time.sleep(.55)
        new = host.state(); batch = rows(proof)[start:]
        require(not any(r['kind'] == 'keyDown' and r.get('context') == last for r in batch),
                'Old device Multi work continued after identity change')
        connect = next(i for i, r in enumerate(batch) if r['kind'] == 'deviceDidConnect' and r['device'] == replacement)
        appearances = [(i, r) for i, r in enumerate(batch) if r['kind'] == 'willAppear']
        require(appearances and all(i > connect and r['device'] == replacement for i, r in appearances),
                'New identity was not announced before its actions')
        old_contexts = {a['context'] for a in old_sdk['actions']}
        require(old_contexts <= {r['context'] for r in batch if r['kind'] == 'willDisappear'}, 'Old SDK contexts not retired')
        for retired in old_contexts:
            boundary = next(i for i, r in enumerate(batch) if r['kind'] == 'willDisappear' and r['context'] == retired)
            require(not any(r['kind'] in ('keyDown', 'keyUp') and r.get('context') == retired
                            for r in batch[boundary + 1:]), 'Retired device action received a late gesture')
        require(changed['pid'] == pid and all(a['device'] == replacement for a in changed['actions']),
                'SDK retained stale device references or plugin restarted')
        require([(s['uuid'], s['resources']) for s in old['slots']] ==
                [(s['uuid'], s['resources']) for s in new['slots']], 'Identity switch lost action/resource configuration')
        old_settings = copy.deepcopy(old['slots'][0]['settings']); new_settings = copy.deepcopy(new['slots'][0]['settings'])
        old_settings.pop('deviceProbe', None); new_settings.pop('deviceProbe', None)
        require(old_settings == new_settings, 'Identity switch lost settings')
        return {'plugin_pid': pid, 'sdk_events': len(rows(proof))}
    finally:
        host.stop()
        require(host.process.returncode == 0, 'Simulated host did not stop cleanly')


def absent(binary, root, plugin, proof):
    # This command only enumerates; explicit selection prevents opening another serial.
    devices = subprocess.check_output([str(binary), 'devices'], text=True, timeout=10)
    require(ABSENT_SERIAL not in devices, 'Absent fixture serial unexpectedly exists')
    require('unknown serial' not in devices, 'Native absent test cannot avoid read-only serial probes on this hardware; run the simulated cases separately')
    saved = None
    for attempt in range(2):
        start = len(rows(proof))
        host = LifecycleHost(binary, [plugin] if attempt == 0 else [], root / 'offline',
                             root / f'offline-{attempt}.log', simulate=False)
        try:
            host.ready(); host.wait(lambda s: all(p['connected'] for p in s['plugins']), 'offline SDK startup')
            context = key(host)['context']
            reply = query(host, proof, context)
            snapshot = host.state()
            require(snapshot['device_online'] is False and snapshot['connection_error'], 'Absent USB did not stay offline')
            require(not any(r['kind'] == 'deviceDidConnect' for r in rows(proof)[start:]), 'Absent USB generated fake connect')
            require(any(d['id'] == snapshot['device'] and not d['connected'] for d in reply['devices']),
                    'Offline registration lacks known disconnected SDK device')
            require(any(a['context'] == context for a in reply['actions']), 'Offline editor action missing from SDK')
            host.post({'command': 'device_event', 'event': 'connected', 'serial': 'forbidden'}, expected=400)
            if attempt == 0:
                settings = {'offlineSaved': 42}
                host.post({'command': 'settings', 'context': context, 'settings': settings})
                saved = context
                time.sleep(1.15)  # Host remains usable while automatic connection retry runs.
                require(key(host)['settings'] == settings and not host.state()['device_online'], 'Offline retry damaged edits')
            else:
                require(key(host)['settings']['offlineSaved'] == 42 and context == saved, 'Offline restart lost edits/context')
        finally:
            host.stop()
            require(host.process.returncode == 0, 'Offline host did not stop cleanly')
    return {'native_absent_launches': 2, 'selected_nonmatching_serial': ABSENT_SERIAL}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/deckforge'))
    args = parser.parse_args(); binary = args.binary.resolve()
    samples = Path(__file__).resolve().parents[2] / 'evidence/official-samples'
    sdk = samples / 'counter/node_modules/@elgato/streamdeck'
    require(json.loads((sdk / 'package.json').read_text())['version'] == '2.0.4', 'Unexpected SDK version')
    require(hashlib.sha256((samples / 'counter/package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256,
            'Official Counter lock changed')
    require(subprocess.check_output(['git', '-C', str(samples), 'rev-parse', 'HEAD'], text=True).strip() == SAMPLE_COMMIT,
            'Official sample pin changed')
    paths = [sdk / p for p in ['package.json', 'dist/plugin/devices/device.js', 'dist/plugin/devices/service.js',
                              'dist/plugin/actions/service.js', 'dist/plugin/actions/context.js']]
    hashes = {str(p.relative_to(sdk)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    with tempfile.TemporaryDirectory(prefix='deckforge-device-') as directory:
        root = Path(directory); plugin, proof = make_plugin(root / (PLUGIN + '.sdPlugin'), sdk)
        result = simulated(binary, root, plugin, proof)
        result.update(absent(binary, root, plugin, proof))
        require(all(hashlib.sha256(p.read_bytes()).hexdigest() == hashes[str(p.relative_to(sdk))] for p in paths),
                'Official SDK files changed')
        print('PASS:', json.dumps(result, sort_keys=True))
        print('Official SDK 2.0.4 hashes:', json.dumps(hashes, sort_keys=True))
        print('No attached USB device was opened, unplugged, reset or rebound.')


if __name__ == '__main__':
    main()
