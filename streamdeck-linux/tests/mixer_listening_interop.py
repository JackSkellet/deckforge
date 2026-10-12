#!/usr/bin/env python3
"""Measure listening switches on a private, hardware-free Pulse-compatible server."""
import argparse
import array
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import threading
import time

from private_audio import private_audio
from system_interop import Host


def pulse(*args):
    return subprocess.check_output(['pactl', *args], text=True)


def inventory(kind):
    return json.loads(pulse('--format=json', 'list', kind))


def stop(child):
    if child.poll() is None:
        child.terminate()
    try:
        child.wait(timeout=3)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=3)
    for pipe in (child.stdin, child.stdout, child.stderr):
        if pipe is not None and not pipe.closed:
            try:
                pipe.close()
            except (BrokenPipeError, OSError):
                pass


def capture(records, seconds):
    selector = selectors.DefaultSelector()
    buffers = {key: bytearray() for key in records}
    for key, child in records.items():
        os.set_blocking(child.stdout.fileno(), False)
        selector.register(child.stdout, selectors.EVENT_READ, key)
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        for entry, _ in selector.select(.02):
            data = os.read(entry.fileobj.fileno(), 32768)
            assert data, 'An owned recording stream stopped during the test'
            buffers[entry.data].extend(data)
            assert len(buffers[entry.data]) < 4 * 1024 * 1024
    selector.close()
    result = {}
    for key, data in buffers.items():
        samples = array.array('f')
        samples.frombytes(data[:len(data)//8*8])
        mono = list(samples)[::2]
        assert len(mono) > 8000, (key, len(mono))
        tail = mono[-12000:]
        def frequency(hz):
            re = sum(value*math.cos(2*math.pi*hz*n/48000) for n, value in enumerate(tail))
            im = sum(value*math.sin(2*math.pi*hz*n/48000) for n, value in enumerate(tail))
            return 2*math.hypot(re, im)/len(tail)
        windows = [math.sqrt(sum(value*value for value in mono[i:i+480])/480)
                   for i in range(0, len(mono)-480, 480)]
        result[key] = {'440hz': frequency(440), '880hz': frequency(880),
                       'rms': math.sqrt(sum(value*value for value in tail)/len(tail)),
                       'minimum_10ms_rms': min(windows)}
    return result


def run(args, root):
    binary = args.binary.resolve(strict=True)
    state = root / 'session'
    state.mkdir()
    prefix = 'deckforge.' + hashlib.sha256(os.fsencode(state.resolve())).hexdigest()[:24] + '.'
    children, feeders, records = [], [], {}
    host = None
    report = {'ok': False, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'boundary': 'Private PipeWire/Pulse server and generated stereo PCM; no hardware monitors, real apps, microphones or system defaults.'}
    outputs = {}
    for name in ['headphones', 'spare', 'fixed']:
        output = 'fixture_' + name
        outputs[name] = (output, int(pulse('load-module', 'module-null-sink', 'sink_name='+output+' rate=48000 channels=2')))
    pulse('set-default-sink', outputs['fixed'][0])
    pulse('set-default-source', outputs['fixed'][0] + '.monitor')
    before = json.loads(pulse('--format=json', 'info'))
    def original_output_controls():
        return {s['name']: (s['mute'], s['volume']) for s in inventory('sinks') if s['name'] in {v[0] for v in outputs.values()}}
    output_controls = original_output_controls()
    def record(key, source):
        assert source.startswith(prefix) or source in {v[0]+'.monitor' for v in outputs.values()}
        child = subprocess.Popen(['pacat', '--record', '--raw', '--format=float32le', '--rate=48000', '--channels=2', '--latency-msec=20', '--device='+source, '--client-name=Deckforge listening capture '+key], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        children.append(child)
        records[key] = child
    def play(channel, frequency):
        child = subprocess.Popen(['pacat', '--playback', '--raw', '--format=float32le', '--rate=48000', '--channels=2', '--latency-msec=20', '--device='+prefix+'channel.'+channel, '--client-name=Deckforge listening generator '+channel], stdin=subprocess.PIPE, stderr=subprocess.DEVNULL)
        children.append(child)
        def feed():
            phase = 0
            try:
                while child.poll() is None:
                    samples = array.array('f')
                    for _ in range(480):
                        value = .2*math.sin(2*math.pi*frequency*phase/48000)
                        samples.extend((value, value))
                        phase += 1
                    child.stdin.write(samples.tobytes())
                    child.stdin.flush()
            except (BrokenPipeError, ValueError):
                pass
        thread = threading.Thread(target=feed, daemon=True)
        thread.start()
        feeders.append(thread)
    def stable_nodes():
        return {kind: {item['name']: item['index'] for item in inventory(kind) if item['name'].startswith(prefix)} for kind in ['sinks', 'sources']}
    def monitoring(**values):
        host.post({'command': 'mixer_control', 'revision': host.state()['mixer']['revision'], 'target': {'kind': 'monitor'}, **values})
    def applied_monitor(mix, output):
        def check(state):
            mixer = state['mixer']
            assert not mixer['error'], mixer['error']
            return mixer['applied_config']['monitoring'] == {'mix': mix, 'output': output}
        return host.wait(check, 'listening selection accepted')
    try:
        host = Host(binary, [], state, root / 'host.log').ready()
        initial = host.state()['mixer']['config']
        config = copy.deepcopy(initial)
        config.update(enabled=True,
            channels=[{'id':'music','name':'Music','apps':[],'sources':[]}, {'id':'chat','name':'Chat','apps':[],'sources':[]}],
            routes=[{'channel':'music','mix':'monitor','volume':100,'muted':False}, {'channel':'chat','mix':'stream','volume':100,'muted':False}],
            monitoring={'output':outputs['headphones'][0], 'mix':'monitor'})
        config['mixes'][1]['outputs'] = [outputs['fixed'][0]]
        host.post({'command':'mixer_config', 'settings':config, 'expected':initial})
        host.wait(lambda s:s['mixer']['active'] and not s['mixer']['pending'], 'initial listening graph')
        play('music', 440)
        play('chat', 880)
        record('listening', outputs['headphones'][0]+'.monitor')
        record('spare', outputs['spare'][0]+'.monitor')
        exports = host.state()['mixer']['recording_sources']
        record('monitor', exports['monitor'])
        record('stream', exports['stream'])
        record('fixed', outputs['fixed'][0]+'.monitor')
        capture(records, .4)
        baseline = capture(records, .5)
        assert .17 < baseline['listening']['440hz'] < .23 and baseline['listening']['880hz'] < .005, baseline
        assert baseline['monitor']['440hz'] > .17 and baseline['monitor']['880hz'] < .005, baseline
        assert baseline['stream']['880hz'] > .17 and baseline['stream']['440hz'] < .005, baseline
        source_names = {source['index']: source['name'] for source in inventory('sources')}
        for key in ['monitor', 'stream']:
            capture_stream = next(item for item in inventory('source-outputs') if
                item.get('properties', {}).get('application.name') == 'Deckforge listening capture ' + key)
            assert source_names[capture_stream['source']] == exports[key], 'Recorder fell back to another input'
        nodes = stable_nodes()
        errors = []
        def switch():
            try:
                time.sleep(.15)
                monitoring(mix='stream')
            except Exception as error:
                errors.append(error)
        thread = threading.Thread(target=switch)
        thread.start()
        switching = capture(records, .8)
        thread.join(timeout=4)
        assert not thread.is_alive() and not errors, errors
        applied_monitor('stream', outputs['headphones'][0])
        assert stable_nodes() == nodes, 'Listening switch rebuilt a mix/source'
        assert all(switching[k]['minimum_10ms_rms'] > .08 for k in ['monitor','stream','fixed']), switching
        capture(records, .3)
        selected = capture(records, .4)
        assert selected['listening']['440hz'] < .005 and .17 < selected['listening']['880hz'] < .23, selected
        host.post({'command':'mixer_control','revision':host.state()['mixer']['revision'],
                   'target':{'kind':'route','channel':'music','mix':'monitor'},'volume':40,'fade_ms':1600})
        host.wait(lambda s:s['mixer']['fading'] and 45 < s['mixer']['applied_config']['routes'][0]['volume'] < 95, 'active route fade')
        monitoring(mix=None)
        applied_monitor(None, outputs['headphones'][0])
        assert host.state()['mixer']['fading'], 'Listening switch cancelled another fade'
        capture(records, .3)
        silent = capture(records, .4)
        assert silent['listening']['rms'] < .002 and silent['stream']['880hz'] > .17, silent
        monitoring(output=outputs['spare'][0], mix='stream')
        applied_monitor('stream', outputs['spare'][0])
        capture(records, .3)
        swapped = capture(records, .4)
        assert swapped['listening']['rms'] < .002 and swapped['spare']['880hz'] > .17, swapped
        host.wait(lambda s:not s['mixer']['fading'] and s['mixer']['applied_config']['routes'][0]['volume']==40, 'route fade completes after listening changes')
        assert stable_nodes() == nodes
        assert original_output_controls() == output_controls, 'Listening changed system output volume/mute'
        previous = host.state()['mixer']['config']
        host.post({'command':'mixer_control','revision':host.state()['mixer']['revision'],
                   'target':{'kind':'monitor'},'output':outputs['fixed'][0]}, expected=400)
        assert host.state()['mixer']['config'] == previous
        pulse('set-sink-mute', outputs['spare'][0], '1')
        host.wait(lambda s:any(d['name']==outputs['spare'][0] and d['muted'] for d in s['mixer']['sinks']), 'read-only output mute status')
        monitoring(mix='monitor')
        applied_monitor('monitor', outputs['spare'][0])
        assert next(s for s in inventory('sinks') if s['name']==outputs['spare'][0])['mute'], 'Monitoring unmuted a system output'
        stop(records.pop('spare'))
        pulse('unload-module', str(outputs['spare'][1]))
        host.wait(lambda s:any('Listening output unavailable' in w for w in s['mixer']['warnings']), 'missing listening device stays selected')
        missing = host.state()['mixer']
        assert missing['config']['monitoring']['output']==outputs['spare'][0]
        assert stable_nodes() == nodes
        remaining = capture(records, .4)
        assert remaining['stream']['880hz'] > .17 and remaining['fixed']['880hz'] > .17, remaining
        # Reappearance under a new server index must recover only the selected
        # named destination; no fallback destination was ever chosen.
        outputs['spare'] = (outputs['spare'][0], int(pulse('load-module', 'module-null-sink',
            'sink_name='+outputs['spare'][0]+' rate=48000 channels=2')))
        record('spare', outputs['spare'][0]+'.monitor')
        host.wait(lambda s:any(d['name']==outputs['spare'][0] for d in s['mixer']['sinks'])
            and not any('Listening output unavailable' in w for w in s['mixer']['warnings']), 'selected output reconnect')
        reconnect_muted = next(s for s in inventory('sinks') if s['name']==outputs['spare'][0])['mute']
        # The private server may restore its previous mute. Only the test/user,
        # never Deckforge, clears that system setting before measuring playback.
        pulse('set-sink-mute', outputs['spare'][0], '0')
        capture(records, .3)
        recovered = capture(records, .5)
        assert .065 < recovered['spare']['440hz'] < .095 and recovered['spare']['880hz'] < .005, recovered
        assert recovered['stream']['880hz'] > .17
        for child in children:
            stop(child)
        for feeder in feeders:
            feeder.join(timeout=1)
        latest = host.state()['mixer']['config']
        disabled = copy.deepcopy(latest)
        disabled['enabled'] = False
        host.post({'command':'mixer_config','settings':disabled,'expected':latest})
        host.wait(lambda s:not s['mixer']['active'] and not s['mixer']['error'], 'disabled graph cleanup')
        host.stop()
        host = None
        assert all(not values for values in stable_nodes().values())
        host = Host(binary, [], state, root / 'restart.log').ready()
        restored = host.state()['mixer']
        assert restored['config']['monitoring'] == disabled['monitoring']
        assert not restored['active'] and not restored['config']['enabled']
        assert all(not values for values in stable_nodes().values())
        host.stop()
        host = None
        after = json.loads(pulse('--format=json', 'info'))
        assert all(before[k]==after[k] for k in ['default_sink_name','default_source_name'])
        report.update(ok=True, baseline=baseline, during_switch=switching, selected=selected,
            no_mix=silent, switched_output=swapped, fade_preserved=True, nodes_preserved=True,
            duplicate_feed_rejected=True, system_output_controls_unchanged=True,
            muted_device_reported=True, missing_output_no_fallback=True, recording_inputs_verified=True, reconnect_initial_muted=reconnect_muted, reconnect=recovered, inactive_restart_preserves_selection=True, cleanup=True,
            default_devices_unchanged=True)
    except Exception as error:
        report['error'] = str(error)
        report['mixer'] = host.state()['mixer'] if host else None
        report['inventory'] = {kind: inventory(kind) for kind in ['sinks', 'sources', 'sink-inputs', 'source-outputs']}
        raise
    finally:
        for child in children:
            stop(child)
        if host:
            host.stop()
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/deckforge'))
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='deckforge-listening-') as directory:
        root = Path(directory)
        with private_audio(root) as env:
            previous = dict(os.environ)
            os.environ.update(env)
            try:
                run(args, root)
            finally:
                os.environ.clear()
                os.environ.update(previous)
    print('PASS: listening mix/output switching, continuous recordings, fades, No mix, missing device and cleanup.')


if __name__ == '__main__':
    main()
