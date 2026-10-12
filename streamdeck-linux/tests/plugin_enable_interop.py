#!/usr/bin/env python3
"""Plugin enable/disable ownership with real owned children and simulated devices.

No external service, Discord, USB, keyboard, audio, or user state is accessed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import time

from multi_device_interop import assign, inspect, post, press, rows, wait
from sample_interop import COUNTER, COUNTER_LOCK_SHA256, require
from system_interop import Host

PLUGIN='com.deckforge.enable-probe'
ACTION=PLUGIN+'.key'
ROUTINE='com.elgato.streamdeck.multiactions.routine'
DELAY='com.elgato.streamdeck.multiactions.delay'


def probe(root, sample):
    path=root/(PLUGIN+'.sdPlugin');path.mkdir()
    events=path/'events.jsonl'
    source=r'''import{createRequire}from'node:module';import{appendFileSync}from'node:fs';
const WebSocket=createRequire(__PACKAGE__)('ws'),args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const record=(kind,extra={})=>appendFileSync(__EVENTS__,JSON.stringify({kind,pid:process.pid,...extra})+'\n');
const ws=new WebSocket('ws://127.0.0.1:'+args['-port']);
const held=new Set(),visible=new Set(),inspectors=new Set();let stopping=false,finished=false;
const pending=()=>({held:[...held],visible:[...visible],inspectors:[...inspectors]});
const finishIfClean=()=>{if(!stopping||finished||held.size||visible.size||inspectors.size)return;
finished=true;record('cooperative-cleanup-complete',pending());process.exit(0);};
ws.on('open',()=>{ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']}));record('registered',{registration:args['-pluginUUID']});});
ws.on('message',data=>{const m=JSON.parse(data);record(m.event,{context:m.context,device:m.device,payload:m.payload,id:m.id});
if(m.event==='willAppear')visible.add(m.context);
if(m.event==='willDisappear')visible.delete(m.context);
if(m.event==='keyDown')held.add(m.context);
if(m.event==='keyUp')held.delete(m.context);
if(m.event==='propertyInspectorDidAppear')inspectors.add(m.context);
if(m.event==='propertyInspectorDidDisappear')inspectors.delete(m.context);
finishIfClean();});
ws.on('error',()=>{});
const heartbeat=setInterval(()=>{if(ws.readyState===1)ws.send(JSON.stringify({event:'getGlobalSettings',context:args['-pluginUUID'],id:'heartbeat'}));},100);
process.on('SIGTERM',()=>{clearInterval(heartbeat);stopping=true;record('sigterm',pending());
if(ws.readyState===1){ws.send(JSON.stringify({event:'setGlobalSettings',context:args['-pluginUUID'],payload:{stalePoison:true}}));record('stale-callback-attempt');}
// SIGTERM and socket delivery are independent event sources. Exit when cleanup
// is actually observed; the deadline only bounds an uncooperative/failed case.
setTimeout(()=>{record('cooperative-cleanup-deadline',pending());process.exit(0);},250);
finishIfClean();});
'''.replace('__PACKAGE__',json.dumps(str(sample/'package.json'))).replace('__EVENTS__',json.dumps(str(events)))
    (path/'plugin.mjs').write_text(source)
    (path/'pi.html').write_text('''<!doctype html><title>Owned enable probe</title><script>
window.connectElgatoStreamDeckSocket=(port,uuid,event,info,action)=>{
 const ws=new WebSocket(`ws://127.0.0.1:${port}`);
 ws.onopen=()=>{ws.send(JSON.stringify({event,uuid}));ws.send(JSON.stringify({event:'sendToPlugin',context:uuid,payload:{inspectorReady:true}}));};
};</script>''')
    (path/'manifest.json').write_text(json.dumps({'UUID':PLUGIN,'Name':'Enable probe','Version':'1.0.0','SDKVersion':2,'CodePath':'plugin.mjs','Nodejs':{'Version':'20'},'Actions':[{'UUID':ACTION,'Name':'Enable probe key','Controllers':['Keypad'],'SupportedInMultiActions':True,'States':[{}],'PropertyInspectorPath':'pi.html'}]}))
    return path,events


def plugin(host,id=PLUGIN):
    return next(p for p in host.state()['plugins'] if p['id']==id)


def child_pids(host):
    # Child ownership is recorded under the creating Tokio worker thread.
    result=set()
    for path in Path(f'/proc/{host.process.pid}/task').glob('*/children'):
        try: result.update(int(value) for value in path.read_text().split())
        except FileNotFoundError: pass
    return result


def identities(host,devices):
    return {device:[{'context':s['context'],'uuid':s['uuid'],'settings':s['settings']} for s in inspect(host,device)['slots']] for device in devices}


def reject_old_registration(host, sample, registration):
    ws=subprocess.check_output(['node','-e','console.log(require.resolve("ws",{paths:[process.argv[1]]}))',str(sample)],text=True).strip()
    script=r'''const WebSocket=require(process.argv[1]);const socket=new WebSocket(process.argv[2]);
socket.on('open',()=>{socket.send(JSON.stringify({event:'registerPlugin',uuid:process.argv[3]}));socket.send(JSON.stringify({event:'setGlobalSettings',payload:{stalePoison:true}}));});
socket.on('close',()=>process.exit(0));socket.on('error',()=>{});setTimeout(()=>process.exit(2),1200);'''
    subprocess.run(['node','-e',script,ws,host.base.replace('http:','ws:'),registration],check=True,timeout=3)


def main():
    project=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/debug/deckforge')
    parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--require-wire-cleanup',action='store_true',help='Require the cooperative fixture to process cleanup before its bounded shutdown exit')
    args=parser.parse_args();binary=args.binary.resolve(strict=True)
    sample=project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'Official sample dependency changed')
    counter=sample/'com.elgato.counter.sdPlugin'
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-enable-') as d:
        root=Path(d);target,events=probe(root,sample);state=root/'state'
        host=Host(binary,[counter,target],state,root/'host.log')
        try:
            host.ready();host.wait(lambda s:len(s['plugins'])==2 and all(p['connected'] for p in s['plugins']),'both plugin registrations')
            a=host.state()['device'];b='enable-mini'
            contexts={a:assign(host,a,0,ACTION,{'marker':'first','Future':9})}
            assign(host,a,1,COUNTER,{'count':4,'incrementBy':1})
            host.post({'command':'device_add','device':b,'model':'mini'})
            contexts[b]=assign(host,b,0,ACTION,{'marker':'second','Future':8})
            assign(host,b,1,COUNTER,{'count':8,'incrementBy':1})
            multi=assign(host,a,2,ROUTINE)
            raw={'Actions':[{'Actions':[{'ActionID':'wait','UUID':DELAY,'Settings':{'delay':1000},'States':[{}]},{'ActionID':'target','UUID':ACTION,'Settings':{'marker':'queued'},'States':[{}]}]},{'Actions':[]}]}
            post(host,a,'multi_save',context=multi,raw=raw)
            before_inspectors=child_pids(host)
            for device,context in contexts.items():
                post(host,device,'open_inspector',context=context)
                wait(host,lambda:any(r.get('kind')=='sendToPlugin' and r.get('context')==context and r.get('payload',{}).get('inspectorReady') for r in rows(events)),'inspector ready')
            inspector_pids=child_pids(host)-before_inspectors
            require(len(inspector_pids)>=2,'Native inspector children were not observed')
            host.post({'command':'device_select','device':b})
            before=identities(host,[a,b]);target_pid=plugin(host)['pid'];counter_pid=plugin(host,'com.elgato.counter')['pid']
            registration=next(r['registration'] for r in rows(events) if r['kind']=='registered' and r['pid']==target_pid)
            post(host,a,'input',event='keyDown',index=0)
            wait(host,lambda:any(r['kind']=='keyDown' and r.get('context')==contexts[a] for r in rows(events)),'owned held key reaches target')
            press(host,a,2)
            wait(host,lambda:next(s for s in inspect(host,a)['slots'] if s['context']==multi)['multi_running'],'delayed Multi running')
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':False,'expected_enabled':True})
            off=plugin(host)
            require(off['enabled'] is False and off['status']=='disabled' and off['pid'] is None and not off['connected'],'Disabled runtime still active')
            require(not Path(f'/proc/{target_pid}').exists(),'Disabled child not reaped')
            old_events=[r for r in rows(events) if r['pid']==target_pid]
            signal_index=next((i for i,r in enumerate(old_events) if r['kind']=='sigterm'),len(old_events))
            before_signal=old_events[:signal_index]
            wire_cleanup={
                'held_key_release':any(r['kind']=='keyUp' and r.get('context')==contexts[a] for r in before_signal),
                'both_disappearances':all(any(r['kind']=='willDisappear' and r.get('context')==c for r in before_signal) for c in contexts.values()),
                'both_inspector_disappearances':all(any(r['kind']=='propertyInspectorDidDisappear' and r.get('context')==c for r in before_signal) for c in contexts.values()),
            }
            complete_index=next((i for i,r in enumerate(old_events) if r['kind']=='cooperative-cleanup-complete'),None)
            consumed=old_events[:complete_index] if complete_index is not None else []
            eventual_cleanup={
                'held_key_release':any(r['kind']=='keyUp' and r.get('context')==contexts[a] for r in consumed),
                'both_disappearances':all(any(r['kind']=='willDisappear' and r.get('context')==c for r in consumed) for c in contexts.values()),
                'both_inspector_disappearances':all(any(r['kind']=='propertyInspectorDidDisappear' and r.get('context')==c for r in consumed) for c in contexts.values()),
                'completed_before_exit':complete_index is not None,
            }
            if args.require_wire_cleanup:
                require(all(eventual_cleanup.values()),'Cooperative plugin did not process cleanup before exit: '+json.dumps(eventual_cleanup))
            require(all(not Path(f'/proc/{pid}').exists() for pid in inspector_pids),'Disable left owned inspector process running')
            require(plugin(host,'com.elgato.counter')['pid']==counter_pid,'Disable restarted unrelated Counter')
            require(host.state()['device']==b,'Disable changed selected device')
            require(identities(host,[a,b])==before,'Disable changed contexts/settings')
            for device in [a,b]:
                s=inspect(host,device)
                require(not s['inspectors'],'Disable left native inspector binding')
                require(not any(slot['multi_running'] for slot in s['slots']),'Disable left affected Multi running')
            host.post({'command':'restart_plugin','id':PLUGIN},expected=400)
            host.post({'command':'open_inspector','device':a,'context':contexts[a]},expected=400)
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':True,'expected_enabled':True},expected=400)
            reject_old_registration(host,sample,registration)
            require(plugin(host)['status']=='disabled','Stale registration changed disabled status')
            # Counter continues to handle simulated input while the target is off.
            press(host,b,1)
            wait(host,lambda:next(s for s in inspect(host,b)['slots'] if s['uuid']==COUNTER)['settings']['count']==9,'independent Counter remains operational')
            # Late release is consumed; re-enable must not resurrect held/Multi work.
            post(host,a,'input',event='keyUp',index=0)
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':True,'expected_enabled':False})
            host.wait(lambda s:next(p for p in s['plugins'] if p['id']==PLUGIN)['connected'],'explicit enable registers')
            new_pid=plugin(host)['pid'];require(new_pid!=target_pid,'Enable reused terminated child')
            wait(host,lambda:all(any(r['kind']=='willAppear' and r['pid']==new_pid and r.get('context')==c for r in rows(events)) for c in contexts.values()),'both devices replay on enable')
            reject_old_registration(host,sample,registration)
            time.sleep(1.1)
            new_events=[r for r in rows(events) if r['pid']==new_pid]
            require(not any(r['kind'] in ('keyDown','keyUp') for r in new_events),'Enable replayed cancelled key/Multi callbacks')
            saved=json.loads((state/'session.json').read_text())
            require(not saved.get('globals',{}).get(PLUGIN,{}).get('stalePoison'),'Old session callback changed globals')
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':False,'expected_enabled':True})
            require(plugin(host,'com.elgato.counter')['pid']==counter_pid,'Enable cycle changed Counter PID')
            final_before=identities(host,[a,b]);records_before=len(rows(events))
        except Exception:
            print(host.log.read_text()[-6000:])
            print(json.dumps(rows(events)[-12:],indent=2))
            raise
        finally:
            host.stop();require(host.process.returncode==0,'Host shutdown failed')
        host=Host(binary,[],state,root/'restart.log')
        try:
            host.ready();host.wait(lambda s:next(p for p in s['plugins'] if p['id']=='com.elgato.counter')['connected'],'Counter restart')
            off=plugin(host)
            require(off['enabled'] is False and off['pid'] is None and off['status']=='disabled','Host restart auto-enabled disabled plugin')
            require(Path(off['path']).resolve()==target.resolve(),'Disabled plugin root identity changed')
            require(identities(host,[a,b])==final_before,'Host restart lost disabled action settings/contexts')
            time.sleep(.2)
            require(len(rows(events))==records_before,'Disabled child launched during host restart')
            host.post({'command':'restart_plugin','id':PLUGIN},expected=400)
            host.post({'command':'plugin_enabled','id':PLUGIN,'enabled':True,'expected_enabled':False})
            host.wait(lambda s:next(p for p in s['plugins'] if p['id']==PLUGIN)['connected'],'enable after saved disabled restart')
            final_pid=plugin(host)['pid']
            wait(host,lambda:all(any(r['kind']=='willAppear' and r['pid']==final_pid and r.get('context')==c for r in rows(events)) for c in contexts.values()),'retained disabled instances reappear')
            report={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'cleanup_observed_before_sigterm':wire_cleanup,'cooperative_cleanup_before_exit':eventual_cleanup,'two_devices':True,'settings_contexts_retained':True,'unrelated_counter_pid_retained_during_toggle':True,'native_inspectors_closed':True,'held_and_multi_cancelled_without_replay':True,'stale_registration_rejected':True,'stale_write_did_not_change_globals':True,'termination_callback_attempted':any(r['kind']=='stale-callback-attempt' for r in rows(events)),'disabled_root_survives_host_restart':True,'explicit_enable_replays_visible_instances':True,'sigterm_records':[r for r in rows(events) if r['kind'] in ('sigterm','stale-callback-attempt')],'boundary':'Temporary simulated host, generated generic-ws plugin and unchanged Counter. Before-SIGTERM fields are observational, not required: OS signals and remote handlers are independently scheduled. Cleanup is required before the cooperative fixture exits, using an event-driven completion condition with250ms safety deadline. This does not prove remote side effects completed in arbitrary plugins. Deep-link queue policy requires separate Host tests; no external services or hardware.'}
        finally:
            host.stop();require(host.process.returncode==0,'Restarted host shutdown failed')
    args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: plugin disable/enable across2devices, PI/held/Multi cleanup, Counter isolation, stale-session rejection, disabled persistence, explicit replay.')

if __name__=='__main__':main()
