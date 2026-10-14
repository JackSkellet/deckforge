#!/usr/bin/env python3
"""Native multi-root choice for canvas/file clipboard, folder media, stale source and restart."""
import argparse
import hashlib
import io
import json
import subprocess
import tempfile
import uuid
import zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup, environment, result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<190;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent===name);
const click=async name=>(await until(()=>{const b=button(name);return b&&!b.disabled&&b},name)).click();
const slot=async index=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===index);
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
if(phase==='canvas'){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 const original=await slot(0);await cmd({command:'slot_copy',context:original.context});const before=await snapshot();
 const key=await until(()=>document.querySelectorAll('.deck-key')[1],'destination');const r=key.getBoundingClientRect();
 window.dispatchEvent(new CustomEvent('deckforge-file-drag',{detail:{phase:'drop',kind:'action',path:__FILE__,x:r.x+r.width/2,y:r.y+r.height/2}}));
 await until(()=>document.querySelector('dialog[open]')&&field('Action in file'),'chooser');s=await snapshot();
 if(s.pending_action_file.info.roots.length!==3||await slot(1)||s.clipboard.context!==before.clipboard.context)throw Error('Inspection mutated destination/clipboard');
 if(![...field('Action in file').options].find(o=>o.text.includes('Dial only root')).disabled)throw Error('Incompatible root is selectable');
 set(field('Find action'),'Text root');await click('Import action');await until(async()=>(await slot(1))?.settings.pastedText==='selected text root','selected text');
 s=await snapshot();if(s.pending_action_file||s.clipboard.context!==before.clipboard.context||s.library.profiles[0].pages.length!==before.library.profiles[0].pages.length)throw Error('Import changed unrelated data');
 await click('Undo');await until(async()=>!(await slot(1)),'undo');await click('Redo');await until(async()=>(await slot(1))?.settings.pastedText==='selected text root','redo');
 await cmd({command:'export_profile',path:__EXPORT__});return {phase,selected_only:true,clipboard_retained:true,compatibility_filter:true,undo_redo:true,usb_access:false};
}
if(phase==='restart'){
 if((await slot(1))?.settings.pastedText!=='selected text root'||(await slot(2))?.uuid!=='com.elgato.streamdeck.profile.openchild')throw Error('Restart lost chosen actions');
 await cmd({command:'export_profile',path:__EXPORT__});return {phase,owned_sources_retained:true,usb_access:false};
}
if(phase==='chooser'){
 await b.request('action_file_prepare',{path:__FILE__,device:s.device,destination:{controller:'Keypad',index:3,expected_target:null,expected_profile:s.library.active_profile,expected_page:s.library.active_page}});
 await until(()=>field('Action in file'),'chooser capture');return {phase,chooser_visible:true,usb_access:false};
}
await until(()=>field('Action in file'),'desktop chooser');s=await snapshot();const pending=s.pending_action_file;
if(!pending||pending.destination!==null)throw Error('Desktop open did not create a clipboard choice');
if(phase==='clipboard'){
 const graph=JSON.stringify(s.library);let rejected=false;try{await b.request('open_file',{path:__FILE__,device:s.device})}catch{rejected=true}
 if(!rejected||(await snapshot()).pending_action_file.id!==pending.id)throw Error('Second file replaced pending choice');
 set(field('Find action'),'Folder root');await click('Copy to clipboard');await until(async()=>!(await snapshot()).pending_action_file,'clipboard choice finished');
 s=await snapshot();if(JSON.stringify(s.library)!==graph||s.clipboard.uuid!=='com.elgato.streamdeck.profile.openchild')throw Error('Clipboard import changed graph');
 await cmd({command:'slot_paste',controller:'Keypad',index:2,expected_target:null});
 await cmd({command:'input',event:'keyDown',index:2});await cmd({command:'input',event:'keyUp',index:2});
 await until(async()=>(await slot(0))?.settings.pastedText==='inside selected folder','folder contents');
 await cmd({command:'folder_back'});await cmd({command:'export_profile',path:__EXPORT__});return {phase,clipboard_choice:true,pending_import_guard:true,selected_folder_retained:true,usb_access:false};
}
const graph=JSON.stringify(s.library),clipboard=JSON.stringify(s.clipboard);
// Replace the test input with another valid shared-action archive while its
// original choice is pending. Existing assigned media has an owned source.
await cmd({command:'slot_export',context:(await slot(1)).context,path:__FILE__});
set(field('Find action'),'Text root');await click('Copy to clipboard');
await until(()=>document.querySelector('dialog [role=alert]')?.textContent.includes('changed'),'source-change error');
s=await snapshot();if(s.pending_action_file.id!==pending.id||JSON.stringify(s.library)!==graph||JSON.stringify(s.clipboard)!==clipboard)throw Error('Stale source changed state');
await click('Cancel');await until(async()=>!(await snapshot()).pending_action_file,'cancelled');
return {phase,stale_source_rejected:true,cancel_preserved_state:true,usb_access:false};
'''

def fixture(path):
    profile,page,pins,folder=(str(uuid.uuid4()) for _ in range(4));base=f'Profiles/{profile}.sdProfile/'
    def action(action_uuid,name,settings,image=None):return {'UUID':action_uuid,'ActionID':str(uuid.uuid4()),'Name':name,'Settings':settings,'States':[{'Title':name,**({'Image':image} if image else {})}]}
    files={'package.json':{'FormatVersion':1,'DeviceModel':'Fake/Storage','RequiredPlugins':[]},
        base+'manifest.json':{'Name':'Multiple roots','Version':'3.0','Device':{'UUID':'$storage','Model':'Fake/Storage'},'Pages':{'Current':page,'Default':pins,'Pages':[page]}},
        base+f'Profiles/{page}/manifest.json':{'Controllers':[
            {'Type':'Keypad','Actions':{'0,0':action('com.elgato.streamdeck.profile.openchild','Folder root',{'ProfileUUID':folder}), '1,0':action('com.elgato.streamdeck.system.text','Text root',{'pastedText':'selected text root'},'Images/red.png')}},
            {'Type':'Encoder','Actions':{'0,0':action('missing.dial.action','Dial only root',{})}}]},
        base+f'Profiles/{pins}/manifest.json':{'Controllers':[]},
        base+f'Profiles/{folder}/manifest.json':{'Controllers':[{'Type':'Keypad','Actions':{'0,0':action('com.elgato.streamdeck.system.text','Folder child',{'pastedText':'inside selected folder'},'Images/blue.png')}}]}}
    images={}
    for name,color,owner in [('red.png','#c8283c',page),('blue.png','#1632cc',folder)]:
        data=io.BytesIO();Image.new('RGBA',(72,72),color).save(data,format='PNG');images[name]=data.getvalue();files[base+f'Profiles/{owner}/Images/{name}']=data.getvalue()
    with zipfile.ZipFile(path,'w',compression=zipfile.ZIP_DEFLATED) as z:
        for name,value in files.items():z.writestr(name,value if isinstance(value,bytes) else json.dumps(value))
    return images

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/action-choice/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True);binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-action-choice-') as d:
        root=Path(d);source=root/'multiple % #.streamDeckAction';images=fixture(source);exported=root/'result.streamDeckProfile'
        for phase in ['canvas','chooser','clipboard','changed','restart']:
            if phase=='restart':
                for path in root.glob('*.streamDeckAction*'):path.unlink()
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__FILE__',json.dumps(str(source))).replace('__EXPORT__',json.dumps(str(exported))));proof=root/'proof.json';proof.unlink(missing_ok=True)
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            args=[str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0']
            if phase in ['clipboard','changed']:args+=['--open',source.as_uri()]
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
        with zipfile.ZipFile(exported) as z:
            values=[z.read(n) for n in z.namelist() if n.endswith('.png')]
            assert all(image in values for image in images.values())
            assert all(b'missing.dial.action' not in z.read(n) for n in z.namelist() if n.endswith('manifest.json'))
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'runs':reports,'selected_media_byte_exact':True,'unselected_action_absent':True,'usb_access':False},indent=2)+'\n')
    print('PASS: native multi-root canvas/clipboard chooser, folder/media, stale-source cancellation, Undo/Redo and restart after deleting inputs; no USB.')

if __name__=='__main__':main()
