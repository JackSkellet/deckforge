#!/usr/bin/env python3
"""Native modifier sides and restart, without executing shortcuts."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<150;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const bridge=window.deckforge,snapshot=()=>bridge.request('snapshot'),cmd=p=>bridge.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('change',{bubbles:true}));};
if(!(await snapshot()).simulate)throw Error('Simulation required');
const initial=await slot();
if(initial?.settings?.Future===7){
 const h=initial.settings.Hotkeys[0];
 if(h.KeyModifiers!==0x10626||h.QTKeyCode!==81||h.Future!==9||h.linuxKeyMode!=='logical')throw Error('Restart lost sides');
 await until(()=>field('Alt side')?.value==='2'&&field('Ctrl side')?.value==='3'&&field('Key mapping')?.value==='logical','restored sides');
 field('Key mapping').scrollIntoView({block:'center'});await pause(80);
 return {phase:'restored',settings:initial.settings};
}
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.hotkey'});
const fresh=await until(async()=>{const s=await slot();return s?.uuid?.endsWith('.hotkey')&&s},'Hotkey');
await cmd({command:'settings',context:fresh.context,settings:{Hotkeys:[{QTKeyCode:65,VKeyCode:65,KeyModifiers:0x10404,Future:9}],Future:7}});
await until(()=>field('Alt side')?.value==='2','loaded imported AltGr');
set(field('Key mapping'),'logical');await pause(80);set(field('Ctrl side'),'3');await pause(80);set(field('Key'),'16');await pause(80);
(await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b&&!b.disabled&&b},'Save')).click();
const saved=await until(async()=>{const s=await slot();return s.settings.Hotkeys[0].KeyModifiers===0x10626&&s.settings.Hotkeys[0].QTKeyCode===81&&s.settings.Hotkeys[0].linuxKeyMode==='logical'&&s},'saved sides');
await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b?.disabled&&b.textContent==='Save changes'},'save completed');
if(saved.settings.Future!==7||saved.settings.Hotkeys[0].Future!==9)throw Error('Opaque settings lost');
field('Key mapping').scrollIntoView({block:'center'});await pause(80);
return {phase:'saved',settings:saved.settings};
'''
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/logical-hotkeys/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);runs=[]
 with tempfile.TemporaryDirectory(prefix='deckforge-logical-native-') as d:
  root=Path(d);script=root/'probe.js';script.write_text(SCRIPT)
  for phase in ['saved','restored']:
   proof=root/(phase+'.json');env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
   with (a.output/(phase+'.log')).open('w') as log:
    process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     code=process.wait(timeout=35);data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No report'}
     assert code==0 and data.get('ok'),data
     assert data['result']['phase']==phase,data
     runs.append(data['result'])
    finally:cleanup(process)
 (a.output/'proof.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':runs,'actions_executed':False},indent=2)+'\n')
 print('PASS: native logical key mapping, AltGr/both Ctrl, key edit, opaque settings and restart. No shortcuts executed.')
if __name__=='__main__':main()
