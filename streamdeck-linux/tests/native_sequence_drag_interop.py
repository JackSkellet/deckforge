#!/usr/bin/env python3
"""Library-to-Multi-Action drops, insertion, drag reorder, Save and restart in native WebKit."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const phase=__PHASE__,pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<180;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const MULTI='com.elgato.streamdeck.multiactions.routine';
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const list=()=>document.querySelector('.sequence-list'),rows=()=>[...list().querySelectorAll(':scope > .sequence-row')],zone=()=>list()?.querySelector(':scope > .sequence-drop-target');
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,textarea');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));};
async function save(order){(await until(()=>{const e=document.querySelector('.inspector-footer button[type="submit"]');return e&&!e.disabled&&e},'Save')).click();const p=await until(async()=>{const p=await slot();return JSON.stringify(p.raw.Actions[0].Actions.map(a=>a.Name))===JSON.stringify(order)&&p},'saved order');await until(()=>{const button=document.querySelector('.inspector-footer button[type="submit"]');return button?.disabled&&button.textContent==='Save changes'&&!document.querySelector('.inspector-fields')?.disabled},'save completed');return p;}
function deliver(data,target,before=false){target.scrollIntoView({block:'center'});const rect=target.getBoundingClientRect();const options={bubbles:true,cancelable:true,dataTransfer:data,clientX:rect.left+20,clientY:before?rect.top+2:rect.bottom-2};target.dispatchEvent(new DragEvent('dragover',options));target.dispatchEvent(new DragEvent('drop',options));}
async function library(name,target,before=false){const source=await until(()=>[...document.querySelectorAll('.library-action')].find(e=>e.querySelector('strong')?.textContent===name),'library '+name);const data=new DataTransfer();source.dispatchEvent(new DragEvent('dragstart',{bubbles:true,cancelable:true,dataTransfer:data}));if(!data.getData('application/x-deckforge-action')||!data.getData('text/plain'))throw Error('Library drag payload missing');deliver(data,target,before);source.dispatchEvent(new DragEvent('dragend',{bubbles:true,dataTransfer:data}));await pause(100);}
if(!(await snapshot()).simulate)throw Error('Private simulation required');
if(phase==='empty'){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:MULTI});await until(()=>zone()?.textContent.includes('Drag actions here'),'empty drop area');zone().scrollIntoView({block:'center'});return {phase,empty_drop_area:true};
}
await until(()=>list(),'sequence editor');
if(phase==='restart'){
 const p=await slot();if(JSON.stringify(p.raw.Actions[0].Actions.map(a=>a.Name))!==JSON.stringify(['Open','Message','Website']))throw Error('Order lost on restart');
 if(p.raw.Actions[0].Actions[1].Settings.pastedText!=='Keep this value')throw Error('Child settings lost');
 list().scrollIntoView({block:'start'});return {phase,order:p.raw.Actions[0].Actions.map(a=>a.Name),child_settings_preserved:true};
}
await library('Website',zone());await until(()=>rows().length===1,'first drop');
await library('Text',rows()[0],true);await until(()=>rows().length===2,'insert before');
set(await until(()=>field('Action label'),'child label'),'Message');await pause(80);set(field('Text'),'Keep this value');await pause(80);
await library('Open',zone());await until(()=>rows().length===3,'append');
if((await slot()).multi_children[0].length!==0)throw Error('Drop persisted before Save');
const saved=await save(['Message','Website','Open']),original=saved.raw.Actions[0].Actions;
if(new Set(original.map(a=>a.ActionID)).size!==3)throw Error('New actions reused identity');
const handle=rows()[2].querySelector('.sequence-drag'),data=new DataTransfer();handle.dispatchEvent(new DragEvent('dragstart',{bubbles:true,cancelable:true,dataTransfer:data}));
if(!data.getData('application/x-deckforge-sequence-step')||data.getData('application/x-deckforge-action'))throw Error('Move payload confused with library copy');
deliver(data,rows()[0],true);handle.dispatchEvent(new DragEvent('dragend',{bubbles:true,dataTransfer:data}));await pause(100);
const reordered=await save(['Open','Message','Website']),expected=[original[2],original[0],original[1]];
if(JSON.stringify(reordered.raw.Actions[0].Actions)!==JSON.stringify(expected))throw Error('Reorder changed action data or identities');
list().scrollIntoView({block:'start'});await pause(80);return {phase,library_drop:true,insert_before:true,reorder:true,identities_and_settings_preserved:true,explicit_save:true};
'''
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/sequence-drag/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);runs=[]
 with tempfile.TemporaryDirectory(prefix='deckforge-sequence-drag-') as d:
  root=Path(d)
  for phase in ['empty','edit','restart']:
   script=root/'probe.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)));proof=root/(phase+'.json');env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
   with (a.output/(phase+'.log')).open('w') as log:
    proc=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     code=proc.wait(timeout=45);data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No report'};assert code==0 and data.get('ok'),data;assert data['result']['phase']==phase;runs.append(data['result'])
    finally:cleanup(proc)
 (a.output/'proof.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':runs,'input':'native WebKit DragEvent/DataTransfer','actions_executed':False,'usb_access':False},indent=2)+'\n');print('PASS: native library drag payload, Multi Action insertion/reorder, explicit Save, unchanged identities/settings and restart. No actions executed.')
if __name__=='__main__':main()
