#!/usr/bin/env python3
"""Native hide/remove/restore, reviewed replacement API, pending artwork and restart."""
import argparse
import hashlib
import json
import subprocess
import tempfile
import zipfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,api=p=>b.request('icon_packs',p),snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__,id='org.test.managed';
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent===name);
const click=async name=>(await until(()=>{const v=button(name);return v&&!v.disabled&&v},name)).click();
const current=async()=>(await snapshot()).slots.find(s=>s.index===0&&s.controller==='Keypad');
const catalog=()=>api({operation:'list'}),pack=async()=>(await catalog()).packs.find(p=>p.id===id);
const open=async()=>{const d=await until(()=>document.querySelector('.appearance-options'),'appearance');if(!d.open)d.querySelector('summary').click();await click('Icon library…');await until(()=>field('Icon pack'),'library');};
const select=async value=>{const f=await until(()=>{const f=field('Icon pack');return f&&!f.disabled&&[...f.options].some(o=>o.value===value)&&f},'pack option');set(f,value);await pause(50);};
const search=async value=>{const f=await until(()=>{const f=field('Search icons');return f&&!f.disabled&&f},'search ready');set(f,value);await pause(50);};
const image=async()=>{const choice=await until(()=>document.querySelector('.pack-icon-grid button[aria-label="Fixture mark"]'),'fixture icon');choice.scrollIntoView({block:'center'});return until(()=>{const img=choice.querySelector('img');return img?.complete&&img.naturalWidth===64&&img},'thumbnail')};
const color=img=>{const c=document.createElement('canvas');c.width=64;c.height=64;const x=c.getContext('2d');x.drawImage(img,0,0);return [...x.getImageData(32,32,1,1).data].slice(0,3).join(',')};
const showHidden=async()=>{const c=[...document.querySelectorAll('input[type=checkbox]')].find(c=>c.closest('label')?.textContent.includes('Show hidden packs'));if(!c.checked)c.click();await pause(40);};
if(!(await snapshot()).simulate)throw Error('Private simulation required');
if(phase==='remove'){
 await api({operation:'install',path:__V1__});await api({operation:'install',path:__PINNED__});
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 await open();await select(id);await image();document.querySelector('.pack-icon-grid button[aria-label="Fixture mark"]').click();
 await until(()=>!document.querySelector('dialog[open]'),'selected draft');
 const draft=await until(async()=>Object.values((await b.request('ui_state_get')).drafts||{}).find(d=>d.dirty),'dirty icon draft');
 if(!Object.values(draft.appearanceEdits||{}).some(e=>e.image_path?.includes('/appearance-assets/')))throw Error('Pending selection is not owned');
 await open();await select(id);await click('Hide pack');await until(async()=>(await pack()).hidden,'hide persisted');
 await search('fixture');await image();await select(id);await click('Show pack');await until(async()=>!(await pack()).hidden,'shown');
 await search('');await select('com.elgato.logoiconscolor');
 if(!button('Remove pack…').disabled)throw Error('Pinned pack removal enabled');
 await click('Hide pack');await until(async()=>(await catalog()).packs.find(p=>p.id==='com.elgato.logoiconscolor').hidden,'pinned hidden');
 await select(id);await click('Remove pack…');await click('Remove pack');await until(async()=>!(await pack()),'removed');
 await click('Cancel');await until(()=>!document.querySelector('dialog[open]'),'library closed');
 await click('Save changes');await until(async()=>(await current()).appearance.image_custom,'saved pending icon after pack removal');
 await cmd({command:'export_profile',path:__EXPORT__});return {phase,hidden_search:true,pinned_rules:true,remove_review:true,pending_icon_survived:true,usb_access:false};
}
if(phase==='update'){
 await open();(await until(()=>document.querySelector('.pack-archives summary'),'archives')).click();
 (await until(()=>document.querySelector('button[aria-label="Restore Managed icons 1.0"]'),'restore removed')).click();await click('Apply replacement');await until(async()=>(await pack())?.version==='1.0','restored');
 // The file picker is covered by frontend tests; replacement staging/commit use
 // the real native API, followed by actual library rendering and restore buttons.
 const review=await api({operation:'review',id,path:__V2__});if(review.current.version!=='1.0'||review.replacement.version!=='2.0')throw Error('Incorrect replacement review');
 await api({operation:'apply',review});await click('Reload');await select(id);
 await until(async()=>color(await image())==='22,50,204','updated blue thumbnail');
 if(!(await current()).appearance.image_custom)throw Error('Replacement changed saved artwork');
 await click('Hide pack');await until(async()=>(await pack()).hidden,'replacement hidden');
 const details=document.querySelector('.pack-archives');if(!details.open)details.querySelector('summary').click();
 (await until(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='Restore Managed icons 1.0');return b&&!b.disabled&&b},'restore button ready')).click();
 await click('Apply replacement');await until(async()=>(await pack()).version==='1.0','old version restored');
 if(!(await pack()).hidden)throw Error('Replacement lost visibility preference');
 await select(id);await until(async()=>color(await image())==='200,40,60','restored red thumbnail');
 return {phase,reviewed_replacement:true,thumbnail_refreshed:true,restore_review:true,visibility_preserved:true,usb_access:false};
}
document.querySelectorAll('.deck-key')[7].click();await click('Manage icon packs…');await until(()=>document.body.textContent.includes('Packs are hidden.'),'hidden restart state');
if((await catalog()).archives.length<2)throw Error('Recovery archives did not persist');
await showHidden();await select(id);await click('Show pack');await until(async()=>!(await pack()).hidden,'unhidden after restart');
if(!(await catalog()).packs.find(p=>p.id==='com.elgato.logoiconscolor').hidden)throw Error('Unhiding changed another pack');
await click('Cancel');await cmd({command:'export_profile',path:__EXPORT__});
return {phase,restart:true,archives_retained:true,visibility_independent:true,usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/pack-management/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True);binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-pack-management-') as d:
        root=Path(d);sources=[]
        for version,color in [('1.0','#c8283c'),('2.0','#1632cc')]:
            source=root/version/'org.test.managed.sdIconPack';(source/'icons').mkdir(parents=True)
            (source/'manifest.json').write_text(json.dumps({'Name':'Managed icons','Author':'Fixture','Version':version,'Icon':'icons/mark.svg'}))
            (source/'icons.json').write_text(json.dumps([{'path':'mark.svg','name':'Fixture mark','tags':['fixture']}]))
            (source/'icons/mark.svg').write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="{color}"/></svg>');sources.append(source)
        expected=(sources[0]/'icons/mark.svg').read_bytes();pinned=root/'com.elgato.logoiconscolor.sdIconPack';pinned.mkdir();prefix='com.elgato.keycreator.sdPlugin/static/'+pinned.name+'/'
        with zipfile.ZipFile(project.parent/'evidence/elgato-7.6/raw/flsy_lZh6qrewVZPcqeNQmr2lhuQjI') as z:
            for name in z.namelist():
                if name.startswith(prefix) and not name.endswith('/'):
                    path=pinned/name[len(prefix):];path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(z.read(name))
        exported=root/'export.streamDeckProfile'
        for phase in ['remove','update','restart']:
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__V1__',json.dumps(str(sources[0]))).replace('__V2__',json.dumps(str(sources[1]))).replace('__PINNED__',json.dumps(str(pinned))).replace('__EXPORT__',json.dumps(str(exported))))
            proof=root/'proof.json';proof.unlink(missing_ok=True);env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
        with zipfile.ZipFile(exported) as z:assert any(z.read(n)==expected for n in z.namelist() if n.endswith('.svg'))
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'runs':reports,'exact_original_svg_after_management':True,'usb_access':False},indent=2)+'\n')
    print('PASS: native hide/search/pinned rules, remove/restore, replacement API/thumbnail refresh, pending icon Save and restart; no USB.')

if __name__=='__main__':main()
