#!/usr/bin/env python3
"""Real WebKit Coalesce control, saved settings and restart. No keys or USB."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import cleanup, environment

SCRIPT = r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge, snapshot=()=>b.request('snapshot'), cmd=p=>b.request('command',p);
if(!(await snapshot()).simulate)throw Error('Simulation required');
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Encoder'&&s.index===0);
const initial=await slot();
if(initial?.settings?.Future==='coalesce-fixture'){
 if(initial.settings.Coalesce!==true)throw Error('Saved Coalesce lost on restart');
 return {phase:'restored',settings:initial.settings};
}
await cmd({command:'assign',controller:'Encoder',index:0,uuid:'com.elgato.streamdeck.system.hotkey'});
const assigned=await until(async()=>{const s=await slot();return s?.uuid.endsWith('.system.hotkey')&&s},'assigned dial');
const settings={...assigned.settings,Coalesce:false,Future:'coalesce-fixture'};
await cmd({command:'settings',context:assigned.context,settings});
(await until(()=>document.querySelector('[aria-label="Dial 1: Hotkey"]'),'dial button')).click();
const control=await until(()=>[...document.querySelectorAll('label')].find(l=>l.textContent.trim()==='Limit queued dial shortcuts')?.querySelector('input'),'Coalesce checkbox');
if(control.checked)throw Error('Explicit false was not loaded');
control.click();
const save=await until(()=>{const e=document.querySelector('.inspector-footer button[type="submit"]');return e&&!e.disabled&&e},'Save');save.click();
const saved=await until(async()=>{const s=await slot();return s.settings.Coalesce===true&&s},'saved Coalesce');
if(saved.settings.Future!=='coalesce-fixture'||JSON.stringify(saved.settings.Hotkeys)!==JSON.stringify(settings.Hotkeys))throw Error('Other settings changed');
return {phase:'saved',settings:saved.settings};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    p.add_argument('--output',type=Path,default=Path('target/hotkey-coalesce'))
    a=p.parse_args(); a.output.mkdir(parents=True,exist_ok=True)
    report=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-coalesce-') as d:
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
    print('PASS: native dial queue checkbox, save, unchanged chords/unknown settings, restart. No USB or keyboard input.')

if __name__=='__main__':main()
