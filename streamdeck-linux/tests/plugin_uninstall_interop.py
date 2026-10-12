#!/usr/bin/env python3
"""Live first-plugin removal and managed reinstall, isolated from user state.

Counter's runtime is copied unchanged; only its temporary manifest's inspector
path points to an owned local protocol probe. No CDN, Discord or hardware use.
"""
import argparse
import hashlib
import http.server
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import zipfile

from multi_device_interop import assign, inspect, post, press, rows, wait
from plugin_enable_interop import ACTION, DELAY, PLUGIN, ROUTINE, child_pids, identities, plugin, probe, reject_old_registration
from sample_interop import COUNTER, COUNTER_LOCK_SHA256, require
from system_interop import Host


class CounterInspector:
    def __init__(self, root):
        self.command={'revision':0};self.events=[];owner=self
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_OPTIONS(self):
                self.send_response(204);self.send_header('Access-Control-Allow-Origin','*');self.send_header('Access-Control-Allow-Methods','GET, POST, OPTIONS');self.send_header('Access-Control-Allow-Headers','Content-Type');self.end_headers()
            def do_GET(self):
                body=json.dumps(owner.command).encode();self.send_response(200)
                self.send_header('Content-Type','application/json');self.send_header('Access-Control-Allow-Origin','*');self.end_headers();self.wfile.write(body)
            def do_POST(self):
                size=int(self.headers.get('Content-Length','0'));require(0<size<8192,'Inspector proof size invalid')
                owner.events.append(json.loads(self.rfile.read(size)));self.send_response(204);self.send_header('Access-Control-Allow-Origin','*');self.end_headers()
            def log_message(self,*args):pass
        self.server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True);self.thread.start()
        endpoint=f'http://127.0.0.1:{self.server.server_port}/probe'
        html=r'''<!doctype html><title>Counter inspector ownership probe</title><script>
window.connectElgatoStreamDeckSocket=(port,uuid,event,info,actionInfo)=>{
 const context=JSON.parse(actionInfo).context,ws=new WebSocket(`ws://127.0.0.1:${port}`);let settings=null,last=0;
 const send=(event,payload,id)=>ws.send(JSON.stringify({event,context:uuid,payload,id}));
 ws.onopen=()=>{ws.send(JSON.stringify({event,uuid}));send('getSettings',{},'initial');};
 ws.onmessage=event=>{const message=JSON.parse(event.data);if(message.event==='didReceiveSettings')settings=message.payload.settings;
 fetch(__ENDPOINT__,{method:'POST',headers:{'Content-Type':'text/plain'},body:JSON.stringify({context,event:message.event,id:message.id,payload:message.payload})}).catch(()=>{});};
 setInterval(async()=>{if(ws.readyState!==1||settings===null)return;try{
  const command=await(await fetch(__ENDPOINT__,{cache:'no-store'})).json();if(command.revision<=last)return;last=command.revision;
  send('setSettings',{...settings,incrementBy:command.incrementBy});
  send('setGlobalSettings',{counterMarker:command.revision});
  send('getSettings',{},'settings-'+last);send('getGlobalSettings',{},'globals-'+last);
 }catch{}},100);
};</script>'''.replace('__ENDPOINT__',json.dumps(endpoint))
        (root/'owned-inspector.html').write_text(html)
    def edit(self,host,context,value):
        revision=self.command['revision']+1
        self.command={'revision':revision,'incrementBy':value}
        wait(host,lambda:any(e.get('context')==context and e.get('id')=='settings-'+str(revision) and e.get('payload',{}).get('settings',{}).get('incrementBy')==value for e in self.events),'existing Counter PI settings roundtrip')
        wait(host,lambda:any(e.get('context')==context and e.get('id')=='globals-'+str(revision) and e.get('payload',{}).get('settings',{}).get('counterMarker')==revision for e in self.events),'existing Counter PI global ownership roundtrip')
        return revision
    def close(self):
        self.server.shutdown();self.server.server_close();self.thread.join(timeout=2)


