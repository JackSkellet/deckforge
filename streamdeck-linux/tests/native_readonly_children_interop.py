#!/usr/bin/env python3
"""Native read-only nested inspection/copy and unchanged-source verification."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
const LOGIC='com.elgato.streamdeck.keys.logic',SWITCH='com.elgato.streamdeck.multiactions.routine2',TIMER='com.elgato.streamdeck.system.timer';
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
if(phase==='seed'){
 const id=s.library.active_profile;
 await cmd({command:'profile_rename',id,name:'Reference'});
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:LOGIC});
 s=await snapshot();const root=s.slots.find(v=>v.index===0&&v.controller==='Keypad');
 await cmd({command:'logic_save',context:root.context,raw:{Actions:[{
  UUID:SWITCH,ActionID:'branch',Name:'Switch',Settings:{},Actions:[{Actions:[]},{Actions:[
   {UUID:TIMER,Name:'First',Settings:{duration:12},States:[{Title:'First'}]},
   {UUID:TIMER,Name:'Second',Settings:{duration:34},States:[{Title:'Second'}]}
  ]}]},{},{}]}});
 await cmd({command:'profile_create',name:'Editable'});
 await cmd({command:'profile_select',id});
 return {phase,source:id,usb_access:false};
}
const profiles=s.library.profiles,reference=profiles.find(p=>p.name==='Reference'),editable=profiles.find(p=>p.name==='Editable');
if(!reference?.readonly)throw Error('Source was not loaded read-only');
const choose=async name=>{
 const button=await until(()=>[...document.querySelectorAll('.profile-sidebar button')].find(b=>b.textContent.includes(name)),name+' profile');button.click();
 await until(async()=>(await snapshot()).library.active_profile===profiles.find(p=>p.name===name).id,'selected '+name);
 await pause(100);
};
if(phase==='restart'){
 await choose('Editable');
 const pasted=(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
 if(pasted?.uuid!==TIMER||pasted.settings.duration!==34)throw Error('Restart lost the selected legacy child');
 return {phase,restored:true,usb_access:false};
}
const root=s.slots.find(v=>v.controller==='Keypad'&&v.index===0),before=JSON.stringify(root.raw);
const parent=root.trigger_children[0],child=parent.multi_children[1][1];
const inspect=async()=>{
 (await until(()=>document.querySelectorAll('.deck-key')[0],'source key')).click();
 (await until(()=>document.querySelector('[aria-label="Key logic"] .sequence-select'),'root child')).click();
 (await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent==='Sequence 2'),'second branch')).click();
 const sequence=await until(()=>document.querySelector('[aria-label="Action sequence"]'),'nested sequence');
 const row=await until(()=>sequence.querySelectorAll('.sequence-select')[1],'second legacy child');row.click();await pause(60);
 return {sequence,row};
};
let {sequence,row}=await inspect();
const copy=()=>[...sequence.querySelectorAll('button')].find(b=>b.textContent==='Copy selected');
await until(()=>copy()&&!copy().matches(':disabled'),'copy enabled');
for(const name of ['Cut selected','Paste after selected']){
 const control=[...sequence.querySelectorAll('button')].find(b=>b.textContent===name);
 if(!control?.matches(':disabled'))throw Error(name+' enabled in read-only profile');
}
const label=[...sequence.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent==='Action label')?.querySelector('input');
if(!label?.matches(':disabled')||label.value!=='Second')throw Error('Read-only fields not inspected correctly');
copy().click();await until(async()=>(await snapshot()).clipboard?.context===child.context,'correct legacy child copied');
await until(()=>copy()&&!copy().matches(':disabled'),'copy finished');
for(const key of ['x','v','Delete'])row.dispatchEvent(new KeyboardEvent('keydown',{key,ctrlKey:key!=='Delete',bubbles:true,cancelable:true}));
const drop=new Event('drop',{bubbles:true,cancelable:true});Object.defineProperty(drop,'dataTransfer',{value:{types:['application/x-deckforge-action'],getData:()=>TIMER}});row.parentElement.dispatchEvent(drop);
for(const payload of [{command:'slot_cut',context:child.context},
 {command:'child_paste',context:parent.context,branch:1,index:2,expected:parent.raw}]){
 let rejected=false;try{await cmd(payload)}catch{rejected=true}if(!rejected)throw Error('Host allowed readonly '+payload.command);
}
if(JSON.stringify((await snapshot()).slots.find(v=>v.context===root.context).raw)!==before)throw Error('Readonly source changed');
await choose('Editable');const key=document.querySelectorAll('.deck-key')[0];key.click();await pause(60);
key.dispatchEvent(new KeyboardEvent('keydown',{key:'v',ctrlKey:true,bubbles:true,cancelable:true}));
const pasted=await until(async()=>{const p=(await snapshot()).slots.find(v=>v.index===0&&v.controller==='Keypad');return p?.uuid===TIMER&&p},'paste into editable profile');
if(pasted.settings.duration!==34||pasted.context===child.context)throw Error('Copied wrong legacy sibling');
await until(()=>document.body.textContent.includes('Pasted'),'paste finished');
await choose('Reference');
if(JSON.stringify((await snapshot()).slots.find(v=>v.context===root.context).raw)!==before)throw Error('Copy/paste changed source');
({sequence,row}=await inspect());copy().scrollIntoView({block:'center'});await pause(100);
return {phase,deep_inspection:true,legacy_second_child:true,copy_to_editable:true,source_unchanged:true,
 mutation_controls_disabled:true,keyboard_drop_and_host_mutations_blocked:true,usb_access:false};
'''

def main():
    root_project=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=root_project/'target/release/deckforge')
    p.add_argument('--output',type=Path,default=root_project/'target/readonly-children/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True)
    reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-readonly-') as d:
        root=Path(d)
        for phase in ['seed','readonly','restart']:
            script=root/'probe.js';proof=root/'proof.json';proof.unlink(missing_ok=True)
            script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)))
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','plus','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=45);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report
                    reports.append(report)
                finally:cleanup(process)
            if phase=='seed':
                state=root/'state/session.json';saved=json.loads(state.read_text())
                source=next(p for p in saved['library']['profiles'] if p['name']=='Reference');source['readonly']=True
                state.write_text(json.dumps(saved))
        (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':reports,'usb_access':False},indent=2)+'\n')
    print('PASS: native read-only nested inspection/copy, correct legacy sibling, blocked mutations, unchanged source and restart; no USB.')

if __name__=='__main__':main()
