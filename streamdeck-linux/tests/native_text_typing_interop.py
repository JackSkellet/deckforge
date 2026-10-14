#!/usr/bin/env python3
"""Native Text mode selection/save/restart without requesting real desktop permission."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment,result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<170;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
const text='Native typing 🌍 æøå\nSecond line';
if(__PHASE__==='edit'){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 const s=await slot();await cmd({command:'settings',context:s.context,settings:{pastedText:text,isTypingMode:false,isSendingEnter:true,Future:{keep:9},Hotkey:{QTKeyCode:86,KeyModifiers:3}}});
 await until(()=>field('Text')?.value===text,'saved text');
 const mode=await until(()=>field('Text mode'),'mode');
 Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(mode,'typing');mode.dispatchEvent(new Event('change',{bubbles:true}));
 (await until(()=>{const e=document.querySelector('.inspector-footer button[type="submit"]');return e&&!e.disabled&&e},'Save')).click();
 await until(async()=>(await slot()).settings.isTypingMode===true,'typing mode saved');
}
await until(()=>field('Text mode')?.value==='typing','restored mode');
const s=await slot();
if(s.settings.pastedText!==text||s.settings.isSendingEnter!==true||s.settings.Future.keep!==9||s.settings.Hotkey.KeyModifiers!==3)throw Error('Settings were not preserved');
await until(()=>button('Enable desktop typing'),'permission control');
if((await snapshot()).typing_status.phase!=='disabled')throw Error('Loading an action unexpectedly requested keyboard permission');
if(!button('Test press').disabled)throw Error('Editor Test press can type into its own controls');
document.querySelector('[aria-label="Desktop typing access"]').scrollIntoView({block:'center'});
return {phase:__PHASE__,settings:s.settings,permissionExplicit:true,selfTypingTestDisabled:true,actionsExecuted:false};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/text-typing/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-native-typing-') as d:
        root=Path(d)
        for phase in ['edit','restart']:
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)));proof=root/'proof.json';proof.unlink(missing_ok=True);env=environment(root)
            env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=30);report=result(proof);assert code==0 and report and report.get('ok'),report;reports.append(report)
                finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'reports':reports,'actions_executed':False,'usb_access':False},indent=2)+'\n')
    print('PASS: native typing mode, preserved Unicode/unknown/custom-shortcut fields, Save/restart, explicit permission and editor self-typing guard. No OS actions executed.')

if __name__=='__main__':main()
