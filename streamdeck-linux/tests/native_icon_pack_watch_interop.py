#!/usr/bin/env python3
"""An open native library follows separate-process installs and filesystem changes."""
import argparse
import ctypes
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result, wait

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),mark=n=>b.request('command',{command:'export_profile',path:__ROOT__+'/ready-'+n+'.streamDeckProfile'});
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent===name);
const click=async name=>(await until(()=>{const v=button(name);return v&&!v.disabled&&v},name)).click();
const icon=name=>[...document.querySelectorAll('.pack-icon-grid button')].find(b=>b.getAttribute('aria-label')===name);
const color=async name=>{const item=await until(()=>icon(name),name);item.scrollIntoView({block:'center'});const img=await until(()=>{const i=item.querySelector('img');return i?.complete&&i.naturalWidth===64&&i},'thumbnail');const c=document.createElement('canvas');c.width=64;c.height=64;const x=c.getContext('2d');x.drawImage(img,0,0);return [...x.getImageData(32,32,1,1).data].slice(0,3).join(',');};
const initial=await snapshot();if(!initial.simulate)throw Error('Private simulation required');
await click('Manage icon packs…');set(await until(()=>field('Search icons'),'search'),'Live');await mark(1);
await until(()=>icon('Live icon'),'external CLI install');await until(async()=>await color('Live icon')==='200,40,60','initial color');
set(field('Icon pack'),'live.a');await pause(50);await click('Remove pack…');await mark(2);
await until(()=>document.body.textContent.includes('This pack changed while you reviewed it.'),'stale review warning');
if(!button('Remove pack').disabled||document.querySelector('.pack-change-review h3').textContent!=='Remove Live pack?')throw Error('External change replaced or approved review');
await click('Cancel change');
if(field('Search icons').value!=='Live'||field('Icon pack').value!=='live.a')throw Error('Refresh lost search or selection');
await until(async()=>await color('Live icon')==='22,50,204','replacement thumbnail');await mark(3);
await until(()=>icon('Live icon updated'),'in-place metadata edit');await until(async()=>await color('Live icon updated')==='17,153,85','in-place pixels');await mark(4);
await until(()=>!icon('Live icon updated')&&field('Icon pack').value==='','external removal');if(field('Search icons').value!=='Live')throw Error('Removal cleared search');await mark(5);
await until(()=>icon('Live second'),'second external install');set(field('Search icons'),'');await mark(6);
await until(()=>document.body.textContent.includes('Packs are hidden.'),'new visibility preferences');await mark(7);
await until(()=>icon('Live second'),'atomic visibility replacement');
const final=await snapshot();if(final.icon_library.error)throw Error(final.icon_library.error);
if(JSON.stringify(final.library)!==JSON.stringify(initial.library))throw Error('Watching changed profile graph');
return {external_cli_install:true,atomic_replacement:true,nested_edits:true,stale_review_preserved:true,search_preserved:true,removed_selection_repaired:true,visibility_creation_and_replace:true,revision:final.icon_library.revision,usb_access:false};
'''

def pack(root,id,name,version,icon_name,color):
    path=root/(id+'.sdIconPack');(path/'icons').mkdir(parents=True,exist_ok=True)
    (path/'manifest.json').write_text(json.dumps({'Name':name,'Author':'Fixture','Version':version,'Icon':'icons/icon.svg'}))
    (path/'icons.json').write_text(json.dumps([{'path':'icon.svg','name':icon_name,'tags':['live']}]))
    (path/'icons/icon.svg').write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="{color}"/></svg>')
    return path

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/pack-watch/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True);binary=a.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='deckforge-pack-watch-') as d:
        root=Path(d);state=root/'state';store=state/'icon-packs';proof=root/'proof.json';script=root/'test.js';script.write_text(SCRIPT.replace('__ROOT__',json.dumps(str(root))))
        v1=pack(root/'v1','live.a','Live pack','1.0','Live icon','#c8283c');v2=pack(root/'v2','live.a','Live pack changed','2.0','Live icon','#1632cc');other=pack(root/'other','live.b','Live second pack','1.0','Live second','#8855aa')
        env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/'library.png'))
        with (a.output/'native.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(state),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            def ready(n):
                def check():
                    if process.poll() is not None:raise AssertionError(result(proof) if proof.exists() else 'Native host exited')
                    return (root/f'ready-{n}.streamDeckProfile').is_file()
                wait(check,f'native step {n}')
            def install(source,destination):subprocess.run([str(binary),'icon-pack','install',str(source),'--destination',str(destination)],check=True,capture_output=True,text=True,timeout=10)
            try:
                ready(1);install(v1,store)
                ready(2);staging=root/'exchange';install(v2,staging)
                libc=ctypes.CDLL(None,use_errno=True);rename=libc.renameat2;rename.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint];rename.restype=ctypes.c_int
                assert rename(-100,os.fsencode(staging/'live.a.sdIconPack'),-100,os.fsencode(store/'live.a.sdIconPack'),2)==0,ctypes.get_errno()
                ready(3);pack(store,'live.a','Live pack changed','3.0','Live icon updated','#119955')
                ready(4);(store/'live.a.sdIconPack').rename(root/'removed.sdIconPack')
                ready(5);install(other,store)
                ready(6);prefs=store/'preferences/visibility';prefs.mkdir(parents=True)
                pending=prefs/'pending.json';pending.write_text(json.dumps(['live.b']));pending.replace(prefs/'hidden.json')
                ready(7);pending.write_text('[]');pending.replace(prefs/'hidden.json')
                code=process.wait(timeout=35);report=result(proof);assert code==0 and report,report
            finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'result':report,'no_manual_reload':True,'usb_access':False},indent=2)+'\n')
    print('PASS: native live catalog from external CLI/filesystem changes, stale review guard, search/selection and visibility updates; no Reload or USB.')

if __name__=='__main__':main()
