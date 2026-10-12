#!/usr/bin/env python3
"""Native host mixer config, key/dial control and restart through owned virtual nodes.

No microphone, real app stream, physical output, or USB is used. Explicit opt-in
creates silent test-owned channels/mixes on the current Pulse-compatible server.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from system_interop import Host


def pulse(*args):
    return json.loads(subprocess.check_output(['pactl', '--format=json', *args], text=True))


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/release/deckforge')
    parser.add_argument('--run-virtual', action='store_true')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    if not args.run_virtual:
        parser.error('--run-virtual is required to create test-owned silent audio nodes')
    binary = args.binary.resolve(strict=True)
    before = pulse('info')
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'boundary': 'Real Rust host and audio server; temporary session and silent owned virtual nodes only.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-mixer-host-') as directory:
        root = Path(directory)
        os.environ['XDG_STATE_HOME'] = str(root / 'journal')
        state = root / 'state'
        state.mkdir()
        namespace = hashlib.sha256(os.fsencode(state.resolve())).hexdigest()[:24]
        prefix = 'deckforge.' + namespace + '.'
        def owned():
            return [d['name'] for d in pulse('list', 'sinks') if d['name'].startswith(prefix)]
        host = None
        try:
            host = Host(binary, [], state, root / 'host.log').ready()
            initial = host.state()['mixer']['config']
            assert not initial['enabled'] and not owned()
            host.post({'command': 'mixer_refresh'})
            host.wait(lambda s: s['mixer']['connected'], 'read-only audio inventory')
            assert not owned(), 'Opening mixer created audio routes'
            stale = copy.deepcopy(initial)
            stale['enabled'] = True
            host.post({'command': 'mixer_config', 'settings': initial, 'expected': stale}, expected=400)
            assert host.state()['mixer']['config'] == initial
            config = copy.deepcopy(initial)
            config.update(enabled=True, channels=[{'id': 'fixture', 'name': 'Fixture', 'apps': [], 'sources': []}],
                          routes=[{'channel': 'fixture', 'mix': 'monitor', 'volume': 70., 'muted': False},
                                  {'channel': 'fixture', 'mix': 'stream', 'volume': 30., 'muted': False}])
            host.post({'command': 'mixer_config', 'settings': config, 'expected': initial})
            def applied(s):
                m = s['mixer']
                assert not m['error'], m['error']
                return m['active'] and not m['pending']
            host.wait(applied, 'independent monitor and stream routes')
            assert set(owned()) == {prefix+'channel.fixture', prefix+'mix.monitor', prefix+'mix.stream'}
            def assign(controller, index, uuid, settings):
                host.post({'command': 'assign', 'controller': controller, 'index': index, 'uuid': uuid})
                slot = next(s for s in host.state()['slots'] if s['controller'] == controller and s['index'] == index)
                host.post({'command': 'settings', 'context': slot['context'], 'settings': settings})
                return slot['context']
            channel_context = assign('Encoder', 0, 'com.deckforge.mixer.channel',
                                     {'channel': 'fixture', 'mix': 'monitor', 'step': 5})
            host.post({'command': 'input', 'event': 'dialRotate', 'index': 0, 'ticks': 2})
            host.wait(applied, 'dial route gain applied')
            current = host.state()['mixer']['config']
            assert current['routes'][0]['volume'] == 80 and current['routes'][1]['volume'] == 30
            host.post({'command': 'input', 'event': 'dialDown', 'index': 0})
            host.post({'command': 'input', 'event': 'dialUp', 'index': 0})
            host.wait(applied, 'dial push mute applied')
            current = host.state()['mixer']['config']
            assert current['routes'][0]['muted'] and not current['routes'][1]['muted']
            assign('Keypad', 0, 'com.deckforge.mixer.mix',
                   {'mix': 'stream', 'mode': 'set_volume', 'volume': 55})
            host.post({'command': 'input', 'event': 'keyDown', 'index': 0})
            host.post({'command': 'input', 'event': 'keyUp', 'index': 0})
            host.wait(applied, 'key mix master gain applied')
            current = host.state()['mixer']['config']
            assert next(m for m in current['mixes'] if m['id'] == 'stream')['volume'] == 55
            host.wait(lambda s: next(x for x in s['slots'] if x['context'] == channel_context)['feedback'].get('value') not in ('--', None), 'dial feedback')
            sequence = assign('Keypad', 1, 'com.elgato.streamdeck.multiactions.routine', {})
            host.post({'command': 'multi_save', 'context': sequence, 'raw': {'Actions': [{'Actions': [
                {'UUID': 'com.deckforge.mixer.mix', 'ActionID': 'mix-child', 'Settings':
                 {'mix': 'stream', 'mode': 'set_volume', 'volume': 45}, 'States': [{}]}
            ]}, {'Actions': []}]}})
            host.post({'command': 'input', 'event': 'keyDown', 'index': 1})
            host.post({'command': 'input', 'event': 'keyUp', 'index': 1})
            host.wait(lambda s: next(m for m in s['mixer']['config']['mixes'] if m['id'] == 'stream')['volume'] == 45, 'mixer child in Multi Action')
            stack = assign('Encoder', 1, 'com.elgato.streamdeck.dial.stack', {})
            host.post({'command': 'stack_save', 'context': stack, 'raw': {
                'Settings': {'CurrentIdx': 0, 'FastNavigation': False, 'ShowStackIcon': True},
                'Actions': [{'UUID': 'com.deckforge.mixer.channel', 'ActionID': 'route-child',
                             'Settings': {'channel': 'fixture', 'mix': 'stream', 'step': 2}, 'States': [{}]}]}})
            host.post({'command': 'input', 'event': 'dialRotate', 'index': 1, 'ticks': 3})
            host.wait(applied, 'mixer child in Dial Stack')
            current = host.state()['mixer']['config']
            assert current['routes'][1]['volume'] == 36
            host.stop()
            host = None
            assert not owned(), 'Normal shutdown leaked owned routes'
            host = Host(binary, [], state, root / 'restart.log').ready()
            host.wait(applied, 'saved enabled graph resumed')
            assert host.state()['mixer']['config'] == current
            assert next(s for s in host.state()['slots'] if s['controller'] == 'Encoder')['context'] == channel_context
            disabled = copy.deepcopy(current)
            disabled['enabled'] = False
            host.post({'command': 'mixer_config', 'settings': disabled, 'expected': current})
            host.wait(lambda s: not s['mixer']['active'] and not s['mixer']['pending'], 'mixer disable')
            assert not owned()
            host.post({'command': 'input', 'event': 'dialRotate', 'index': 0, 'ticks': 1}, expected=400)
            assert host.state()['mixer']['config'] == disabled and not owned(), 'Dial enabled or changed disabled graph'
            report.update(read_only_inventory=True, stale_write_rejected=True,
                          separate_route_levels=True, simulated_key_and_dial_commands=True,
                          multi_action_and_dial_stack=True,
                          saved_graph_restarted=True, disabled_action_cannot_enable=True)
        finally:
            if host is not None:
                host.stop()
            until = time.monotonic() + 5
            while owned() and time.monotonic() < until:
                time.sleep(.05)
            assert not owned(), 'Owned test nodes remain'
        after = pulse('info')
        for key in ('default_sink_name', 'default_source_name'):
            assert before[key] == after[key], 'System default changed'
        report['default_devices_unchanged'] = True
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print('PASS: native host mixer inventory, independent routing, dial/key control, feedback, persistence/restart and cleanup; system defaults unchanged.')


if __name__ == '__main__':
    main()
