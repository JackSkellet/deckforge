#!/usr/bin/env python3
"""Native profile copies and archive round trips retain Smart bindings and artwork."""
import argparse
import hashlib
import io
import json
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup, environment, result
from system_interop import Host

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot'),phase=__PHASE__,expected=__EXPECTED__;
const sameBinding=(a,b)=>a?.kind===b.kind&&a?.value===b.value&&a?.name===b.name;
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
if(phase==='archive'||phase==='archive-restart'){
 const p=s.library.profiles.find(p=>p.id===s.library.active_profile),config=s.smart_profiles.config;
 if(p?.name!=='On device B'||!sameBinding(config.bindings[p.id],{...expected.binding,value:'Independent copy'}))throw Error('Archive import lost binding');
 if(config.enabled||config.fallback!==null||Object.keys(config.bindings).length!==1)throw Error('Archive import changed destination preferences');
 const dial=s.slots.find(v=>v.controller==='Encoder'&&v.index===0);if(!dial?.raw.Encoder?.background)throw Error('Archive import lost cropped image');
 await until(()=>document.body.textContent.includes('Smart: Copy app'),'imported application label');
 await cmd({command:'export_profile',device:s.device,path:__EXPORT__});
 return {phase,archive_binding_retained:true,destination_disabled:true,artwork_retained:true,restart:phase==='archive-restart',usb_access:false};
}
if(phase==='copy'){
 const original=s.slots.find(v=>v.controller==='Encoder'&&v.index===0);
 const source=await b.request('image_crop',{mode:'load',context:original.context,path:__IMAGE__});
 await b.request('image_crop',{mode:'save',context:original.context,path:__IMAGE__,source_hash:source.source_hash,expected:null,rect:{x:0,y:0,width:200,height:100}});
 const menu=document.querySelector('.sidebar-bottom details');if(!menu.open)menu.querySelector('summary').click();
 [...menu.querySelectorAll('button')].find(v=>v.textContent==='Duplicate profile').click();
 set(await until(()=>field('Name'),'duplicate name'),'Same-device copy');await until(async()=>(await b.request('ui_state_get')).libraryEdit?.name==='Same-device copy','duplicate draft');
 [...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Save').click();
 s=await until(async()=>{const s=await snapshot();return s.library.profiles.some(p=>p.name==='Same-device copy')&&s},'duplicate');
 await until(()=>!document.querySelector('dialog[open]'),'duplicate dialog closed');
 const copied=s.library.active_profile;
 if(copied===expected.source||!sameBinding(s.smart_profiles.config.bindings[copied],expected.binding))throw Error('Duplicate lost binding');
 if(s.smart_profiles.config.fallback!==expected.source||!s.smart_profiles.config.enabled)throw Error('Duplicate changed device preferences');
 const settings=structuredClone(s.smart_profiles.config);settings.bindings[copied].value='Independent copy';
 await cmd({command:'smart_config',device:s.device,expected:s.smart_profiles.config,settings});
 const after=await snapshot();if(after.smart_profiles.config.bindings[expected.source].value!==expected.binding.value)throw Error('Copy binding aliases original');
 const options=document.querySelector('.sidebar-bottom details');if(!options.open)options.querySelector('summary').click();
 [...options.querySelectorAll('button')].find(v=>v.textContent==='Copy to device…').click();
 set(await until(()=>field('Target device'),'copy dialog'),expected.target);await pause(50);
 set(field('Copy name'),'On device B');await pause(60);
 [...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Copy profile').click();
 await until(()=>!document.querySelector('dialog[open]'),'copy committed');
 if((await snapshot()).device!==expected.device)throw Error('Copy switched editor device');
}
const picker=document.querySelector('.device-picker select');set(picker,expected.target);
s=await until(async()=>{const s=await snapshot();return s.device===expected.target&&s},'target selected');
const copy=s.library.profiles.find(p=>p.name==='On device B');if(!copy)throw Error('Destination profile missing');
if(s.library.active_profile!==copy.id)await cmd({command:'profile_select',device:expected.target,id:copy.id});
s=await snapshot();const config=s.smart_profiles.config;
if(config.enabled!==false||config.fallback!==expected.target_default||config.bindings[copy.id]?.value!=='Independent copy')throw Error('Destination routing preferences not preserved');
if(!sameBinding(config.bindings[expected.target_default],expected.target_binding))throw Error('Existing destination binding changed');
const dial=s.slots.find(v=>v.controller==='Encoder'&&v.index===0);if(!dial?.raw.Encoder?.background)throw Error('Copy lost cropped image');
await cmd({command:'export_profile',device:expected.target,path:__EXPORT__});
return {phase,independent_binding:true,destination_preferences_preserved:true,artwork_retained:true,restart:phase==='restart',usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/smart-copy/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True);binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-smart-copy-') as d:
        root=Path(d);state=root/'state';image=root/'art.png';archive=root/'copy.streamDeckProfile';Image.new('RGB',(200,100),'#246ac8').save(image)
        seed=Host(binary,[],state,a.output/'seed.log',model='plus').ready()
        try:
            s=seed.state();device=s['device'];source=s['library']['active_profile']
            seed.post({'command':'profile_rename','id':source,'name':'Bound source'})
            seed.post({'command':'assign','controller':'Encoder','index':0,'uuid':'com.elgato.streamdeck.system.hotkey'})
            binding={'kind':'window_class','value':'Deckforge copy fixture','name':'Copy app'}
            seed.post({'command':'smart_config','expected':seed.state()['smart_profiles']['config'],'settings':{'enabled':True,'fallback':source,'bindings':{source:binding}}})
            target='copy-target';seed.post({'command':'device_add','device':target,'model':'plus'});seed.post({'command':'device_select','device':target})
            s=seed.state();target_default=s['library']['active_profile'];target_binding={'kind':'window_class','value':'Existing target app','name':'Existing'}
            seed.post({'command':'smart_config','expected':s['smart_profiles']['config'],'settings':{'enabled':False,'fallback':target_default,'bindings':{target_default:target_binding}}})
            seed.post({'command':'device_select','device':device})
        finally:seed.stop()
        expected={'device':device,'source':source,'binding':binding,'target':target,'target_default':target_default,'target_binding':target_binding}
        for phase in ['copy','restart','archive','archive-restart']:
            if phase=='archive':
                shutil.rmtree(state)  # Only the exported file travels to the new session.
                state=root/'fresh-state'
            script=root/'test.js';proof=root/'proof.json';proof.unlink(missing_ok=True)
            script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__EXPECTED__',json.dumps(expected)).replace('__IMAGE__',json.dumps(str(image))).replace('__EXPORT__',json.dumps(str(archive))))
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                args=[str(binary),'desktop','--simulate','--state',str(state)]
                if phase=='archive':args+=['--model','plus','--open',archive.as_uri()]
                process=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
            if phase=='copy':image.unlink()
        with zipfile.ZipFile(archive) as z:
            metadata=[n for n in z.namelist() if n.endswith('/Deckforge/SmartProfile.json')];assert len(metadata)==1
            assert json.loads(z.read(metadata[0]))=={'version':1,'binding':{**binding,'value':'Independent copy'}}
            images=[n for n in z.namelist() if 'encoder-owned-' in n and n.endswith('.png')];assert images
            for name in images:
                im=Image.open(io.BytesIO(z.read(name))).convert('RGBA');assert im.size==(200,100) and im.getpixel((10,10))==(36,106,200,255)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'runs':reports,'portable_crop_verified':True,'usb_access':False},indent=2)+'\n')
    print('PASS: native duplicate/device copy, archive import into fresh state after deleting originals, bindings/artwork and restart; no USB.')

if __name__=='__main__':main()
