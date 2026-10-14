#!/usr/bin/env python3
"""Native editor history with an unchanged Counter plugin, grouped saves and restart."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result
from sample_interop import SAMPLE_COMMIT, COUNTER_LOCK_SHA256

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<180;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const set=(e,value)=>{const proto=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent===name);
const counter=async()=>(await snapshot()).slots.find(s=>s.index===1&&s.controller==='Keypad');
const textAction=async()=>(await snapshot()).slots.find(s=>s.index===2&&s.controller==='Keypad');
const shortcut=(key,shift=false,target=document.body)=>target.dispatchEvent(new KeyboardEvent('keydown',{key,ctrlKey:true,shiftKey:shift,bubbles:true,cancelable:true}));
const tap=async()=>{await cmd({command:'input',event:'keyDown',index:1});await cmd({command:'input',event:'keyUp',index:1});};
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
await until(async()=>(await snapshot()).plugins.some(p=>p.id==='com.elgato.counter'&&p.connected),'Counter ready');
if(phase==='restart'){
 const c=await counter(),t=await textAction();
 if(c.settings.count!==4||c.settings.incrementBy!==1||t.settings.pastedText!=='Portable history text'||t.title!=='History title')throw Error('Restart lost undone/redone state');
 if(s.history.undo!==null||s.history.redo!==null)throw Error('Session history unexpectedly persisted');
 return {phase,persisted_graph:true,session_history_cleared:true,usb_access:false};
}
await cmd({command:'assign',controller:'Keypad',index:1,uuid:'com.elgato.counter.action'});
await until(async()=>(await counter())?.title==='0','Counter appeared');
await tap();await until(async()=>(await counter())?.settings.count===1,'plugin count');
const context=(await counter()).context;
document.querySelectorAll('.deck-key')[1].click();
(await until(()=>{const v=button('Undo');return v&&!v.disabled&&v},'Undo ready')).click();
await until(async()=>!(await counter()),'assignment undone');
await until(()=>!button('Redo').disabled,'Redo ready');shortcut('z',true);await until(async()=>(await counter())?.settings.count===1,'assignment redone');
if((await counter()).context!==context)throw Error('Redo lost original identity');
set(await until(()=>field('Increment by'),'increment field'),'3');
await until(()=>{const v=button('Save changes');return v&&!v.disabled&&v},'Save ready');button('Save changes').click();
await until(async()=>(await counter()).settings.incrementBy===3,'increment saved');
await tap();await until(async()=>(await counter()).settings.count===4,'new plugin count');
await until(()=>!button('Undo').disabled,'Undo increment ready');button('Undo').click();
await until(async()=>{const c=await counter();return c.settings.incrementBy===1&&c.settings.count===4},'undo preserved runtime counter');
await cmd({command:'assign',controller:'Keypad',index:2,uuid:'com.elgato.streamdeck.system.text'});
await until(()=>document.querySelectorAll('.deck-key').length>1,'canvas');document.querySelectorAll('.deck-key')[2].click();
const before=await textAction();
set(await until(()=>field('Title'),'title'),'History title');await pause(60);
set(await until(()=>field('Text'),'text'),'Portable history text');await pause(60);
if(!button('Undo').disabled)throw Error('Dirty edit did not disable undo');
const revision=(await snapshot()).history.revision;shortcut('z',false,field('Title'));await pause(50);
if((await snapshot()).history.revision!==revision)throw Error('Text undo reached host');
button('Save changes').click();await until(async()=>(await textAction()).title==='History title'&&(await textAction()).settings.pastedText==='Portable history text','grouped Save');
await until(()=>!button('Undo').disabled,'Undo grouped ready');button('Undo').click();
await until(async()=>{const t=await textAction();return t.title===before.title&&JSON.stringify(t.settings)===JSON.stringify(before.settings)},'one Undo reverses whole Save');
await until(()=>!button('Redo').disabled,'Redo grouped ready');shortcut('y');await until(async()=>(await textAction()).title==='History title'&&(await textAction()).settings.pastedText==='Portable history text','Redo grouped');
return {phase,plugin_updates_preserved:true,assignment_identity_restored:true,grouped_save:true,undo_redo_controls:true,dirty_and_text_guards:true,usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/editor-history/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True)
    binary=a.binary.resolve();samples=project.parent/'evidence/official-samples';counter=samples/'counter/com.elgato.counter.sdPlugin'
    assert hashlib.sha256((samples/'counter/package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
    if (samples/'.git').exists():assert subprocess.check_output(['git','-C',str(samples),'rev-parse','HEAD'],text=True).strip()==SAMPLE_COMMIT
    reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-history-native-') as d:
        root=Path(d)
        for phase in ['edit','restart']:
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)));proof=root/'proof.json';proof.unlink(missing_ok=True)
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--plugin',str(counter),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sample_commit':SAMPLE_COMMIT,'counter_bundle_sha256':hashlib.sha256((counter/'bin/plugin.js').read_bytes()).hexdigest(),'runs':reports,'usb_access':False},indent=2)+'\n')
    print('PASS: native Undo/Redo, unchanged Counter live updates, grouped Save, dirty/text guards and restart; no USB.')

if __name__=='__main__':main()
