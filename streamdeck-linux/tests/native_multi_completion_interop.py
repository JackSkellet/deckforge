#!/usr/bin/env python3
"""Native Multi Action progress, cancellation, completion and saved preference. No USB."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import environment, result, cleanup

SCRIPT = r'''
const bridge=window.deckforge, command=payload=>bridge.request('command',payload);
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,label)=>{for(let n=0;n<200;n++){const value=await fn();if(value)return value;await pause(30)}throw Error('Timed out: '+label)};
const snapshot=()=>bridge.request('snapshot');
const initial=await snapshot();if(!initial.simulate)throw Error('Simulation required');
if(__RESTART__){if(initial.multi_action_feedback!==false)throw Error('Preference lost on restart');return{restart_preserved:true,usb_access:false};}
if(initial.multi_action_feedback!==true)throw Error('Fresh preference should show feedback');
const delay=(id,ms)=>({UUID:'com.elgato.streamdeck.multiactions.delay',ActionID:id,Settings:{delay:ms},States:[{}]});
const configure=async(type,wait=1100)=>{
 await command({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.multiactions.'+type});
 const p=await parent();await command({command:'multi_save',context:p.context,raw:{Actions:[{Actions:[delay('a',350),delay('b',wait)]},{Actions:[delay('c',350),delay('d',wait)]}]}});
};
const parent=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const tap=async()=>{await command({command:'input',event:'keyDown',index:0});await command({command:'input',event:'keyUp',index:0});};
const image=async()=>{const p=await parent();return until(()=>{const img=document.querySelector('.deck-key img');return img?.src==='data:image/png;base64,'+p.preview&&img.complete&&img.naturalWidth>0&&img},'current key image');};
const green=img=>{const canvas=document.createElement('canvas');canvas.width=img.naturalWidth;canvas.height=img.naturalHeight;const ctx=canvas.getContext('2d');ctx.drawImage(img,0,0);const pixels=ctx.getImageData(0,0,canvas.width,canvas.height).data;let left=0,right=0;for(let y=0;y<canvas.height;y++)for(let x=0;x<canvas.width;x++){const i=(y*canvas.width+x)*4,[r,g,b]=pixels.slice(i,i+3);if(g>70&&g>r*1.3&&g>b+8){if(x<canvas.width/2)left++;else right++;}}return{left,right};};
await configure('routine');await tap();
await until(async()=>{const p=await parent(),g=green(await image());return p.multi_running&&g.right>30&&g.left===0},'clockwise half progress after first completed delay');
await command({command:'multi_cancel'});
await until(async()=>{const p=await parent(),g=green(await image());return !p.multi_running&&g.left+g.right===0},'cancel restores original image');
await tap();
await until(async()=>{const p=await parent(),g=green(await image());return !p.multi_running&&g.left>30&&g.right>30},'completion check mark');
await until(async()=>{const g=green(await image());return g.left+g.right===0},'completion expiry');
const button=text=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===text);
(await until(()=>button('Preferences'),'preferences button')).click();
(await until(()=>button('Startup & about'),'preference tab')).click();
const toggle=await until(()=>[...document.querySelectorAll('label')].find(l=>l.textContent.includes('Show Multi Action progress and completion check marks'))?.querySelector('input'),'feedback toggle');
if(!toggle.checked)throw Error('Preference checkbox has wrong default');toggle.click();
await until(async()=>(await snapshot()).multi_action_feedback===false,'preference saved');
document.querySelector('[aria-label="Close Preferences"]').click();
await configure('routine2',600);await tap();
let sawRunning=false;
for(let n=0;n<80;n++){const p=await parent(),g=green(await image());if(g.left+g.right!==0)throw Error('Suppressed feedback painted');sawRunning||=p.multi_running;if(sawRunning&&!p.multi_running){if(p.state!==1)throw Error('Switch did not advance');break;}await pause(30);}
if(!sawRunning||(await parent()).multi_running)throw Error('Suppressed sequence did not complete');
return{clockwise_progress:true,cancel_has_no_success:true,completion_expires:true,native_preference_saved:true,suppression_keeps_switch_behavior:true,usb_access:false};
'''


def main():
    project=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(); binary=args.binary.resolve(strict=True); output=args.output.resolve(); output.mkdir(parents=True,exist_ok=True)
    report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'phases':[]}
    with tempfile.TemporaryDirectory(prefix='deckforge-multi-completion-') as directory:
        root=Path(directory)
        for restart in (False,True):
            name='restart' if restart else 'feedback'; script=root/(name+'.js'); script.write_text(SCRIPT.replace('__RESTART__',json.dumps(restart)))
            proof=root/(name+'.json'); env=environment(root)
            env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(output/(name+'.png')))
            with (output/(name+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    assert process.wait(timeout=35)==0,(output/(name+'.log')).read_text()[-5000:]
                    value=result(proof); assert value,(output/(name+'.log')).read_text()[-5000:]
                    report['phases'].append(value['result'])
                finally:cleanup(process)
    report['ok']=True
    (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: native clockwise step progress, cancel restoration, completion/expiry, saved suppression and Switch behavior across restart; no USB.')

if __name__=='__main__':main()
