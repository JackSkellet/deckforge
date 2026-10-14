#!/usr/bin/env python3
"""Save Clipboard and Run command using the native editor, then verify restart."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import cleanup, environment, result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<160;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const set=(e,value)=>{const proto=e.tagName==='SELECT'?HTMLSelectElement.prototype:e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const slot=async index=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===index);
async function save(index,test){(await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b&&!b.disabled&&b},'Save')).click();return until(async()=>{const s=await slot(index);return test(s)&&s},'saved settings');}
await until(async()=>(await snapshot()).simulate,'simulation');
if(__PHASE__==='edit'){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.deckforge.system.clipboard'});
 set(await until(()=>field('Clipboard text'),'clipboard form'),'  Hei 🌍\næøå  ');
 await save(0,s=>s.settings.text==='  Hei 🌍\næøå  ');
 await cmd({command:'assign',controller:'Keypad',index:1,uuid:'com.deckforge.system.command'});
 (await until(()=>document.querySelector('[aria-label="Key 2: Run command"]'),'command key')).click();
 set(await until(()=>field('Program'),'program'),'/usr/bin/printf');
 await pause(50);set(field('Arguments'),"'%s' 'two words'");
 await pause(50);set(field('Working directory'),'/tmp');
 await save(1,s=>s.settings.program==='/usr/bin/printf'&&s.settings.args==="'%s' 'two words'"&&s.settings.directory==='/tmp');
 set(field('Command mode'),'shell');
 set(await until(()=>field('Shell script'),'shell form'),'printf hello | cat');
 await pause(50);
 const checkbox=[...document.querySelectorAll('input[type="checkbox"]')].find(e=>e.closest('label')?.textContent.includes('Run in terminal window'));
 checkbox.click();
 await until(()=>field('Terminal program'),'terminal form');
 await save(1,s=>s.settings.mode==='shell'&&s.settings.script==='printf hello | cat'&&s.settings.terminal===true);
}
const clipboard=await slot(0),command=await slot(1);
if(clipboard.settings.text!=='  Hei 🌍\næøå  '||command.settings.script!=='printf hello | cat'||command.settings.args!=="'%s' 'two words'"||command.settings.terminal!==true)throw Error('Settings changed across save/restart');
const awaitSnapshot=await snapshot();
if(!['com.deckforge.system.clipboard','com.deckforge.system.command'].every(id=>(awaitSnapshot.actions.find(a=>a.uuid===id)?.category==='System')))throw Error('Wrong action category');
return {phase:__PHASE__,clipboard:clipboard.settings,command:command.settings,actionsExecuted:false};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    p.add_argument('--output',type=Path,default=Path('target/os-actions/native'))
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-native-os-') as d:
        root=Path(d)
        for phase in ['edit','restart']:
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)))
            proof=root/'proof.json';proof.unlink(missing_ok=True)
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=30);report=result(proof)
                    assert code==0 and report and report.get('ok'),report
                    reports.append(report)
                finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'runs':reports,'usb_access':False,'actions_executed':False},indent=2)+'\n')
    print('PASS: native Clipboard and Run command forms, shell/terminal mode, retained program settings and restart; no OS actions executed.')

if __name__=='__main__':main()
