#!/usr/bin/env python3
"""Real WebKit Key Logic slots, nested settings, action drop, Save and restart."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import cleanup, environment

SCRIPT = r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const LOGIC='com.elgato.streamdeck.keys.logic', TIMER='com.elgato.streamdeck.system.timer', TEXT='com.elgato.streamdeck.system.text';
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
if(!(await snapshot()).simulate)throw Error('Simulation required');
const initial=await slot();
if(initial?.raw?.Future==='key-logic-native'){
 if(initial.raw.Actions[0].Settings.duration!==12||initial.raw.Actions[1].UUID!==TEXT)throw Error('Restart lost children');
 return {phase:'restored',raw:initial.raw};
}
const key=await until(()=>document.querySelector('[aria-label="Key 1: Unassigned"]'),'empty key');
key.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:400,clientY:200}));
(await until(()=>[...document.querySelectorAll('.control-edit-options button')].find(e=>e.textContent==='Create Key Logic'),'context-menu create')).click();
await until(()=>document.querySelector('[aria-label="Key logic"]'),'Key Logic editor');
const parent=await slot();await cmd({command:'logic_save',context:parent.context,raw:{...parent.raw,Future:'key-logic-native'}});
await pause(250);
const rows=()=>document.querySelectorAll('[aria-label="Key logic"] .sequence-row');
if(rows().length!==3)throw Error('Expected three gesture positions');
rows()[0].querySelector('.sequence-select').click();
const select=document.querySelector('select[aria-label="Action to add"]');
if([...select.options].some(o=>o.value===LOGIC))throw Error('Key Logic opt-out ignored');
set(select,TIMER);await pause(30);
[...document.querySelectorAll('.add-action button')].find(e=>e.textContent==='Assign').click();
set(await until(()=>field('Duration (seconds)'),'nested Timer controls'),'12');await pause(60);
const drop=new Event('drop',{bubbles:true,cancelable:true});
Object.defineProperty(drop,'dataTransfer',{value:{types:['application/x-deckforge-action'],getData:()=>TEXT}});rows()[1].dispatchEvent(drop);
await until(()=>document.querySelector('[aria-label="Key logic"]')?.textContent.includes('Text'),'dropped second action');
(await until(()=>{const e=document.querySelector('.inspector-footer button[type="submit"]');return e&&!e.disabled&&e},'Save')).click();
const saved=await until(async()=>{const s=await slot();return s.raw.Actions[0].Settings?.duration===12&&s.raw.Actions[1].UUID===TEXT&&s},'saved logic');
if(saved.uuid!==LOGIC||saved.raw.Future!=='key-logic-native'||saved.trigger_children.length!==3||saved.trigger_children[2]!==null)throw Error('Positional children changed');
return {phase:'saved',raw:saved.raw,native_nested_fields:true,native_library_drop:true};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    p.add_argument('--output',type=Path,default=Path('target/key-logic/native'))
    a=p.parse_args(); a.output.mkdir(parents=True,exist_ok=True)
    report=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-keylogic-native-') as d:
        root=Path(d); script=root/'probe.js';script.write_text(SCRIPT)
        for phase in ['saved','restored']:
            proof=root/(phase+'.json');env=environment(root)
            env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=35)
                    data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No native report'}
                    assert code==0 and data.get('ok'),data
                    assert data['result']['phase']==phase,data
                    report.append(data['result'])
                finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: native Key Logic fixed slots, nested Timer settings, library drop, eligibility, save and restart. No actions executed.')

if __name__=='__main__':main()
