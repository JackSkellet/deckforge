#!/usr/bin/env python3
"""Real plugin MultiAction lifecycle/timing, native builtins, persistence and original sample.
No USB or keyboard injection; only test-owned loopback HTTP is triggered.
"""
import argparse
import http.server
import hashlib
import json
from pathlib import Path
import tempfile
import threading
import time
import uuid
import zipfile
from system_interop import Host
from sample_interop import require,slot,tap

ROUTINE='com.elgato.streamdeck.multiactions.routine'
SWITCH='com.elgato.streamdeck.multiactions.routine2'
RANDOM='com.elgato.streamdeck.multiactions.random'
DELAY='com.elgato.streamdeck.multiactions.delay'
ACTION='com.deckforge.multi-probe.action'
HELLO='com.elgato.hello-world.newstring'

def child(action=ACTION,settings=None,**extra):
    return {'UUID':action,'ActionID':str(uuid.uuid4()),'Name':action,'Settings':settings or {},'States':[{},{}],'State':0,**extra}

def make_probe(path,require_root):
    path.mkdir()
    (path/'plugin.js').write_text(r'''import {createRequire} from 'node:module';
const WebSocket=createRequire(__ROOT__)('ws');
const args={};for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
const send=(event,context,payload)=>ws.send(JSON.stringify({event,context,payload}));
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{const m=JSON.parse(data);if(!m.payload)return;
 if(['willAppear','willDisappear','keyDown','keyUp','didReceiveSettings','propertyInspectorDidAppear','propertyInspectorDidDisappear'].includes(m.event))
 send('logMessage',args['-pluginUUID'],{message:'MULTI '+JSON.stringify({event:m.event,context:m.context,payload:m.payload,at:performance.now()})});
 if(m.event==='keyUp') {send('setSettings',m.context,{...m.payload.settings,count:(m.payload.settings.count||0)+1});send('setState',m.context,{state:1});}
});'''.replace('__ROOT__',json.dumps(str(require_root))))
    (path/'manifest.json').write_text(json.dumps({'UUID':'com.deckforge.multi-probe','Name':'Multi probe','CodePath':'plugin.js','Nodejs':{'Version':'20'},'SDKVersion':2,'Actions':[{'UUID':ACTION,'Name':'Step probe','Controllers':['Keypad'],'States':[{},{}],'PropertyInspectorPath':'pi.html','MultiActionDelays':{'Delay1':25,'Delay2':30}}]}))
    (path/'pi.html').write_text('''<!doctype html><html><body><label>Child count<input id="count" type="number"></label><script>
window.connectElgatoStreamDeckSocket=(port,id,event,info,action)=>{const a=JSON.parse(action);const ws=new WebSocket('ws://127.0.0.1:'+port);ws.onopen=()=>ws.send(JSON.stringify({event,uuid:id}));document.querySelector('#count').value=a.payload.settings.count||0;document.querySelector('#count').onchange=e=>ws.send(JSON.stringify({event:'setSettings',context:a.context,payload:{...a.payload.settings,count:Number(e.target.value)}}));};
</script></body></html>''')
    return path

def events(state,kind=None):
    rows=[json.loads(line.split('MULTI ',1)[1]) for line in state['logs'] if 'MULTI ' in line]
    return [r for r in rows if kind is None or r['event']==kind]

def configure(host,uuid_,groups):
    host.post({'command':'assign','controller':'Keypad','index':0,'uuid':uuid_})
    parent=slot(host.state(),uuid_)
    raw={'Actions':[{'Actions':group} for group in groups],'FutureContainer':73}
    host.post({'command':'multi_save','context':parent['context'],'raw':raw})
    return slot(host.state(),uuid_)

def finished(host):
    return host.wait(lambda s:not s['multi_runs'],'sequence completion')

