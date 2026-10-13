#!/usr/bin/env python3
"""Native Multi Action child images and SDK feedback; isolated simulated host."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from feedback_interop import make_plugin, PRIMARY, child
from native_editor_interop import environment, result, cleanup

SCRIPT = r'''
const bridge=window.deckforge, command=payload=>bridge.request('command',payload);
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,label)=>{for(let n=0;n<160;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+label)};
const snapshot=()=>bridge.request('snapshot');
await until(async()=>{const s=await snapshot();if(!s.simulate)throw Error('Simulation required');return s.plugins.some(p=>p.id===__PLUGIN__&&p.connected)},'plugin registration');
await command({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.multiactions.routine'});
const parent=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
await command({command:'multi_save',context:(await parent()).context,raw:__RAW__});
(await until(()=>document.querySelector('.deck-key'),'key')).click();
const images=()=>[...document.querySelectorAll('.sequence-row img.sequence-preview')];
await until(()=>images().length===2&&images().every(i=>i.complete&&i.naturalWidth>0),'child image decoding');
const initial=await parent(), first=initial.multi_children[0][0], second=initial.multi_children[0][1];
const firstSource=images()[0].src, secondSource=images()[1].src;
let nonce=0;
const send=async commands=>{const p=await parent(),child=p.multi_children[0][0];await command({command:'settings',context:child.context,settings:{...child.settings,feedbackProbe:{nonce:'native-sequence-'+(++nonce),commands}}})};
const colors=(image,kind)=>{const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;const ctx=canvas.getContext('2d');ctx.drawImage(image,0,0);const data=ctx.getImageData(0,0,canvas.width,canvas.height).data;let count=0;for(let i=0;i<data.length;i+=4){const [r,g,b]=data.slice(i,i+3);if(kind==='alert'?r>70&&g>60&&b<Math.min(r,g)*.65:g>70&&g>r*1.3&&g>b+8)count++}return count};
await send([{event:'showAlert'}]);
await until(()=>images()[0]?.src!==firstSource&&images()[0]?.complete&&colors(images()[0],'alert')>30,'visible child warning');
if(images()[1].src!==secondSource||(await parent()).preview!==initial.preview)throw Error('Warning changed sibling or parent');
await send([{event:'showOk'}]);
await until(()=>images()[0]?.complete&&colors(images()[0],'ok')>30,'visible child success');
await send([{event:'setTitle',payload:{title:'Updated child'}}]);
await until(async()=> (await parent()).multi_children[0][0].title==='Updated child','live title');
await until(async()=>{const p=await parent();return images()[0]?.complete&&colors(images()[0],'ok')===0&&p.multi_children[0][0].title==='Updated child'},'feedback expiry retains updated title');
if(images()[1].src!==secondSource||(await parent()).preview!==initial.preview)throw Error('Child display changed parent or sibling');
const rect=images()[0].getBoundingClientRect();if(rect.width<30||rect.height<30)throw Error('Child preview too small');
return {native_child_images:true,warning:true,success:true,expiry_keeps_latest_title:true,sibling_unchanged:true,parent_unchanged:true,usb_access:false,dimensions:[images()[0].naturalWidth,images()[0].naturalHeight]};
'''


def main():
    project=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args(); binary=args.binary.resolve(strict=True); output=args.output.resolve(); output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-sequence-feedback-') as directory:
        root=Path(directory)
        plugin,_=make_plugin(root/(PRIMARY+'.sdPlugin'),PRIMARY,project.parent/'evidence/official-samples/counter/package.json')
        raw={'Actions':[{'Actions':[child('key','First step'),child('key','Second step')]},{'Actions':[]}]}
        for entry in raw['Actions'][0]['Actions']:
            for state in entry['States']: state.pop('Title', None)
        script=root/'fixture.js'; script.write_text(SCRIPT.replace('__PLUGIN__',json.dumps(PRIMARY)).replace('__RAW__',json.dumps(raw)))
        proof=root/'proof.json'; env=environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(output/'native.png'))
        with (output/'native.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0','--plugin',str(plugin)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=35)==0,(output/'native.log').read_text()[-5000:]
                report=result(proof); assert report,(output/'native.log').read_text()[-5000:]
                report['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
                (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
            finally:cleanup(process)
    print('PASS: native Multi Action child previews, warning/success, latest-title restore, sibling/parent isolation; no USB.')

if __name__=='__main__':main()
