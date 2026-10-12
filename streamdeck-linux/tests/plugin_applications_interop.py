#!/usr/bin/env python3
"""Real current-user process observation through the unchanged official SDK.

Only copied sleep binaries owned by this test are launched/stopped. No USB,
user application manipulation, command-line inventory or external network.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

from multi_device_interop import rows, wait
from sample_interop import COUNTER_LOCK_SHA256, require
from system_interop import Host


def probe(root, sdk, plugin_id, declared):
    plugin = root/(plugin_id+'.sdPlugin'); plugin.mkdir()
    events = plugin/'events.jsonl'
    source = r'''import streamDeck from __SDK__;
import {appendFileSync} from 'node:fs';
const write=(event,ev)=>appendFileSync(__EVENTS__,JSON.stringify({event,application:ev.application})+'\n');
streamDeck.system.onApplicationDidLaunch(ev=>write('launch',ev));
streamDeck.system.onApplicationDidTerminate(ev=>write('exit',ev));
await streamDeck.connect();
'''.replace('__SDK__',json.dumps(sdk.as_uri())).replace('__EVENTS__',json.dumps(str(events)))
    (plugin/'plugin.mjs').write_text(source)
    (plugin/'manifest.json').write_text(json.dumps({
        'UUID':plugin_id,'Name':plugin_id,'Version':'1.0.0','SDKVersion':2,
        'CodePath':'plugin.mjs','Nodejs':{'Version':'20'},'ApplicationsToMonitor':{'windows':[declared]},
        'Actions':[{'UUID':plugin_id+'.key','Name':'Application probe','Controllers':['Keypad'],'States':[{}]}],
    }))
    return plugin, events


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/debug/deckforge')
    parser.add_argument('--report',type=Path,required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    sample = project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'SDK fixture changed')
    sdk = sample/'node_modules/@elgato/streamdeck/dist/plugin/index.js'
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-applications-') as directory:
        root = Path(directory)
        program = root/'owned application'; shutil.copy2('/usr/bin/sleep',program)
        other = root/'other application'; shutil.copy2('/usr/bin/sleep',other)
        processes = []
        def start(path=program):
            child = subprocess.Popen([str(path),'120'])
            processes.append(child)
            return child
        first, second = start(), start()
        plugin_id, idle_id = 'com.deckforge.application-probe','com.deckforge.application-other'
        plugin, events = probe(root,sdk,plugin_id,'Game.exe')
        idle, idle_events = probe(root,sdk,idle_id,'Other.exe')
        state = root/'state'
        host = Host(binary,[plugin,idle],state,root/'host.log')
        try:
            host.ready()
            host.wait(lambda s:len(s['plugins'])==2 and all(p['connected'] for p in s['plugins']),'SDK plugins')
            require(not host.state()['application_monitor']['enabled'],'Windows name automatically enabled scanning')
            require(not rows(events),'Unmapped Windows name received events')
            host.post({'command':'plugin_applications','id':plugin_id,'baseline':{},'mappings':{'Game.exe':str(program)}})
            wait(host,lambda:len(rows(events))==2,'initial replay for both existing processes')
            require(rows(events)==[{'event':'launch','application':'Game.exe'}]*2,'Initial process instances collapsed or logical name lost')
            host.post({'command':'plugin_applications','id':idle_id,'baseline':{},'mappings':{'Other.exe':str(other)}})
            third = start()
            wait(host,lambda:len(rows(events))==3,'new process launch')
            first.terminate(); first.wait(timeout=3)
            wait(host,lambda:len(rows(events))==4,'one process exit while other instances remain')
            require(rows(events)[-1]=={'event':'exit','application':'Game.exe'},'Exit was incorrectly aggregated')
            require(not rows(idle_events),'Uninterested plugin received another executable')
            unrelated = start(other)
            wait(host,lambda:len(rows(idle_events))==1,'other plugin launch')
            require(rows(idle_events)[0]=={'event':'launch','application':'Other.exe'},'Wrong plugin logical ID')
            require(len(rows(events))==4,'Target plugin received unrelated process')
            pids = {p['id']:p['pid'] for p in host.state()['plugins']}
            host.post({'command':'restart_plugin','id':plugin_id})
            host.wait(lambda s:next(p for p in s['plugins'] if p['id']==plugin_id)['connected'],'plugin reconnect')
            wait(host,lambda:len(rows(events))==6,'running processes replayed on plugin reconnect')
            require(rows(events)[-2:]==[{'event':'launch','application':'Game.exe'}]*2,'Reconnect did not replay each process')
            require(next(p for p in host.state()['plugins'] if p['id']==idle_id)['pid']==pids[idle_id],'Other plugin restarted')
            host.post({'command':'plugin_applications','id':plugin_id,'baseline':{},'mappings':{}},expected=400)
            require(json.loads((state/'session.json').read_text())['application_mappings'][plugin_id]=={'Game.exe':str(program)},'Mappings not persisted')
            host.stop()
            host = Host(binary,[],state,root/'restart.log')
            host.ready()
            wait(host,lambda:len(rows(events))==8,'saved mappings restored and each process replayed')
            wait(host,lambda:len(rows(idle_events))==2,'other saved mapping restored')
            host.post({'command':'plugin_applications','id':plugin_id,'baseline':{'Game.exe':str(program)},'mappings':{'Game.exe':''}})
            wait(host,lambda:len(rows(events))==10,'removing subscription balances delivered processes')
            require(rows(events)[-2:]==[{'event':'exit','application':'Game.exe'}]*2,'Unmapping delivered wrong transitions')
            require(second.poll() is None and third.poll() is None and unrelated.poll() is None,'Monitoring changed process lifetime')
            report = {'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sdk_lock_sha256':COUNTER_LOCK_SHA256,
                      'initial_replay_per_process':True,'live_launch_and_individual_exit':True,'interested_plugin_isolation':True,
                      'plugin_reconnect_replay':True,'host_restart_saved_mappings':True,'stale_settings_rejected':True,
                      'unmapping_balanced_without_terminating_processes':True,
                      'boundary':'Real /proc observation of temporary owned executables; unchanged SDK 2.0.4, simulated devices. No user apps, audio, USB or external network modified.'}
        finally:
            host.stop()
            for child in processes:
                if child.poll() is None: child.terminate()
                child.wait(timeout=3)
            require(host.process.returncode==0,'Host shutdown failed')
    args.report.parent.mkdir(parents=True,exist_ok=True)
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: SDK receives per-process launch/exit, initial/reconnect replay, isolated mapped names and saved mappings.')


if __name__=='__main__': main()
