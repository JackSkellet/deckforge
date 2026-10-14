#!/usr/bin/env python3
"""Real native nested Copy/Paste controls, keyboard routing, crop portability and restart."""
import argparse
import hashlib
import io
import json
import subprocess
import tempfile
import zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup, environment, result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<180;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
const TEXT='com.elgato.streamdeck.system.text',MULTI='com.elgato.streamdeck.multiactions.routine',STACK='com.elgato.streamdeck.dial.stack',HOTKEY='com.elgato.streamdeck.system.hotkey';
const control=async(type,index)=>(await snapshot()).slots.find(s=>s.controller===type&&s.index===index);
const button=label=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===label&&!b.disabled);
const click=async label=>(await until(()=>button(label),label)).click();
const select=async(type,index)=>{(await until(()=>document.querySelectorAll(type==='Keypad'?'.deck-key':'.deck-dial')[index],'control')).click();await pause(100)};
const shortcut=(node,key)=>node.dispatchEvent(new KeyboardEvent('keydown',{key,ctrlKey:true,bubbles:true,cancelable:true}));
if(!(await snapshot()).simulate)throw Error('Private simulation required');
if(phase==='edit'){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:TEXT});
 await cmd({command:'assign',controller:'Keypad',index:1,uuid:MULTI});
 await select('Keypad',0);shortcut(document.querySelectorAll('.deck-key')[0],'c');
 await until(async()=>(await snapshot()).clipboard?.uuid===TEXT,'root copied');
 await select('Keypad',1);await click('Paste at end');
 let parent=await until(async()=>{const p=await control('Keypad',1);return p?.multi_children[0]?.length===1&&p},'nested paste');
 const child=parent.multi_children[0][0];
 const row=await until(()=>document.querySelector('.sequence-select'),'child selection');row.click();await pause(50);shortcut(row,'c');
 await until(async()=>(await snapshot()).clipboard?.context===child.context,'nested keyboard copy');
 await select('Keypad',2);shortcut(document.querySelectorAll('.deck-key')[2],'v');
 const root=await until(()=>control('Keypad',2),'child pasted to canvas');
 if(root.context===child.context||root.uuid!==TEXT)throw Error('Child not independently copied to canvas');
 await cmd({command:'assign',controller:'Encoder',index:0,uuid:STACK});
 await cmd({command:'assign',controller:'Encoder',index:1,uuid:HOTKEY});
 const dial=await control('Encoder',1),source=await b.request('image_crop',{mode:'load',context:dial.context,path:__IMAGE__});
 await b.request('image_crop',{mode:'save',context:dial.context,path:__IMAGE__,source_hash:source.source_hash,
   expected:null,rect:{x:0,y:0,width:200,height:100}});
 await cmd({command:'slot_copy',context:dial.context});
 await select('Encoder',0);await click('Paste at end');
 const stack=await until(async()=>{const s=await control('Encoder',0);return s?.stack_children.length===1&&s},'stack paste');
 if(!stack.stack_children[0].raw.Encoder?.background)throw Error('Stack lost cropped image');
 const stackRow=await until(()=>document.querySelector('.sequence-select'),'stack child');stackRow.click();
 await click('Copy selected');
 await until(async()=>(await snapshot()).clipboard?.context===stack.stack_children[0].context,'stack copy');
 await select('Encoder',2);shortcut(document.querySelectorAll('.deck-dial')[2],'v');
 await until(()=>control('Encoder',2),'stack child on canvas');
 await until(()=>document.body.textContent.includes('Pasted Hotkey'),'paste completion');
 await select('Encoder',0);
 await until(()=>document.querySelector('[aria-label="Dial stack"]'),'source stack editor');
 const cutRow=document.querySelector('.sequence-select');cutRow.click();await pause(50);shortcut(cutRow,'x');
 await until(async()=>(await snapshot()).clipboard?.mode==='cut','nested cut');
 if((await control('Encoder',0)).stack_children.length!==1)throw Error('Cut removed source before paste');
 for(let attempt=0;attempt<3;attempt++){
 await until(()=>button('Cut selected')&&document.querySelector('.sequence-row.cut-control'),'cut reflected in editor');
 document.querySelector('.sequence-select').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
 await until(async()=>!(await snapshot()).clipboard,'cut cancelled');
 if((await control('Encoder',0)).stack_children.length!==1)throw Error('Cancel removed source');
 await until(()=>button('Cut selected')&&!document.querySelector('.sequence-row.cut-control'),'cancel reflected in editor');
 shortcut(document.querySelector('.sequence-select'),'x');await until(async()=>(await snapshot()).clipboard?.mode==='cut','cut again');
 }
 await cmd({command:'assign',controller:'Encoder',index:3,uuid:STACK});
 await select('Encoder',3);await click('Paste at end');
 await until(async()=>{const s=await snapshot();return !s.clipboard&&s.slots.find(v=>v.controller==='Encoder'&&v.index===3)?.stack_children.length===1},'nested cut committed');
 if((await control('Encoder',0)).stack_children.length!==0)throw Error('Cut left duplicate source');
 await until(()=>document.body.textContent.includes('Pasted action'),'paste status updated');
 await until(()=>document.querySelector('[aria-label="Dial stack"]'),'stack editor selected');
 document.querySelector('.sequence-select').click();
 await until(()=>button('Copy selected'),'stack child controls');
 button('Copy selected').scrollIntoView({block:'center'});await pause(100);
 return {phase,native_copy_buttons:true,nested_keyboard_cut:true,cut_cancelled:true,cancel_repeats:3,source_retained_until_paste:true,nested_keyboard_copy:true,canvas_keyboard_paste:true,
   multi_child_preserved:true,stack_crop_preserved:true,usb_access:false};
}
const multi=await control('Keypad',1),stack=await control('Encoder',3),dial=await control('Encoder',2);
if(multi?.multi_children[0].length!==1||(await control('Keypad',2))?.uuid!==TEXT)throw Error('Restart lost multi copy');
if(stack?.stack_children.length!==1||(await control('Encoder',0))?.stack_children.length!==0||!dial?.raw.Encoder?.background)throw Error('Restart lost stack copy');
await cmd({command:'export_profile',path:__EXPORT__});
return {phase,restart_preserved:true,reexport_without_source:true,usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    p.add_argument('--output',type=Path,default=project/'target/action-clipboard/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True)
    reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-nested-clipboard-') as d:
        root=Path(d);source=root/'crop.png';archive=root/'result.streamDeckProfile'
        Image.new('RGB',(200,100),'#248cce').save(source)
        for phase in ['edit','restart']:
            script=root/'test.js';proof=root/'proof.json';proof.unlink(missing_ok=True)
            script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__IMAGE__',json.dumps(str(source))).replace('__EXPORT__',json.dumps(str(archive))))
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','plus','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=45);report=result(proof)
                    assert code==0 and report and report['result']['phase']==phase,report
                    reports.append(report)
                finally:cleanup(process)
            if phase=='edit':source.unlink()
        with zipfile.ZipFile(archive) as z:
            found=[]
            for name in z.namelist():
                if name.endswith('.png') and 'encoder-owned-' in name:
                    im=Image.open(io.BytesIO(z.read(name))).convert('RGBA')
                    assert im.size==(200,100) and im.getpixel((10,10))==(36,140,206,255)
                    found.append(name)
            assert len(found)>=3,found
        (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':reports,'owned_crop_files':len(found),'restart_without_source':True,'usb_access':False},indent=2)+'\n')
    print('PASS: native nested Copy/Cut/Paste, keyboard scope, Multi/Stack copies, portable crops, restart; no USB.')

if __name__=='__main__':main()
