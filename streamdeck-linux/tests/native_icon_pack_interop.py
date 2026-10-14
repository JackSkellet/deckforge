#!/usr/bin/env python3
"""Real official icon pack import, native library selection, Save/Undo and portable restart."""
import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<180;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent===name);
const current=async()=>(await snapshot()).slots.find(s=>s.index===0&&s.controller==='Keypad');
if(!(await snapshot()).simulate)throw Error('Private simulation required');
if(phase==='restart'){
 if(!(await current()).appearance.image_custom)throw Error('Custom artwork lost after removing pack');
 await until(()=>{const img=document.querySelector('.selected-art img');return img?.complete&&img.naturalWidth>0},'retained image rendered');
 await cmd({command:'export_profile',path:__EXPORT__});
 if((await b.request('icon_packs',{operation:'list'})).packs.length!==0)throw Error('Source packs were not removed');
 return {phase,owned_artwork_retained:true,pack_source_removed:true,usb_access:false};
}
if(phase==='catalog'){
 const installed=await b.request('icon_packs',{operation:'install',path:__PACK__});
 if(installed.icons.length!==134)throw Error('Original pack catalog changed');
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
}
const appearance=await until(()=>document.querySelector('.appearance-options'),'appearance');if(!appearance.open)appearance.querySelector('summary').click();
(await until(()=>{const v=button('Icon library…');return v&&!v.disabled&&v},'library')).click();
set(await until(()=>{const f=field('Icon pack');return f&&[...f.options].some(o=>o.value==='com.elgato.logoiconscolor')&&f},'packs'),'com.elgato.logoiconscolor');
set(field('Search icons'),'creative');
const choice=await until(()=>document.querySelector('.pack-icon-grid button[aria-label="Adobe logo (Color)"]'),'tag match');choice.scrollIntoView({block:'center'});
await until(()=>{const img=choice.querySelector('img');return img?.complete&&img.naturalWidth>0},'official SVG thumbnail');
if(phase==='catalog')return {phase,official_pack_installed:true,tag_search:true,thumbnail_rendered:true,usb_access:false};
choice.click();await until(()=>!document.querySelector('dialog[open]'),'selected draft');
await until(async()=>Object.values((await b.request('ui_state_get')).drafts||{}).some(d=>d.dirty&&Object.values(d.appearanceEdits||{}).some(e=>e.image_path)),'icon draft');
(await until(()=>{const v=button('Save changes');return v&&!v.disabled&&v},'Save')).click();
await until(async()=>(await current()).appearance.image_custom,'icon saved');
(await until(()=>{const v=button('Undo');return v&&!v.disabled&&v},'Undo')).click();await until(async()=>!(await current()).appearance.image_custom,'icon undone');
(await until(()=>{const v=button('Redo');return v&&!v.disabled&&v},'Redo')).click();await until(async()=>(await current()).appearance.image_custom,'icon redone');
await cmd({command:'export_profile',path:__EXPORT__});
return {phase,icon_saved:true,undo_redo:true,exported:true,usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/icon-packs/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True)
    binary=a.binary.resolve();original=project.parent/'evidence/elgato-7.6/raw/flsy_lZh6qrewVZPcqeNQmr2lhuQjI';reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-pack-native-') as d:
        root=Path(d);pack=root/'com.elgato.logoiconscolor.sdIconPack';pack.mkdir();prefix='com.elgato.keycreator.sdPlugin/static/'+pack.name+'/'
        with zipfile.ZipFile(original) as z:
            for name in z.namelist():
                if name.startswith(prefix) and not name.endswith('/'):
                    relative=Path(name[len(prefix):]);assert not relative.is_absolute() and '..' not in relative.parts
                    path=pack/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(z.read(name))
        expected=(pack/'icons/LogoAdobe-Color.svg').read_bytes();archive=root/'export.streamDeckProfile'
        for phase in ['catalog','apply','restart']:
            if phase=='restart':shutil.rmtree(pack);shutil.rmtree(root/'state/icon-packs')
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__PACK__',json.dumps(str(pack))).replace('__EXPORT__',json.dumps(str(archive))));proof=root/'proof.json';proof.unlink(missing_ok=True)
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
        with zipfile.ZipFile(archive) as z:assert any(z.read(n)==expected for n in z.namelist() if n.endswith('.svg'))
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'original_archive_sha256':hashlib.sha256(original.read_bytes()).hexdigest(),'selected_svg_sha256':hashlib.sha256(expected).hexdigest(),'runs':reports,'exact_exported_svg':True,'usb_access':False},indent=2)+'\n')
    print('PASS: official pack native import, tag search/thumbnail, icon Save/Undo/Redo, restart and exact export after source deletion; no USB.')

if __name__=='__main__':main()
