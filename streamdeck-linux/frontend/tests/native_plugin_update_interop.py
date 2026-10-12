#!/usr/bin/env python3
"""Managed update API plus actual native rollback review buttons, all temporary.

Package chooser interaction is covered by frontend unit tests. This fixture uses
inert loopback-only plugin backends and does not access USB/user installations.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

PROJECT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(PROJECT/'tests'))
from native_editor_interop import cleanup, environment, result
ID='com.deckforge.update-probe'
OTHER='com.deckforge.update-survivor'
SCRIPT=r'''
const expected=__EXPECTED__,bridge=window.deckforge;
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,name)=>{for(let i=0;i<220;i++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const read=()=>bridge.request('snapshot'),plugin=(s,id)=>s.plugins.find(p=>p.id===id);
const initial=await until(async()=>{const s=await read();if(!s.simulate)throw Error('Simulation required');return plugin(s,expected.id)?.connected&&plugin(s,expected.other)?.connected&&s},'owned plugin registration');
const survivorPid=plugin(initial,expected.other).pid;
const cmd=payload=>bridge.request('command',{...payload,device:initial.device});
await cmd({command:'assign',controller:'Keypad',index:0,uuid:expected.id+'.keep'});
await cmd({command:'assign',controller:'Keypad',index:1,uuid:expected.id+'.old'});
const slots=await until(async()=>{const s=await read();return s.slots.some(v=>v.uuid===expected.id+'.old')&&s.slots},'owned action assignments');
const keep=slots.find(v=>v.uuid===expected.id+'.keep'),old=slots.find(v=>v.uuid===expected.id+'.old');
await cmd({command:'settings',context:keep.context,settings:{foreign:{preserved:true}}});
await cmd({command:'settings',context:old.context,settings:{removed:'retained'}});
document.querySelector('.deck-key').click();
const title=()=>document.querySelector('textarea[aria-label="Title"]');
await until(()=>title(),'title editor');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(title(),'Unsaved update draft');title().dispatchEvent(new Event('input',{bubbles:true}));title().dispatchEvent(new Event('change',{bubbles:true}));
await until(async()=>(await bridge.request('ui_state_get'))?.drafts?.[keep.context]?.dirty,'retained update draft');
await bridge.request('command',{command:'plugin_enabled',id:expected.id,enabled:false,expected_enabled:true});
const disabled=await until(async()=>{const s=await read();return plugin(s,expected.id)?.enabled===false&&!plugin(s,expected.id).pid&&s},'disabled update target');
const prepared=await bridge.request('plugin_update_prepare',{id:expected.id,path:expected.package,expected_path:plugin(disabled,expected.id).path,expected_version:'1.0.0'});
if(prepared.comparison!=='upgrade'||prepared.was_enabled||!prepared.removed_actions.includes(expected.id+'.old')||!prepared.added_actions.includes(expected.id+'.new'))throw Error('Update review contract incorrect');
if(plugin(await read(),expected.id).version!=='1.0.0')throw Error('Prepare changed installed version');
await bridge.request('command',{command:'plugin_update_apply',ticket:prepared.ticket,confirm:true});
const updated=await until(async()=>{const s=await read();return plugin(s,expected.id)?.version==='2.0.0'&&s},'actual package update');
if(plugin(updated,expected.id).enabled!==false||plugin(updated,expected.id).pid)throw Error('Update enabled disabled plugin');
if(plugin(updated,expected.other).pid!==survivorPid||!plugin(updated,expected.other).connected)throw Error('Update disrupted other plugin');
if(updated.slots.find(v=>v.context===old.context)?.available!==false||updated.slots.find(v=>v.context===old.context)?.settings.removed!=='retained')throw Error('Removed action lost preserved assignment');
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
[...document.querySelectorAll('.header-tools button')].find(b=>b.textContent.trim().startsWith('Plugins')).click();
const row=()=>[...document.querySelectorAll('.plugin-row')].find(row=>row.textContent.includes(expected.id));
const restore=()=>row()&&[...row().querySelectorAll('button')].find(b=>b.textContent.trim().startsWith('Restore version'));
await until(()=>restore()&&!restore().disabled,'native rollback button');restore().click();
await until(()=>button('Confirm restore'),'native rollback review');
if(!row().textContent.includes('2.0.0 → 1.0.0')||!row().textContent.includes('older version')||!row().textContent.includes('remain disabled'))throw Error('Native rollback review missing version/state boundaries');
button('Cancel review').click();await until(()=>!button('Confirm restore'),'native review cancel');
if(plugin(await read(),expected.id).version!=='2.0.0')throw Error('Cancelling review changed installed version');
restore().click();await until(()=>button('Confirm restore'),'second native rollback review');button('Confirm restore').click();
const restored=await until(async()=>{const s=await read();return plugin(s,expected.id)?.version==='1.0.0'&&s},'native confirmed rollback');
if(plugin(restored,expected.id).enabled!==false||plugin(restored,expected.id).pid)throw Error('Rollback enabled disabled plugin');
if(!restored.actions.some(a=>a.uuid===expected.id+'.old')||restored.actions.some(a=>a.uuid===expected.id+'.new'))throw Error('Rollback did not restore action definitions');
if(plugin(restored,expected.other).pid!==survivorPid||!plugin(restored,expected.other).connected)throw Error('Rollback disrupted survivor');
document.querySelector('[aria-label="Close Plugins"]').click();await until(()=>title()?.value==='Unsaved update draft','draft after update/rollback');
const final=await read();if(!final.slots.find(s=>s.context===keep.context)?.settings.foreign?.preserved)throw Error('Update lost saved action settings');
// Capture a same-version package review is covered in unit tests; here leave
// the real restored runtime and native plugin controls visible.
[...document.querySelectorAll('.header-tools button')].find(b=>b.textContent.trim().startsWith('Plugins')).click();
await until(()=>row()?.textContent.includes('1.0.0'),'restored plugin presentation');
return{prepared,update_applied:true,actual_rollback_prepare_cancel_confirm:true,disabled_stays_off:true,other_plugin_pid:survivorPid,other_plugin_unchanged:true,removed_assignment_retained:true,draft_preserved:true,final_version:plugin(final,expected.id).version,picker_button_exercised:false,usb_access:false};
'''

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binary',type=Path,default=PROJECT/'target/release/deckforge');ap.add_argument('--output',type=Path,required=True);args=ap.parse_args()
    binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    report={'ok':False,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix='deckforge-native-update-') as temporary:
        root=Path(temporary);state=root/'state';managed=state/'plugins'/f'{ID}.sdPlugin';other=root/'external'/f'{OTHER}.sdPlugin';managed.mkdir(parents=True);other.mkdir(parents=True)
        backend="import WebSocket from "+json.dumps((PROJECT/'frontend/node_modules/ws/wrapper.mjs').as_uri())+";const args=process.argv.slice(2),get=name=>args[args.indexOf(name)+1];const ws=new WebSocket('ws://127.0.0.1:'+get('-port'));ws.on('open',()=>ws.send(JSON.stringify({event:get('-registerEvent'),uuid:get('-pluginUUID')})));ws.on('message',data=>{const m=JSON.parse(data);if(m.event==='willAppear')ws.send(JSON.stringify({event:'setTitle',action:m.action,context:m.context,payload:{title:'Fixture',target:0}}));});"
        def manifest(identity,version,extra):
            return {'UUID':identity,'Name':'Update probe' if identity==ID else 'Survivor','Version':version,'SDKVersion':2,'Software':{'MinimumVersion':'7.1'},'Nodejs':{'Version':'20'},'CodePath':'backend.mjs','Actions':[{'UUID':identity+'.'+name,'Name':name.title()+' action','Controllers':['Keypad'],'States':[{}]} for name in ['keep']+extra]}
        for folder,identity,extra in ((managed,ID,['old']),(other,OTHER,[])):
            (folder/'manifest.json').write_text(json.dumps(manifest(identity,'1.0.0',extra)));(folder/'backend.mjs').write_text(backend)
        package=root/'update.streamDeckPlugin'
        with zipfile.ZipFile(package,'w') as archive:
            archive.writestr(f'{ID}.sdPlugin/manifest.json',json.dumps(manifest(ID,'2.0.0',['new'])));archive.writestr(f'{ID}.sdPlugin/backend.mjs',backend)
        private=state/'plugin-data'/hashlib.sha256(ID.encode()).hexdigest();private.mkdir(parents=True);private.chmod(0o700);sentinel=private/'retained.txt';sentinel.write_text('private fixture')
        script,proof=root/'fixture.js',root/'proof.json';script.write_text(SCRIPT.replace('__EXPECTED__',json.dumps({'id':ID,'other':OTHER,'package':str(package)})))
        env=environment(root);env.update(DECKFORGE_CAPTURE=str(output/'update.png'),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path=output/'update.log'
        with log_path.open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(state),'--plugin',str(managed),'--plugin',str(other)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=50)==0,log_path.read_text();value=result(proof);assert value,log_path.read_text();assert json.loads((managed/'manifest.json').read_text())['Version']=='1.0.0';assert sentinel.read_text()=='private fixture';assert json.loads((other/'manifest.json').read_text())['Version']=='1.0.0';report.update(ok=True,native=value['result'],private_data_retained=True,restored_files_verified=True)
            except Exception as error:report['error']=str(error);raise
            finally:cleanup(process);(output/'update.json').write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: native update API and actual rollback review/cancel/confirm, disabled-state/draft/profile/private-data retention; isolated state only.')
if __name__=='__main__':main()
