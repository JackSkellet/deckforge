#!/usr/bin/env python3
"""Forget devices through the real host, with isolated profiles and loopback plugins."""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import time

from multi_device_interop import (ACTION, PLUGIN, ROUTINE, DELAY, SDK, assign, at,
                                 create_probe, inspect, pid_map, post, press, rows, selected)
from sample_interop import require
from system_interop import Host


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    counter = project.parent/'evidence/official-samples/counter/com.elgato.counter.sdPlugin'
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'boundary': 'Temporary simulated session, generated inert SDK probe, no USB or OS actions.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-forget-') as directory:
        root = Path(directory)
        probe = root/(PLUGIN+'.sdPlugin')
        proof, control, report['probe_sha256'] = create_probe(probe, counter)
        state = root/'state'
        host = Host(binary, [probe], state, root/'host.log')
        try:
            host.ready()
            host.wait(lambda s: s['plugins'][0]['connected'], 'probe registration')
            a, b = host.state()['device'], 'forget-mini-b'
            host.post({'command': 'device_add', 'device': b, 'model': 'mini'})
            post(host, b, 'profile_create', name='To forget')
            a_slot = assign(host, a, 0, ACTION)
            b_slot = assign(host, b, 0, ACTION)
            routine = assign(host, b, 2, ROUTINE)
            children = [{'UUID': ACTION, 'ActionID': 'before', 'Settings': {}, 'States': [{}]},
                        {'UUID': DELAY, 'ActionID': 'delay', 'Settings': {'delay': 1500}, 'States': [{}]},
                        {'UUID': ACTION, 'ActionID': 'after', 'Settings': {}, 'States': [{}]}]
            post(host, b, 'multi_save', context=routine, raw={'Actions': [{'Actions': children}, {'Actions': []}]})
            last = at(inspect(host, b), 2)['multi_children'][0][2]['context']
            sdk = SDK(host, proof, control)
            host.post({'command': 'device_forget', 'device': b, 'confirm': True}, expected=400)
            press(host, b, 2)
            post(host, b, 'device_event', event='disconnected', reason='fixture')
            host.post({'command': 'device_forget', 'device': b}, expected=400)
            post(host, b, 'slot_cut', context=b_slot)
            selected(host, a)
            post(host, a, 'input', event='keyDown', index=0)
            sdk.send()
            offset = len(rows(proof))
            require(not any(e.get('event') == 'keyDown' and e.get('context') == last for e in rows(proof)), 'Fixture delay already elapsed before removal')
            pids = pid_map(host)
            before_a = host.state()
            host.post({'command': 'device_forget', 'device': b, 'confirm': True})
            sdk.send({'event': 'setSettings', 'context': b_slot, 'payload': {'late': True}})
            time.sleep(1.65)
            sdk.send()
            events = rows(proof)[offset:]
            require(not any(e.get('event') == 'keyDown' and e.get('context') == last for e in events), 'Forgotten device kept a pending Multi Action')
            require(any(e.get('event') == 'willDisappear' and e.get('context') == b_slot for e in events), 'Forgotten actions stayed in plugin lifecycle')
            require(not any(e.get('device') == a and e.get('event') in ('willDisappear', 'keyUp') for e in events), 'Another device lost its held action')
            post(host, a, 'input', event='keyUp', index=0)
            sdk.send()
            require(any(e.get('event') == 'keyUp' and e.get('context') == a_slot for e in rows(proof)[offset:]), 'Other held control did not release normally')
            after = host.state()
            require(after['device'] == a and pid_map(host) == pids, 'Forgetting a background device changed selection or plugin process')
            require(after['library'] == before_a['library'] and after['slots'] == before_a['slots'], 'Other device profiles or actions changed')
            require(after['clipboard'] is None and len(after['devices']) == 1, 'Forgotten device retained its clipboard or association')
            saved = json.loads((state/'session.json').read_text())
            require(all(p['device'] != b for p in saved['library']['profiles']), 'Forgotten profiles remain saved')
            backups = list((state/'forgotten-devices').glob('*.json'))
            require(len(backups) == 1 and backups[0].stat().st_mode & 0o777 == 0o600, 'Missing private recovery session')
            backup = json.loads(backups[0].read_text())
            require(any(p['device'] == b for p in backup['library']['profiles']), 'Recovery session lost forgotten profiles')
            post(host, a, 'device_event', event='disconnected', reason='fixture')
            host.post({'command': 'device_forget', 'device': a, 'confirm': True})
            empty = host.state()
            require(empty['device'].startswith('offline-streamdeck-plus-') and not empty['slots'], 'Last removal did not create an empty workspace')
            require(len(empty['devices']) == 1 and not empty['device_online'], 'Empty workspace incorrectly reports a device connection')
            host.stop()
            offset = len(rows(proof))
            host = Host(binary, [probe], state, root/'restart.log')
            host.ready()
            host.wait(lambda s: s['plugins'][0]['connected'], 'restart registration')
            sdk = SDK(host, proof, control)
            sdk.send()
            launched = next(e for e in rows(proof)[offset:] if e['event'] == 'fixtureLaunch')
            require([d['id'] for d in launched['info']['devices']] == [empty['device']], 'Restart registration resurrected forgotten devices')
            require(not host.state()['slots'], 'Restart repopulated the empty workspace')
            report.update(background_isolation=True, held_control_preserved=True,
                          pending_multi_cancelled=True, late_plugin_reply_ignored=True,
                          private_backups=True, last_device_empty_workspace=True,
                          forgotten_devices_absent_after_restart=True)
            host.stop()
            recovered = root/'recovered'
            recovered.mkdir()
            (recovered/'session.json').write_bytes(backups[0].read_bytes())
            host = Host(binary, [probe], recovered, root/'recovery.log')
            host.ready()
            host.wait(lambda s: s['plugins'][0]['connected'], 'recovery registration')
            require({d['id'] for d in host.state()['devices']} == {a, b}, 'Recovery session did not restore both devices')
            require(at(inspect(host, a), 0)['context'] == a_slot and at(inspect(host, b), 0)['context'] == b_slot, 'Recovery session lost original action identities')
            report['recovery_session_restored'] = True
        finally:
            host.stop()
            require(host.process.returncode == 0, host.log.read_text())
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: device removal, lifecycle, pending action cancellation, independent held controls, backup and restart; simulation only.')


if __name__ == '__main__':
    main()
