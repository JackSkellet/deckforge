#!/usr/bin/env python3
"""Observe animated artwork in native WebKit keys, Counter input and restart. No USB."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from PIL import Image
from native_editor_interop import environment,result,cleanup
from sample_interop import COUNTER_LOCK_SHA256

SCRIPT=r'''
const b=window.deckforge, command=p=>b.request('command',p), snapshot=()=>b.request('snapshot');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,label)=>{for(let i=0;i<160;i++){const v=await fn();if(v)return v;await pause(35)}throw Error('Timed out: '+label)};
const counter=async()=>(await snapshot()).slots.find(s=>s.uuid==='com.elgato.counter.action');
await until(async()=>{const s=await snapshot();if(!s.simulate)throw Error('Simulation required');return s.plugins.some(p=>p.id==='com.elgato.counter'&&p.connected)},'Counter registration');
if(!__RESTART__){
 await command({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.counter.action'});
 const a=await until(async()=>{const a=await counter();return a?.title==='0'&&a},'Counter ready');
 await command({command:'appearance',context:a.context,expected_state:a.state,image_path:__IMAGE__});
}
await until(()=>document.querySelector('.deck-key img')?.complete,'key image');
const colors=new Set(),frames=new Set();const start=performance.now();
while(performance.now()-start<700){
 const img=document.querySelector('.deck-key img');
 if(img?.complete&&img.naturalWidth){const c=document.createElement('canvas');c.width=img.naturalWidth;c.height=img.naturalHeight;const ctx=c.getContext('2d');ctx.drawImage(img,0,0);colors.add([...ctx.getImageData(0,0,1,1).data].slice(0,3).join(','));frames.add(img.src);}
 await pause(20);
}
if(!colors.has('255,0,0')||!colors.has('0,0,255'))throw Error('Native canvas did not animate: '+[...colors]);
if(!__RESTART__){
 await command({command:'input',event:'keyDown',index:0});await command({command:'input',event:'keyUp',index:0});
 await until(async()=>(await counter()).settings.count===1,'Counter consumed press during animation');
}else if((await counter()).settings.count!==1)throw Error('Counter settings lost at restart');
return {colors:[...colors],distinct_frames:frames.size,counter_count:(await counter()).settings.count,restart:__RESTART__,usb_access:false};
'''

def main():
 project=Path(__file__).resolve().parents[1];ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge');ap.add_argument('--output',type=Path,required=True);args=ap.parse_args();binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
 sample=project.parent/'evidence/official-samples/counter';assert hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
 report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'phases':[]}
 with tempfile.TemporaryDirectory(prefix='deckforge-native-animation-') as temporary:
  root=Path(temporary);gif=root/'motion.gif';a=Image.new('RGB',(32,32),(255,0,0));b=Image.new('RGB',(32,32),(0,0,255));a.save(gif,save_all=True,append_images=[b],duration=[80,120],loop=0)
  for restart in [False,True]:
   name='restart' if restart else 'animation';script=root/(name+'.js');script.write_text(SCRIPT.replace('__RESTART__',json.dumps(restart)).replace('__IMAGE__',json.dumps(str(gif))));proof=root/(name+'.json');env=environment(root)
   env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(output/(name+'.png')))
   with (output/(name+'.log')).open('w') as log:
    process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0','--plugin',str(sample/'com.elgato.counter.sdPlugin')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     assert process.wait(timeout=35)==0,(output/(name+'.log')).read_text()[-4000:]
     value=result(proof);assert value;report['phases'].append(value['result'])
    finally:cleanup(process)
   if not restart:gif.unlink()
 report['ok']=True;(output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('PASS: native animated key images, unchanged Counter input and restart after source deletion; no USB.')
if __name__=='__main__':main()
