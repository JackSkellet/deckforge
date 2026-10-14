#!/usr/bin/env python3
"""Save the new system-action forms through the real native WebKit editor."""
import argparse,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment,result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<150;i++){const v=await f();if(v)return v;await pause(40);}throw Error('Timed out: '+name)};
const bridge=window.deckforge, snapshot=()=>bridge.request('snapshot'),cmd=p=>bridge.request('command',p);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const set=(e,value)=>{const proto=e.tagName==='SELECT'?HTMLSelectElement.prototype:e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,String(value));e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const slot=()=>snapshot().then(s=>s.slots.find(x=>x.controller==='Keypad'&&x.index===0));
async function assign(name){const uuid='com.elgato.streamdeck.system.'+name;await cmd({command:'assign',controller:'Keypad',index:0,uuid});await until(async()=>{const s=await slot();return s?.uuid===uuid},name);}
async function save(test){const button=await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b&&!b.disabled&&b},'Save');button.click();return until(async()=>{const s=await slot();return test(s)&&s},'saved settings');}
await until(async()=>(await snapshot()).simulate===true,'simulated host');
await assign('text');set(await until(()=>field('Text'),'text control'),'Hei 🌍\næøå');
const text=await save(s=>s.settings.pastedText==='Hei 🌍\næøå');
await assign('hotkeyswitch');await until(()=>document.querySelectorAll('.settings-fields fieldset').length===2,'two shortcut fields');
const selects=[...document.querySelectorAll('.settings-fields label.field')].filter(l=>l.querySelector('span')?.textContent==='Key').map(l=>l.querySelector('select'));set(selects[1],0);
const switchState=await save(s=>s.settings.Hotkeys[1].QTKeyCode===65);
await assign('close');set(await until(()=>field('Application executable'),'close control'),'/not-executed/owned-app');
const close=await save(s=>s.settings.path==='/not-executed/owned-app');
await assign('timer');set(await until(()=>field('Duration (seconds)'),'duration'),90);await pause(50);set(field('Alarm sound'),6);
const timer=await save(s=>s.settings.duration===90&&s.settings.actionIdx===6);
await cmd({command:'assign',controller:'Encoder',index:0,uuid:'com.elgato.streamdeck.system.mouse-wheel'});
(await until(()=>document.querySelector('[aria-label="Dial 1: Mouse Wheel"]'),'wheel dial')).click();
await until(()=>document.querySelector('.settings-fields')?.textContent.includes('scroll horizontally'),'wheel guidance');
const s=await snapshot();
if(!s.actions.some(a=>a.uuid.endsWith('.multiactions.random')&&a.category==='Stream Deck'))throw Error('Random Action missing from Stream Deck category');
return {text:text.settings,switch:switchState.settings,close:close.settings,timer:timer.settings,mouseWheelPresent:true,categories:[...new Set(s.actions.map(a=>a.category))]};
'''
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/debug/deckforge'));p.add_argument('--report',type=Path,default=Path('target/system-actions-next/native.json'));a=p.parse_args();a.report.parent.mkdir(parents=True,exist_ok=True)
 with tempfile.TemporaryDirectory(prefix='deckforge-new-controls-') as d:
  root=Path(d);script=root/'probe.js';proof=root/'proof.json';script.write_text(SCRIPT);env=environment(root)
  env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.report.resolve().with_suffix('.png')))
  with (root/'log').open('w') as log:
   process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],stdout=log,stderr=subprocess.STDOUT,env=env,start_new_session=True)
   try:
    code=process.wait(timeout=35);data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':(root/'log').read_text()};a.report.write_text(json.dumps(data,indent=2)+'\n');assert code==0 and data.get('ok'),data
   finally:cleanup(process)
 print('PASS: native Text, Hotkey Switch, Close, Timer form saves, Mouse Wheel guidance and Stream Deck category. No actions executed.')
if __name__=='__main__':main()