class Requests(http.server.BaseHTTPRequestHandler):
    paths=[]
    def do_GET(self):self.paths.append(self.path);self.send_response(204);self.end_headers()
    def log_message(self,*_):pass

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    args=parser.parse_args()
    project=Path(__file__).resolve().parents[1];evidence=project.parent/'evidence'
    hello=evidence/'official-samples/hello-world/com.elgato.hello-world.sdPlugin'
    require((hello/'bin/plugin.js').exists(),'Build unchanged official Hello World sample first')
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Requests);threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='deckforge-multi-') as directory:
            root=Path(directory);state=root/'state'
            probe=make_probe(root/'probe.sdPlugin',evidence/'official-samples/counter/package.json')
            host=Host(args.binary.resolve(),[probe,hello],state,root/'host.log')
            try:
                host.ready();host.wait(lambda s:all(p['connected'] for p in s['plugins']),'plugin connections')
                # Exact configured timing, explicit delay and desired-state semantics.
                parent=configure(host,ROUTINE,[[child(settings={'tag':'one'},OverrideState=1,MultiActionDelays={'Delay1':35,'Delay2':45}),child(DELAY,{'delay':120}),child(settings={'tag':'two'})],[]])
                children=parent['multi_children'][0];contexts=[children[0]['context'],children[2]['context']]
                host.wait(lambda s:all(any(e['context']==c for e in events(s,'willAppear')) for c in contexts),'children appeared')
                tap(host);host.wait(lambda s:len([e for e in events(s,'keyUp') if e['context'] in contexts])==2,'two child releases');snapshot=finished(host)
                steps=[e for e in events(snapshot) if e['context'] in contexts and e['event'] in ('keyDown','keyUp')]
                require([e['event'] for e in steps]==['keyDown','keyUp','keyDown','keyUp'],'Sequence order incorrect')
                require(steps[1]['at']-steps[0]['at']>=30,'Configured virtual hold compressed')
                require(steps[2]['at']-steps[1]['at']>=155,'Gap+explicit delay compressed')
                require(steps[3]['at']-steps[2]['at']>=20,'Manifest virtual hold fallback ignored')
                for event in steps:
                    require(event['payload']['isInMultiAction'] is True and 'coordinates' not in event['payload'],'Wrong child SDK payload')
                require(steps[0]['payload']['userDesiredState']==1 and steps[0]['payload']['state']==0,'OverrideState confused with live state')
                parent=slot(snapshot,ROUTINE)
                require(parent['raw']['Actions'][0]['Actions'][0]['Settings']['count']==1,'Child settings not synchronized')
                require(parent['raw']['Actions'][0]['Actions'][0]['OverrideState']==1,'Desired state overwritten by runtime state')
                # Child inspector receives its own stable context.
                host.post({'command':'open_inspector','context':contexts[0]})
                host.wait(lambda s:any(i['context']==contexts[0] and i['connected'] for i in s['inspectors']),'child inspector registration')
                # Cancel while held: balanced Up, no next child, Switch does not flip.
                parent=configure(host,SWITCH,[[child(MultiActionDelays={'Delay1':100,'Delay2':100}),child(DELAY,{'delay':3000}),child()], [child(settings={'branch':2})]])
                first=parent['multi_children'][0][0]['context'];last=parent['multi_children'][0][2]['context']
                tap(host);host.wait(lambda s:any(e['context']==first for e in events(s,'keyDown')),'running switch began')
                tap(host);snapshot=finished(host)
                require(slot(snapshot,SWITCH)['state']==0,'Cancelled switch advanced')
                require(not any(e['context']==last for e in events(snapshot,'keyDown')),'Cancelled later step executed')
                require(len([e for e in events(snapshot,'keyUp') if e['context']==first])==1,'Cancelled key was not released exactly once')
                parent=configure(host,SWITCH,[[child(settings={'branch':1})],[child(settings={'branch':2})]])
                tap(host);host.wait(lambda s:slot(s,SWITCH)['state']==1,'successful switch advanced')
                tap(host);host.wait(lambda s:slot(s,SWITCH)['state']==0,'second switch branch returned')
                # Original allowed Hello World SDK sample stays unchanged.
                parent=configure(host,ROUTINE,[[child(HELLO,{'language':'en','titleKey':'__before_multi_test__'})],[]])
                original_context=parent['multi_children'][0][0]['context']
                tap(host)
                host.wait(lambda s:slot(s,ROUTINE)['multi_children'][0][0]['settings'].get('titleKey') in ['flash','selectprop','on','off','toggle','action','dynamic','clickme'],'unchanged HelloWorld nested action')
                finished(host)
                exported=root/'multi.streamDeckProfile';host.post({'command':'export_profile','path':str(exported)})
                with zipfile.ZipFile(exported) as archive:
                    records=[v for name in archive.namelist() if name.endswith('manifest.json') for c in json.loads(archive.read(name)).get('Controllers',[]) for v in c.get('Actions',{}).values()]
                outer=next(r for r in records if r.get('UUID')==ROUTINE)
                require(outer['FutureContainer']==73 and outer['Actions'][0]['Actions'][0]['UUID']==HELLO,'Export dropped nested tree')
                before=slot(host.state(),ROUTINE)['multi_children'][0][0]['settings']
            finally:host.stop()
            restarted=Host(args.binary.resolve(),[],state,root/'restart.log')
            try:
                restarted.ready();restarted.wait(lambda s:all(p['connected'] for p in s['plugins']),'restart plugins')
                child_state=slot(restarted.state(),ROUTINE)['multi_children'][0][0]
                require(child_state['context']==original_context and child_state['settings']==before,'Child identity/settings lost on restart')
                # Builtin URL execution remains ordered through routine dispatch.
                url=f'http://127.0.0.1:{server.server_port}/multi'
                configure(restarted,ROUTINE,[[child('com.elgato.streamdeck.system.website',{'path':url,'openInBrowser':False}),child(DELAY,{'delay':50}),child('com.elgato.streamdeck.system.website',{'path':url+'2','openInBrowser':False})],[]])
                tap(restarted);restarted.wait(lambda s:Requests.paths==['/multi','/multi2'],'ordered builtin requests');finished(restarted)
                # Random chooses one child per invocation and never runs the entire list.
                parent=configure(restarted,RANDOM,[[child(settings={'choice':i}) for i in range(3)],[]])
                ids={c['context'] for c in parent['multi_children'][0]}
                for _ in range(4):
                    count=len([e for e in events(restarted.state(),'keyDown') if e['context'] in ids])
                    tap(restarted);restarted.wait(lambda s:len([e for e in events(s,'keyDown') if e['context'] in ids])==count+1,'one random child');finished(restarted)
                # Native save validates nesting before modifying the active sequence.
                prior=slot(restarted.state(),RANDOM)['raw']
                restarted.post({'command':'multi_save','context':slot(restarted.state(),RANDOM)['context'],'raw':{'Actions':[{'Actions':[child(ROUTINE)]}]}},expected=400)
                require(slot(restarted.state(),RANDOM)['raw']==prior,'Rejected save changed sequence')

            finally:restarted.stop()
            # The shipped routine belongs to Stream Deck XL (20GAT9902).
            # Importing it on Plus preserves it but does not activate it.
            original_state=root/'original-xl-state'
            original_host=Host(args.binary.resolve(),[],original_state,root/'original-xl.log',model='xl')
            try:
                original_host.ready()
                # Read-only acceptance of the unchanged shipped Windows routine.
                archive=evidence/'elgato-7.6/raw/flsFWZ6Vo1szVxzaQW28UVc39Qlk1w'
                require(hashlib.sha256(archive.read_bytes()).hexdigest()=='f1f4ba26000ae5f39a25cb2524e273295e5930e9373477c838389abf3f5a1294','Original routine fixture changed')
                original_host.post({'command':'import_profile','path':str(archive)})
                loaded=json.loads((original_state/'session.json').read_text())
                profile=next(p for p in loaded['library']['profiles'] if p['id']==original_host.state()['library']['active_profile'])
                page=next(p for p in profile['pages'] if any(s['uuid']==ROUTINE for s in p['slots']))
                original_host.post({'command':'page_select','id':page['id']})
                routine=slot(original_host.state(),ROUTINE)
                require(len(routine['multi_children'][0])==5 and not routine['multi_children'][1],'Original child groups lost')
                require([c['settings']['delay'] for c in routine['multi_children'][0] if c['uuid']==DELAY]==[3000,1500],'Original delay values changed')
                require(not original_host.state()['multi_runs'],'Import must never execute original hotkeys')
                original_export=root/'original-multi.streamDeckProfile'
                original_host.post({'command':'export_profile','path':str(original_export)})
                with zipfile.ZipFile(original_export) as archive:
                    rows=[a for name in archive.namelist() if name.endswith('manifest.json') for c in json.loads(archive.read(name)).get('Controllers',[]) for a in (c.get('Actions') or {}).values()]
                exported_routine=next(a for a in rows if a.get('UUID')==ROUTINE)
                require(exported_routine['Actions']==routine['raw']['Actions'],'Original nested settings/fields changed on export')

            finally:original_host.stop()
        print('PASS: child SDK lifecycle/settings/PI, configured+manifest timings, delays, cancellation, success-only switch, random single-step selection, unchanged HelloWorld, builtin HTTP order, export and restart identities. No USB or keyboard injection.')
    finally:server.shutdown();server.server_close()
if __name__=='__main__':main()
