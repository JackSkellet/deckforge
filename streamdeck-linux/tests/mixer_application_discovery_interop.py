#!/usr/bin/env python3
"""Installed app selection routes a future real process through owned silent nodes.

The test builds its own tiny Pulse client, installs a temporary desktop entry and
runs an isolated simulated Deckforge host. It never launches a user's application,
plays through real speakers, captures microphones, or changes default devices.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import uuid

from system_interop import Host
from private_audio import private_audio

PLAYER = r'''
#include <pulse/simple.h>
#include <pulse/error.h>
#include <stdint.h>
#include <stdio.h>
int main(int argc, char **argv) {
    if (argc < 2 || argc > 3) return 2;
    pa_sample_spec spec = { PA_SAMPLE_S16LE, 48000, 2 };
    int error = 0;
    pa_simple *stream = pa_simple_new(NULL, argc == 3 ? argv[2] : "Deckforge discovery fixture", PA_STREAM_PLAYBACK,
        argv[1], "Owned silent discovery stream", &spec, NULL, NULL, &error);
    if (!stream) { fprintf(stderr, "%s\n", pa_strerror(error)); return 3; }
    int16_t samples[960] = {0};
    while (pa_simple_write(stream, samples, sizeof(samples), &error) >= 0) {}
    pa_simple_free(stream);
    return 0;
}
'''


def pulse(*args):
    return subprocess.check_output(['pactl', *args], text=True)


def info():
    return json.loads(pulse('--format=json', 'info'))


def modules():
    return {int(parts[0]): parts[1:3]
            for row in pulse('list', 'short', 'modules').splitlines()
            if len(parts := row.split('\t')) >= 3 and parts[0].isdigit()}


def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/deckforge'))
    parser.add_argument('--run-virtual', action='store_true')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    if not args.run_virtual:
        parser.error('--run-virtual is required for these test-owned silent audio nodes')
    with tempfile.TemporaryDirectory(prefix='deckforge-private-audio-') as directory:
        with private_audio(Path(directory)) as env:
            previous = dict(os.environ)
            os.environ.update(env)
            try:
                run(args)
            finally:
                os.environ.clear()
                os.environ.update(previous)


def run(args):
    binary = args.binary.resolve(strict=True)
    report = {'ok': False, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-app-discovery-') as directory:
        root = Path(directory)
        source, player = root / 'player.c', root / 'Player with spaces'
        source.write_text(PLAYER)
        flags = shlex.split(subprocess.check_output(['pkg-config', '--cflags', '--libs', 'libpulse-simple'], text=True))
        subprocess.run(['gcc', str(source), '-o', str(player), *flags], check=True)
        data = root / 'data'
        applications = data / 'applications'
        applications.mkdir(parents=True)
        launcher = root / 'Player launcher'
        launcher.write_text('#!/bin/sh\nexec "' + str(player) + '" "$@"\n')
        launcher.chmod(0o700)
        app_id = 'org.deckforge.DiscoveryFixture.desktop'
        (applications / app_id).write_text(
            '[Desktop Entry]\nType=Application\nName=Automatic Discovery Player\n'
            f'Exec="{launcher}" %U\nTerminal=false\n')
        os.environ.update(XDG_DATA_HOME=str(data), XDG_DATA_DIRS=str(root / 'empty-data'),
                          XDG_STATE_HOME=str(root / 'journal'))
        state = root / 'state'
        state.mkdir()
        namespace = hashlib.sha256(os.fsencode(state.resolve())).hexdigest()[:24]
        prefix = 'deckforge.' + namespace + '.'
        sink = 'df_discovery_' + uuid.uuid4().hex[:12]
        arguments = f'sink_name={sink} rate=48000 channels=2'
        fixture_module = int(pulse('load-module', 'module-null-sink', arguments))
        pulse('set-default-sink', sink)
        pulse('set-default-source', sink + '.monitor')
        before = info()
        host, process = None, None
        additional = []
        try:
            host = Host(binary, [], state, root / 'host.log').ready()
            host.post({'command': 'mixer_refresh'})
            catalog = host.wait(lambda s: any(a['id'] == app_id for a in s['mixer'].get('applications', [])),
                                'installed application before it plays audio')['mixer']
            entry = next(a for a in catalog['applications'] if a['id'] == app_id)
            assert entry['name'] == 'Automatic Discovery Player' and not entry['detected']
            assert not catalog['active'] and not catalog['config']['enabled']
            assert not any(d['owned'] for d in catalog['sinks'])
            previous = catalog['config']
            config = copy.deepcopy(previous)
            config.update(enabled=True,
                          channels=[{'id': 'automatic', 'name': 'Automatic', 'apps': [
                              {'field': 'desktop_application', 'value': app_id}], 'sources': []},
                              {'id': 'general', 'name': 'Other apps', 'apps': [], 'sources': [], 'capture_unassigned': True}],
                          routes=[{'channel': 'automatic', 'mix': 'monitor', 'volume': 100, 'muted': False},
                                  {'channel': 'general', 'mix': 'monitor', 'volume': 100, 'muted': False}])
            host.post({'command': 'mixer_config', 'settings': config, 'expected': previous})
            host.wait(lambda s: s['mixer']['active'] and not s['mixer']['pending'], 'owned graph created')
            with (root / 'player.log').open('w') as log:
                process = subprocess.Popen([str(player), sink], stdout=log, stderr=log)
                routed = host.wait(lambda s: any(a.get('desktop_application') == app_id and
                    a['sink'] == prefix + 'channel.automatic' for a in s['mixer']['apps']),
                    'automatically matched future process routed')['mixer']
                assert next(a for a in routed['applications'] if a['id'] == app_id)['detected']
                other = root / 'Other player'
                shutil.copy2(player, other)
                additional.append(subprocess.Popen([str(other), sink, 'Default routing fixture'], stdout=log, stderr=log))
                def at_sink(snapshot, name, destination):
                    return any(a['name'] == name and a['sink'] == destination for a in snapshot['mixer']['apps'])
                host.wait(lambda s: at_sink(s, 'Default routing fixture', prefix + 'channel.general'), 'new unassigned app enters default group')
                latest = host.state()['mixer']['config']
                explicit = copy.deepcopy(latest)
                explicit['channels'][0]['apps'].append({'field': 'process_binary', 'value': 'Other player'})
                host.post({'command': 'mixer_config', 'settings': explicit, 'expected': latest})
                host.wait(lambda s: at_sink(s, 'Default routing fixture', prefix + 'channel.automatic'), 'specific assignment takes priority over default')
                latest = host.state()['mixer']['config']
                restore_default = copy.deepcopy(latest)
                restore_default['channels'][0]['apps'].pop()
                host.post({'command': 'mixer_config', 'settings': restore_default, 'expected': latest})
                host.wait(lambda s: at_sink(s, 'Default routing fixture', prefix + 'channel.general'), 'removing exception returns app to default')
                latest = host.state()['mixer']['config']
                no_default = copy.deepcopy(latest)
                no_default['channels'][1]['capture_unassigned'] = False
                host.post({'command': 'mixer_config', 'settings': no_default, 'expected': latest})
                host.wait(lambda s: at_sink(s, 'Default routing fixture', sink), 'turning default group off restores unmatched app')
                assert at_sink(host.state(), 'Deckforge discovery fixture', prefix + 'channel.automatic')
                latest = host.state()['mixer']['config']
                enabled_default = copy.deepcopy(latest)
                enabled_default['channels'][1]['capture_unassigned'] = True
                host.post({'command': 'mixer_config', 'settings': enabled_default, 'expected': latest})
                host.wait(lambda s: at_sink(s, 'Default routing fixture', prefix + 'channel.general'), 're-enabling default group captures unmatched app')
                direct = root / 'Direct player'
                shutil.copy2(player, direct)
                additional.append(subprocess.Popen([str(direct), prefix + 'channel.automatic', 'Direct routing fixture'], stdout=log, stderr=log))
                host.wait(lambda s: at_sink(s, 'Direct routing fixture', prefix + 'channel.automatic'), 'direct channel choice observed')
                time.sleep(1.1)
                assert at_sink(host.state(), 'Direct routing fixture', prefix + 'channel.automatic'), 'Default stole direct group assignment'
                moving = next(a for a in host.state()['mixer']['apps'] if a['name'] == 'Default routing fixture')
                pulse('move-sink-input', str(moving['index']), sink)
                time.sleep(1.1)
                assert at_sink(host.state(), 'Default routing fixture', sink), 'Default undid manual output choice'
                latest = host.state()['mixer']['config']
                unrelated = copy.deepcopy(latest)
                unrelated['channels'][0]['apps'].append({'field': 'application_id', 'value': 'org.deckforge.NotRunningFixture'})
                host.post({'command': 'mixer_config', 'settings': unrelated, 'expected': latest})
                time.sleep(1.1)
                assert at_sink(host.state(), 'Default routing fixture', sink), 'Unrelated app assignment undid manual output choice'
                for child in additional:
                    stop(child)
                additional.clear()
                routed = host.state()['mixer']
                disabled = copy.deepcopy(routed['config'])
                disabled['enabled'] = False
                host.post({'command': 'mixer_config', 'settings': disabled, 'expected': routed['config']})
                host.wait(lambda s: any(a.get('desktop_application') == app_id and a['sink'] == sink
                    for a in s['mixer']['apps']) and not s['mixer']['active'], 'original owned sink restored')
                assert process.poll() is None, 'Discovery must not terminate the application'
                stop(process)
                process = None
            host.stop()
            host = None
            assert not any(prefix in value[1] for value in modules().values())
            after = info()
            assert all(before[key] == after[key] for key in ('default_sink_name', 'default_source_name'))
            report.update(ok=True, listed_before_audio=True, friendly_name=True,
                          quoted_executable_matched=True, literal_launcher_matched=True, future_process_routed=True,
                          default_captures_unassigned=True, default_toggle_restores=True, explicit_assignment_wins=True,
                          direct_group_assignment_preserved=True, manual_output_change_preserved=True, unrelated_edit_preserves_manual_route=True,
                          original_output_restored=True, default_devices_unchanged=True,
                          no_user_apps_launched=True, no_physical_audio=True, private_audio_server=True)
        finally:
            stop(process)
            for child in additional:
                stop(child)
            if host:
                host.stop()
            for index, details in reversed(list(modules().items())):
                if prefix in details[1] and modules().get(index) == details:
                    subprocess.run(['pactl', 'unload-module', str(index)], check=True)
            if modules().get(fixture_module) == ['module-null-sink', arguments]:
                subprocess.run(['pactl', 'unload-module', str(fixture_module)], check=True)
            args.report.write_text(json.dumps(report, indent=2) + '\n')
    print('PASS: installed app selected before audio; actual process detected/routed/restored, defaults unchanged.')


if __name__ == '__main__':
    main()
