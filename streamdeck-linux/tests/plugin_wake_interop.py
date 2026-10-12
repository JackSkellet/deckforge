#!/usr/bin/env python3
"""Unchanged SDK wake subscriptions across devices. No real sleep or USB access."""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile

from multi_device_interop import assign, inspect, pid_map, rows, wait
from sample_interop import COUNTER_LOCK_SHA256, require
from system_interop import Host


def probe(root, sdk, name):
    plugin = root/(name+'.sdPlugin')
    plugin.mkdir()
    events = plugin/'events.jsonl'
    source = r'''import streamDeck from __SDK__;
import {appendFileSync} from 'node:fs';
const write=value=>appendFileSync(__EVENTS__,JSON.stringify(value)+'\n');
streamDeck.actions.onWillAppear(ev=>write({event:'appear',context:ev.action.id,device:ev.action.device.id,settings:ev.payload.settings}));
streamDeck.system.onSystemDidWakeUp(()=>write({event:'wake',contexts:[...streamDeck.actions].map(a=>a.id)}));
await streamDeck.connect();
'''.replace('__SDK__',json.dumps(sdk.as_uri())).replace('__EVENTS__',json.dumps(str(events)))
    (plugin/'plugin.mjs').write_text(source)
    (plugin/'manifest.json').write_text(json.dumps({
        'UUID':name,'Name':name,'Version':'1.0.0','SDKVersion':2,
        'CodePath':'plugin.mjs','Nodejs':{'Version':'20'},
        'Actions':[{'UUID':name+'.key','Name':'Wake probe','Controllers':['Keypad'],'States':[{}]}],
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
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-wake-') as directory:
        root = Path(directory)
        plugin, events = probe(root,sdk,'com.deckforge.wake')
        idle, idle_events = probe(root,sdk,'com.deckforge.wake-idle')
        host = Host(binary,[plugin,idle],root/'state',root/'host.log')
        try:
            host.ready()
            host.wait(lambda s:len(s['plugins'])==2 and all(p['connected'] for p in s['plugins']),'SDK registration')
            a = host.state()['device']
            action = 'com.deckforge.wake.key'
            hidden = assign(host,a,0,action,{'marker':'hidden'})
            host.post({'command':'page_create','name':'Visible'})
            first = assign(host,a,1,action,{'marker':'first'})
            host.post({'command':'device_add','device':'wake-mini','model':'mini'})
            second = assign(host,'wake-mini',2,action,{'marker':'second'})
            host.post({'command':'device_select','device':'wake-mini'})
            wait(host,lambda:any(r.get('context')==second for r in rows(events)),'second appearance')
            before = {d:inspect(host,d)['slots'] for d in [a,'wake-mini']}
            pids = pid_map(host)
            offsets = len(rows(events)), len(rows(idle_events))
            for count in [1,2]:
                host.post({'command':'system_wake'})
                wait(host,lambda:sum(r.get('event')=='wake' for r in rows(events))==count,'SDK wake')
                wait(host,lambda:sum(r.get('event')=='wake' for r in rows(idle_events))==count,'idle plugin wake')
            observed = rows(events)[offsets[0]:]
            require([r['event'] for r in observed]==['appear','appear','wake']*2,'Wrong replay count or ordering')
            for wake in [r for r in observed if r['event']=='wake']:
                require(set(wake['contexts'])=={first,second},'SDK store contains hidden/missing contexts')
            require(all(r.get('context')!=hidden for r in observed),'Hidden page replayed')
            require(all(r['event']=='wake' and r['contexts']==[] for r in rows(idle_events)[offsets[1]:]),'Idle plugin delivery wrong')
            require(host.state()['device']=='wake-mini','Wake changed selection')
            require(pid_map(host)==pids,'Wake restarted a plugin')
            require({d:inspect(host,d)['slots'] for d in [a,'wake-mini']}==before,'Wake changed settings or slot identity')
            require(host.state()['system_resume']['enabled'] is False,'Simulation started the real observer')
            report = {'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
                      'sdk_lock_sha256':COUNTER_LOCK_SHA256,'two_device_visible_replay':True,
                      'one_global_event_per_plugin_per_wake':True,'idle_plugin_notified':True,
                      'hidden_page_excluded':True,'selection_settings_contexts_pids_preserved':True,
                      'boundary':'Simulated diagnostic wake through unchanged SDK 2.0.4; OS source verified separately on a private D-Bus. No physical suspend or USB.'}
        finally:
            host.stop()
            require(host.process.returncode==0,'Host shutdown failed')
    args.report.parent.mkdir(parents=True,exist_ok=True)
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: SDK wake replay across two devices and an idle plugin; hidden actions excluded, state and processes preserved.')


if __name__=='__main__':
    main()
