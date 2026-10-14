#!/usr/bin/env python3
"""Real WebKit routing/host imports from synthetic native drag notifications."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result
from native_file_open_interop import archive

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
const key=i=>document.querySelectorAll('.deck-key')[i];
const drag=(phase,path,kind,element,error)=>{
 const rect=element?.getBoundingClientRect();
 window.dispatchEvent(new CustomEvent('deckforge-file-drag',{detail:{phase,path,kind,
  x:rect?rect.left+rect.width/2:30,y:rect?rect.top+rect.height/2:30,...(error?{error}:{})}}));
};
if(phase==='restart'){
 const original=s.library.profiles.find(p=>p.name==='Default profile');await cmd({command:'profile_select',id:original.id});
 s=await snapshot();if(s.slots.find(v=>v.controller==='Keypad'&&v.index===1)?.settings.text!=='from desktop')throw Error('Restart lost dropped action');
 return {phase,restored:true,os_drag_automated:false,usb_access:false};
}
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
s=await snapshot();const original=s.slots.find(v=>v.controller==='Keypad'&&v.index===0);
await cmd({command:'slot_copy',context:original.context});
const input=await until(()=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent==='Title')?.querySelector('textarea,input'),'title');
Object.getOwnPropertyDescriptor(input.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(input,'Unsaved original');input.dispatchEvent(new Event('input',{bubbles:true}));
await until(async()=>(await b.request('ui_state_get')).drafts[original.context]?.dirty,'draft');
drag('drop',__ACTION__,'action',key(0));
await until(()=>document.body.textContent.includes('Save or revert changes before replacing this action'),'unsaved target blocked');
if((await snapshot()).slots.length!==1)throw Error('Rejected drop changed controls');
drag('enter',__ACTION__,'action',key(1));await until(()=>key(1).classList.contains('file-drop-target'),'hover');
drag('leave');await until(()=>!key(1).classList.contains('file-drop-target'),'hover cleared');
drag('enter',__ACTION__,'action',key(1));drag('drop',__ACTION__,'action',key(1));
await until(async()=>(await snapshot()).slots.find(v=>v.controller==='Keypad'&&v.index===1)?.settings.text==='from desktop','action dropped');
await until(()=>document.body.textContent.includes('Action imported'),'drop finished');
s=await snapshot();if(s.clipboard?.context!==original.context||s.library.profiles.length!==1)throw Error('Targeted action drop changed clipboard/profile');
drag('drop',undefined,undefined,key(2),'Drop one Stream Deck file at a time.');
await until(()=>document.body.textContent.includes('Drop one Stream Deck file at a time.'),'multiple files rejected');
drag('drop',__BAD__,'action',key(2));
await until(()=>document.body.textContent.includes('Not a readable Stream Deck archive'),'invalid archive rejected');
if((await snapshot()).slots.some(v=>v.controller==='Keypad'&&v.index===2))throw Error('Bad drop assigned a control');
document.querySelector('[aria-label="New profile"]').click();await until(()=>document.querySelector('dialog[open]'),'modal');
drag('drop',__ACTION__,'action',key(2));await until(()=>document.body.textContent.includes('Close the dialog before dropping a file.'),'modal blocked');
[...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Cancel').click();await until(()=>!document.querySelector('dialog[open]'),'modal closed');
drag('drop',__ACTION__,'action',null);
await until(async()=>(await snapshot()).clipboard?.name==='Desktop import','background drop clipboard');
await until(()=>document.body.textContent.includes('Action copied from file'),'background complete');
drag('drop',__PROFILE__,'profile',key(2));
await until(async()=>{const s=await snapshot();return s.library.profiles.find(p=>p.id===s.library.active_profile)?.name==='Desktop profile'},'profile drop');
if(!(await b.request('ui_state_get')).drafts[original.context]?.dirty)throw Error('Profile drop lost draft');
if((await snapshot()).library.profiles.length!==2)throw Error('Unexpected profile created');
await until(()=>document.body.textContent.includes('Profile imported from file'),'profile complete');
return {phase,hit_tested_action:true,hover:true,clipboard_unchanged_for_targeted_drop:true,
 unsaved_target_blocked:true,bad_and_multiple_files_rejected:true,modal_blocked:true,
 background_action_clipboard:true,profile_drop:true,draft_preserved:true,os_drag_automated:false,usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/file-drop/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True);reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-file-drop-') as d:
        root=Path(d);action=root/'action % #.streamDeckAction';profile=root/'profile.streamDeckProfile';bad=root/'bad.streamDeckAction'
        archive(action,True);archive(profile,False);bad.write_bytes(b'not a zip')
        for phase in ['drop','restart']:
            script=root/'test.js';proof=root/'proof.json';proof.unlink(missing_ok=True)
            text=SCRIPT
            for key,value in {'PHASE':phase,'ACTION':str(action),'PROFILE':str(profile),'BAD':str(bad)}.items():text=text.replace('__'+key+'__',json.dumps(value))
            script.write_text(text);env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','plus','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=45);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
            if phase=='drop':action.unlink();profile.unlink()
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':reports,'os_drag_automated':False,'usb_access':False},indent=2)+'\n')
    print('PASS: native UI/host file drop routing, target guards, profile/clipboard import and restart; OS drag gesture not automated; no USB.')

if __name__=='__main__':main()
