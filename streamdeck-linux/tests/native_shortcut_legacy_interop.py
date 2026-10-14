#!/usr/bin/env python3
"""Legacy modifier interpretation and Text paste-shortcut editing in native WebKit."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<150;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const bridge=window.deckforge,snapshot=()=>bridge.request('snapshot'),cmd=p=>bridge.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const check=name=>[...document.querySelectorAll('label')].find(l=>l.textContent.trim()===name)?.querySelector('input[type="checkbox"]');
const set=(e,value)=>{const proto=e.tagName==='SELECT'?HTMLSelectElement.prototype:e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const openPaste=async()=>{const summary=await until(()=>[...document.querySelectorAll('summary')].find(e=>e.textContent==='Paste shortcut'),'paste controls');if(!summary.parentElement.open)summary.click();await pause(60);};
async function save(test){(await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b&&!b.disabled&&b},'Save')).click();const saved=await until(async()=>{const s=await slot();return test(s.settings)&&s},'saved settings');await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b?.disabled&&b.textContent==='Save changes'},'save complete');return saved.settings;}
if(!(await snapshot()).simulate)throw Error('Simulation required');
const initial=await slot();
if(initial?.uuid?.endsWith('.text')&&initial.settings.pastedText==='Terminal 🌍'){
 if(initial.settings.Hotkey.KeyModifiers!==3||initial.settings.Hotkey.linuxKeyMode!=='logical')throw Error('Restart lost paste shortcut');
 await openPaste();if(!check('Ctrl').checked||!check('Shift').checked||field('Key').selectedOptions[0].textContent!=='V')throw Error('Restart form mismatch');
 field('Key').scrollIntoView({block:'center'});await pause(80);return {phase:'restored',settings:initial.settings};
}
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.hotkey'});
let action=await until(async()=>{const s=await slot();return s?.uuid?.endsWith('.hotkey')&&s},'Hotkey');
await cmd({command:'settings',context:action.context,settings:{Hotkeys:[{QTKeyCode:65,KeyModifiers:8192,KeyCtrl:true,Future:9}],Future:7}});
await until(()=>check('Ctrl')?.checked&&field('Key')?.selectedOptions[0]?.textContent==='A'&&[...document.querySelectorAll('textarea')].some(e=>e.value.includes('8192')),'legacy rendered');
check('Shift').click();const legacy=await save(s=>s.Hotkeys[0].KeyModifiers===3&&s.Hotkeys[0].KeyShift===true);
if(legacy.Hotkeys[0].Future!==9||legacy.Future!==7)throw Error('Legacy metadata lost');
action=await slot();await cmd({command:'settings',context:action.context,settings:{Hotkeys:[{QTKeyCode:65,KeyModifiers:2,KeyCtrl:false,KeyShift:true,Future:10}],Future:8}});
await until(()=>check('Ctrl')?.checked&&!check('Shift')?.checked&&[...document.querySelectorAll('textarea')].some(e=>e.value.includes('10')),'modern precedence rendered');
set(field('Key mapping'),'logical');const modern=await save(s=>s.Hotkeys[0].linuxKeyMode==='logical');
if(modern.Hotkeys[0].KeyCtrl!==false||modern.Hotkeys[0].KeyShift!==true||modern.Hotkeys[0].KeyModifiers!==2)throw Error('Dormant booleans rewritten');
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
action=await until(async()=>{const s=await slot();return s?.uuid?.endsWith('.text')&&s},'Text');if(action.settings.linuxKeyMode!=='logical'||action.settings.Hotkey!==undefined)throw Error('New text defaults incorrect');
set(await until(()=>field('Text'),'Text input'),'Terminal 🌍');await pause(60);await openPaste();
if(!check('Ctrl').checked||field('Key mapping').value!=='logical'||field('Key').selectedOptions[0].textContent!=='V')throw Error('Default paste form incorrect');
check('Shift').click();const paste=await save(s=>s.Hotkey?.KeyModifiers===3&&s.pastedText==='Terminal 🌍');
set(field('Text mode'),'typing');await save(s=>s.isTypingMode===true);if([...document.querySelectorAll('summary')].some(e=>e.textContent==='Paste shortcut'))throw Error('Dormant paste editor shown in typing mode');
set(field('Text mode'),'paste');const restored=await save(s=>s.isTypingMode===false);if(JSON.stringify(restored.Hotkey)!==JSON.stringify(paste.Hotkey))throw Error('Typing transition lost shortcut');
await openPaste();[...document.querySelectorAll('button')].find(e=>e.textContent==='Reset paste shortcut').click();await save(s=>s.Hotkey===undefined);
if(!check('Ctrl').checked||check('Shift').checked||field('Key mapping').value!=='logical')throw Error('Reset did not restore standard paste');
check('Shift').click();const saved=await save(s=>s.Hotkey?.KeyModifiers===3);field('Key').scrollIntoView({block:'center'});await pause(80);
return {phase:'saved',legacy,modern,settings:saved,typing_retains_custom:true,reset_verified:true};
'''
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/shortcut-legacy/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);runs=[]
 with tempfile.TemporaryDirectory(prefix='deckforge-shortcut-native-') as d:
  root=Path(d);script=root/'probe.js';script.write_text(SCRIPT)
  for phase in ['saved','restored']:
   proof=root/(phase+'.json');env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
   with (a.output/(phase+'.log')).open('w') as log:
    proc=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     code=proc.wait(timeout=45);data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No native report'};assert code==0 and data.get('ok'),data;assert data['result']['phase']==phase,data;runs.append(data['result'])
    finally:cleanup(proc)
 (a.output/'proof.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':runs,'actions_executed':False},indent=2)+'\n');print('PASS: native legacy modifier interpretation, modern-mask precedence, Text paste shortcut editor/reset, typing preservation and restart. No input actions executed.')
if __name__=='__main__':main()
