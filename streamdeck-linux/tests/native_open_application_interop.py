#!/usr/bin/env python3
"""Real WebKit application search, selection, arguments, held-key settings and restart."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import cleanup, environment

SCRIPT = r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
if(!(await snapshot()).simulate)throw Error('Simulation required');
const initial=await slot();
if(initial?.settings?.app_name==='Deckforge owned picker'){
 if(initial.settings.long_press!=='force_kill'||initial.settings.args!=='"space argument"')throw Error('Restart lost settings');
 const state=await snapshot(),profile=state.library.active_profile,restored=!!state.smart_profiles.config.bindings[profile];
 const menu=document.querySelector('.sidebar-bottom details');if(!menu.open)menu.querySelector('summary').click();
 [...menu.querySelectorAll('button')].find(v=>v.textContent==='Smart Profiles…').click();
 set(await until(()=>field('Find installed application'),'Smart search'),'Deckforge themed picker');
 const radio=await until(()=>document.querySelector('.application-choice input[aria-label="Deckforge themed picker"]'),'themed application');
 radio.scrollIntoView({block:'center'});
 const icon=await until(()=>{const img=radio.closest('label').querySelector('img');return img?.complete&&img.naturalWidth===64&&img},'themed icon');
 if(restored){
   if(!radio.checked||state.smart_profiles.config.enabled)throw Error('Smart selection did not survive restart');
   return {phase:'smart-restored',smart_icon:true,theme_icon:true,settings:initial.settings};
 }
 radio.click();await pause(80);
 [...document.querySelectorAll('dialog button')].find(v=>v.textContent==='Save Smart Profiles').click();
 await until(async()=>{const c=(await snapshot()).smart_profiles.config;return c.bindings[profile]?.value==='OwnedThemedClass'&&!c.enabled},'Smart binding');
 return {phase:'restored',settings:initial.settings,smart_picker:true,theme_icon:true};
}
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.openapp'});
(await until(()=>[...document.querySelectorAll('button')].find(e=>e.textContent==='Browse applications…'),'browse')).click();
set(await until(()=>field('Search applications'),'search'),'Deckforge owned picker');
const choice=await until(()=>document.querySelector('.application-choice input[aria-label="Deckforge owned picker"]'),'catalog');
choice.scrollIntoView({block:'center'});choice.click();
await until(()=>{const img=choice.closest('label').querySelector('img');return img?.complete&&img.naturalWidth===64},'application icon');
await pause(60);set(field('Arguments'),'"space argument"');await pause(60);set(field('When held'),'force_kill');
(await until(()=>{const e=document.querySelector('.inspector-footer button[type="submit"]');return e&&!e.disabled&&e},'Save')).click();
const saved=await until(async()=>{const s=await slot();return s.settings.long_press==='force_kill'&&s.settings.app_name==='Deckforge owned picker'&&s},'saved');
if(!saved.settings.source.endsWith('.desktop')||saved.settings.exec!=='/usr/bin/true')throw Error('Application mapping incorrect');
return {phase:'saved',settings:saved.settings,catalog_search:true,application_icon:true};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    p.add_argument('--output',type=Path,default=Path('target/open-application/native'))
    a=p.parse_args(); a.output.mkdir(parents=True,exist_ok=True)
    report=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-openapp-native-') as d:
        root=Path(d); script=root/'probe.js';script.write_text(SCRIPT)
        apps=root/'data/applications';apps.mkdir(parents=True)
        icon=root/'owned.svg';icon.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="12" fill="#85d9ba"/><circle cx="32" cy="32" r="16" fill="#143329"/></svg>')
        (apps/'owned.desktop').write_text(f'[Desktop Entry]\nType=Application\nName=Deckforge owned picker\nExec=/usr/bin/true\nIcon={icon}\n')
        theme=root/'data/icons/hicolor';(theme/'64x64/apps').mkdir(parents=True)
        (theme/'index.theme').write_text('[Icon Theme]\nName=Hicolor\nDirectories=64x64/apps\n[64x64/apps]\nSize=64\nType=Fixed\nContext=Applications\n')
        (theme/'64x64/apps/deckforge-fixture.svg').write_text(icon.read_text())
        (apps/'owned-theme.desktop').write_text('[Desktop Entry]\nType=Application\nName=Deckforge themed picker\nExec=/usr/bin/true\nStartupWMClass=OwnedThemedClass\nIcon=deckforge-fixture\n')
        for phase in ['saved','restored','smart-restored']:
            proof=root/(phase+'.json');env=environment(root);env['XDG_DATA_HOME']=str(root/'data')
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
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':report,'usb_access':False,'application_launched':False},indent=2)+'\n')
    print('PASS: native application search, absolute/theme icons, Open Application settings, Smart bindings and restart. No application launched; no USB.')

if __name__=='__main__':main()