def main():
    project=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/debug/deckforge')
    p.add_argument('--report',type=Path,required=True)
    args=p.parse_args();binary=args.binary.resolve(strict=True)
    sample=project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'Pinned SDK dependencies changed')
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-uninstall-') as d:
        root=Path(d);state=root/'state';original,events=probe(root,sample)
        # Only synthetic globals, deliberately separate from private marker data.
        with (original/'plugin.mjs').open('a') as f:
            f.write("\nws.on('open',()=>ws.send(JSON.stringify({event:'setGlobalSettings',context:args['-pluginUUID'],payload:{retainedGlobal:99}})));\n")
        package=root/'original.streamDeckPlugin'
        with zipfile.ZipFile(package,'w',zipfile.ZIP_DEFLATED) as archive:
            for file in original.rglob('*'):
                if file.is_file():archive.write(file,original.name+'/'+file.relative_to(original).as_posix())
        subprocess.run([str(binary),'install',str(package),'--destination',str(state/'plugins')],check=True,capture_output=True,text=True)
        managed=state/'plugins'/original.name
        counter=root/'counter.sdPlugin';shutil.copytree(sample/'com.elgato.counter.sdPlugin',counter)
        before_hash=hashlib.sha256((counter/'bin/plugin.js').read_bytes()).hexdigest()
        pi=CounterInspector(counter)
        manifest=json.loads((counter/'manifest.json').read_text())
        manifest['Actions'][0]['PropertyInspectorPath']='owned-inspector.html'
        (counter/'manifest.json').write_text(json.dumps(manifest))
        host=Host(binary,[managed,counter],state,root/'host.log')
        try:
            try:
                host.ready();host.wait(lambda s:len(s['plugins'])==2 and all(p['connected'] for p in s['plugins']),'A then Counter registration')
                require([p['id'] for p in host.state()['plugins']]==[PLUGIN,'com.elgato.counter'],'Fixture must remove first of two plugins')
                a=host.state()['device'];b='uninstall-mini'
                first=assign(host,a,0,ACTION,{'keep':'first'});assign(host,a,1,COUNTER,{'count':4,'incrementBy':1})
                host.post({'command':'device_add','device':b,'model':'mini'})
                second=assign(host,b,0,ACTION,{'keep':'second'});counter_context=assign(host,b,1,COUNTER,{'count':8,'incrementBy':1})
                multi=assign(host,a,2,ROUTINE)
                post(host,a,'multi_save',context=multi,raw={'Actions':[{'Actions':[{'UUID':DELAY,'ActionID':'delay','Settings':{'delay':1000},'States':[{}]},{'UUID':ACTION,'ActionID':'after-delay','Settings':{'keep':'queued'},'States':[{}]}]}]})
                children=child_pids(host);post(host,a,'open_inspector',context=first)
                wait(host,lambda:any(r['kind']=='sendToPlugin' and r.get('context')==first for r in rows(events)),'target native PI ready')
                target_pi=child_pids(host)-children;require(target_pi,'Target inspector process missing')
                children=child_pids(host);post(host,b,'open_inspector',context=counter_context)
                wait(host,lambda:any(e.get('id')=='initial' and e.get('context')==counter_context for e in pi.events),'Counter native PI ready')
                counter_pi=child_pids(host)-children;require(counter_pi,'Counter inspector process missing')
                pi.edit(host,counter_context,2)
                host.post({'command':'device_select','device':b})
                before=identities(host,[a,b]);old=plugin(host);counter_pid=plugin(host,'com.elgato.counter')['pid']
                registration=next(r['registration'] for r in rows(events) if r['kind']=='registered' and r['pid']==old['pid'])
                host.post({'command':'uninstall_plugin','id':PLUGIN,'expected_path':old['path'],'expected_enabled':old['enabled'] is not False,'confirm':False},expected=400)
                require(plugin(host)['pid']==old['pid'] and managed.exists(),'Unconfirmed removal disrupted installed plugin')
                host.post({'command':'uninstall_plugin','id':PLUGIN,'expected_path':old['path']+'/stale','expected_enabled':old['enabled'] is not False,'confirm':True},expected=400)
                require(plugin(host)['pid']==old['pid'] and managed.exists(),'Stale path removal disrupted installed plugin')
                private=state/'plugin-data'/hashlib.sha256(PLUGIN.encode()).hexdigest();private.mkdir(parents=True,exist_ok=True)
                marker=private/'retained-private.json';marker.write_text('{"fixture":42}');marker.chmod(0o600)
                post(host,a,'input',event='keyDown',index=0)
                wait(host,lambda:any(r['kind']=='keyDown' and r.get('context')==first for r in rows(events)),'target owns held input')
                press(host,a,2)
                wait(host,lambda:next(s for s in inspect(host,a)['slots'] if s['context']==multi)['multi_running'],'target Multi pending')
                host.post({'command':'uninstall_plugin','id':PLUGIN,'expected_path':old['path'],'expected_enabled':old['enabled'] is not False,'confirm':True})
                require([p['id'] for p in host.state()['plugins']]==['com.elgato.counter'],'Wrong plugin removed')
                require(plugin(host,'com.elgato.counter')['pid']==counter_pid,'Second plugin restarted after first removed')
                require(not Path(f'/proc/{old["pid"]}').exists(),'Uninstalled child not reaped')
                require(all(not Path(f'/proc/{pid}').exists() for pid in target_pi),'Target PI process leaked')
                require(all(Path(f'/proc/{pid}').exists() for pid in counter_pi),'Remaining native PI was replaced/closed')
                require(identities(host,[a,b])==before,'Uninstall changed action identities/settings')
                require(not any(action['plugin']==PLUGIN for action in host.state()['actions']),'Removed actions remain assignable in action library')
                require(all(next(s for s in inspect(host,device)['slots'] if s['context']==context)['available'] is False for device,context in [(a,first),(b,second)]),'Retained assignments do not expose missing-plugin state')
                require(host.state()['device']==b,'Uninstall changed selected device')
                require(all(not s['multi_running'] for s in inspect(host,a)['slots']),'Uninstall left affected Multi running')
                require(not any(i['context']==first for i in inspect(host,a)['inspectors']),'Target PI binding remains')
                require(any(i['context']==counter_context and i['connected'] for i in inspect(host,b)['inspectors']),'Remaining PI binding lost')
                revision=pi.edit(host,counter_context,3)
                press(host,b,1)
                wait(host,lambda:next(s for s in inspect(host,b)['slots'] if s['context']==counter_context)['settings']['count']==11,'Counter old socket handles input after reindex')
                require(plugin(host,'com.elgato.counter')['pid']==counter_pid,'Remaining child PID changed after callback')
                require(all(Path(f'/proc/{pid}').exists() for pid in counter_pi),'Remaining inspector PID changed after callback')
                require(not managed.exists(),'Managed installation not archived')
                archives=list((state/'removed-plugins').glob('*/'+managed.name));require(len(archives)==1,'Managed archive not found uniquely')
                archive=archives[0];require((archive/'manifest.json').read_bytes()==(original/'manifest.json').read_bytes(),'Archived manifest changed')
                require((archive/'plugin.mjs').read_bytes()==(original/'plugin.mjs').read_bytes(),'Archived runtime changed')
                require(marker.read_text()=='{"fixture":42}','Private plugin data deleted')
                saved=json.loads((state/'session.json').read_text());require(str(managed) not in saved['plugins'],'Saved plugin list retains removed installation')
                require(saved['globals'][PLUGIN]['retainedGlobal']==99,'Removed plugin global settings deleted')
                require(saved['globals']['com.elgato.counter']['counterMarker']==revision,'PI global settings routed to wrong shifted index')
                reject_old_registration(host,sample,registration)
                require('stalePoison' not in json.dumps(json.loads((state/'session.json').read_text()).get('globals',{})),'Old removed session wrote globals')
                post(host,a,'input',event='keyUp',index=0)
                retained=identities(host,[a,b]);offset=len(rows(events))
            except Exception:
                print(host.log.read_text()[-6000:]);raise
            finally:host.stop()
            host=Host(binary,[],state,root/'restart.log')
            try:
                host.ready();host.wait(lambda s:len(s['plugins'])==1 and s['plugins'][0]['connected'],'only remaining plugin restarts')
                require(host.state()['plugins'][0]['id']=='com.elgato.counter','Removed plugin restarted')
                require(len(rows(events))==offset,'Removed child ran on host restart')
                require(identities(host,[a,b])==retained,'Removed assignments lost on restart')
                host.post({'command':'install_plugin','path':str(package)})
                host.wait(lambda s:any(p['id']==PLUGIN and p['connected'] for p in s['plugins']),'reinstalled target registration')
                new=plugin(host);require(new['pid']!=old['pid'],'Reinstall reused old process')
                wait(host,lambda:all(any(r['kind']=='willAppear' and r['pid']==new['pid'] and r.get('context')==ctx and r.get('payload',{}).get('settings',{}).get('keep')==value for r in rows(events)) for ctx,value in [(first,'first'),(second,'second')]),'reinstall restores both assigned contexts/settings')
                time.sleep(1.1)
                require(not any(r['pid']==new['pid'] and r['kind'] in ('keyDown','keyUp') for r in rows(events)),'Reinstall replayed cancelled input or Multi callback')
                require(all(next(s for s in inspect(host,device)['slots'] if s['context']==context)['available'] is True for device,context in [(a,first),(b,second)]),'Reinstalled assigned actions remain unavailable')
                require(marker.read_text()=='{"fixture":42}','Reinstall lost private retained data')
                require(archive.exists(),'Reinstall destroyed recovery archive')
                require(hashlib.sha256((counter/'bin/plugin.js').read_bytes()).hexdigest()==before_hash,'Official Counter runtime changed')
                report={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'confirmation_and_expected_path_required':True,'removed_first_of_two':True,'remaining_counter_socket_pid_and_native_pi_continued':True,'two_device_settings_contexts_retained':True,'held_multi_and_target_pi_cleaned':True,'managed_install_archived':True,'globals_private_data_retained':True,'old_registration_blocked':True,'no_removed_restart':True,'reinstall_restores_actions':True,'missing_assignments_marked_unavailable_and_recovered':True,'counter_runtime_sha256':before_hash,'boundary':'Temporary simulated host; unchanged official Counter runtime with a test-owned native inspector. No external services, user state, keyboard, audio or USB. Managed path covered; external detach/save rollback covered separately by Host tests.'}
            finally:host.stop()
        finally:pi.close()
    args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: first-plugin uninstall preserves second plugin/socket/PI; cleanup, archive, persistence and reinstall verified.')

if __name__=='__main__':main()
