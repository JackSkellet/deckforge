#!/usr/bin/env python3
"""Real cropper UI, in-process window draft restore, save/export and app restart."""
import argparse,json,subprocess,tempfile,zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup,environment,command,wait,result
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));const until=async(f,name)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timed out: '+name)};
const b=window.deckforge,cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot'),phase=__PHASE__;
let s=await snapshot();if(!s.simulate)throw Error('Requires simulated host');
if(phase!=='restart'&&!(await b.request('ui_state_get'))?.cropEdit?.path){
 let slot=s.slots.find(s=>s.controller==='Encoder'&&s.index===0);
 if(!slot){await cmd({command:'assign',controller:'Encoder',index:0,uuid:'com.elgato.streamdeck.system.hotkey'});s=await snapshot();slot=s.slots.find(s=>s.controller==='Encoder'&&s.index===0);}
 (await until(()=>document.querySelector('.deck-dial'),'dial canvas')).click();
 await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent==='Customize background…'),'crop entry');
 if(phase==='edit'){
  [...document.querySelectorAll('button')].find(b=>b.textContent==='Customize background…').click();await until(()=>document.querySelector('dialog[open]'),'blank crop dialog');
  if(![...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Set Image')?.disabled)throw Error('Save enabled without source');
  [...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Cancel').click();await until(()=>!document.querySelector('dialog[open]'),'cancel');
 }
 await pause(120);const state=await b.request('ui_state_get');
 await b.request('ui_state_set',{state:{...state,cropEdit:__DRAFT__??{device:s.device,context:slot.context,expected:slot.raw.Encoder?.background??null,path:__SOURCE__,frame:0}}});
 void b.request('_test_result',{ok:true,result:{phase:'seed'}});await b.request('window',{action:'hide'});await new Promise(()=>{});
}
if(phase!=='restart'){
 const save=()=>[...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Set Image');
 await until(()=>save()&&!save().disabled,'loaded crop draft');
 if(phase==='edit'){
  const input=()=>document.querySelector('dialog input[aria-label="Frame"]');
  const set=async value=>{const e=input();Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,String(value));e.dispatchEvent(new Event('input',{bubbles:true}));await until(async()=>!save().disabled&&(await b.request('ui_state_get')).cropEdit.frame===value-1,'frame change');};
  const sourcePixel=async()=>{const el=document.querySelector('.crop-viewport image');const image=new Image();image.src=el.getAttribute('href');await until(()=>image.complete&&image.naturalWidth>0,'source frame pixels');const c=document.createElement('canvas');c.width=20;c.height=20;const ctx=c.getContext('2d');ctx.drawImage(image,0,0);return ctx.getImageData(5,5,1,1).data;};
  await set(2);let pixel=await sourcePixel();if(pixel[1]<100||pixel[0]>5)throw Error('Frame2 is not green');await set(1);pixel=await sourcePixel();if(pixel[0]<250||pixel[1]>5)throw Error('Frame1 is not red');
  [...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Align to width').click();await until(async()=>(await b.request('ui_state_get')).cropEdit.rect.width===200,'width fit');
  [...document.querySelectorAll('dialog button')].find(b=>b.textContent==='Reset image').click();await until(async()=>(await b.request('ui_state_get')).cropEdit.rect.width===400,'cover reset');
  const svg=document.querySelector('.crop-viewport');svg.focus();for(let i=0;i<10;i++){svg.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',shiftKey:true,bubbles:true}));await pause(30);}
  const draft=await until(async()=>{const d=(await b.request('ui_state_get')).cropEdit;return d.rect.x===-200&&d},'keyboard pan');
  s=await snapshot();if(s.slots.find(v=>v.context===draft.context).raw.Encoder?.background)throw Error('Draft changed saved background');
  const rect=document.querySelector('dialog').getBoundingClientRect();if(rect.left<0||rect.right>innerWidth+1||rect.top<0||rect.bottom>innerHeight+1)throw Error('Crop dialog outside window');
  return {phase,draft,frame_picker:true,keyboard_pan:true,cancel:true,unsaved:true,window_draft_restored:true};
 }
 save().click();await until(()=>!document.querySelector('dialog[open]'),'saved crop closes');
}
s=await snapshot();const slot=s.slots.find(s=>s.controller==='Encoder'&&s.index===0);if(!slot?.raw.Encoder?.background)throw Error('Saved background missing');
const image=new Image();image.src='data:image/png;base64,'+slot.preview;await until(()=>image.complete,'preview');const canvas=document.createElement('canvas');canvas.width=200;canvas.height=100;const ctx=canvas.getContext('2d');ctx.drawImage(image,0,0);const pixel=ctx.getImageData(100,50,1,1).data;if(pixel[2]<250||pixel[0]>5)throw Error('Crop did not select blue half: '+[...pixel]);
if(phase==='save')await cmd({command:'export_profile',path:__EXPORT__});
return {phase,blue_crop:true,restored:phase==='restart',usb_access:false};
'''
def main():
 project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/cropper/native');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve()
 with tempfile.TemporaryDirectory(prefix='deckforge-cropper-') as d:
  root=Path(d);state=root/'state';source=root/'source.gif';export=root/'export.streamDeckProfile';first=Image.new('RGB',(400,100),'red');first.paste('blue',(200,0,400,100));first.save(source,save_all=True,append_images=[Image.new('RGB',(400,100),'green')],duration=[100,100],loop=0)
  draft=None;reports=[]
  for phase in ['edit','save','restart']:
   script=root/'test.js';proof=root/'proof.json';proof.unlink(missing_ok=True)
   script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__DRAFT__',json.dumps(draft)).replace('__SOURCE__',json.dumps(str(source))).replace('__EXPORT__',json.dumps(str(export))))
   env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
   with (a.output/(phase+'.log')).open('w') as log:
    process=subprocess.Popen([str(binary),'desktop','--simulate','--model','plus','--state',str(state)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     if phase!='restart':
      wait(lambda:result(proof,'seed'),'draft seed');wait(lambda:not command(binary,state,'status')['visible'],'owned editor hidden');assert process.poll() is None;command(binary,state,'show')
     code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
    finally:cleanup(process)
   if phase=='edit':draft=report['result']['draft']
   if phase=='save':source.unlink()
  with zipfile.ZipFile(export) as z:
   manifests=[(n,json.loads(z.read(n))) for n in z.namelist() if n.endswith('manifest.json')];found=False
   for name,r in manifests:
    for c in r.get('Controllers',[]):
     for action in c.get('Actions',{}).values():
      path=action.get('Encoder',{}).get('background')
      if path:
       import io
       pixels=Image.open(io.BytesIO(z.read(name.removesuffix('manifest.json')+path))).convert('RGBA');assert pixels.size==(200,100) and pixels.getpixel((100,50))==(0,0,255,255);found=True
   assert found
  (a.output/'native.json').write_text(json.dumps({'runs':reports,'portable_png_crop':True,'restart_without_source':True,'usb_access':False},indent=2)+'\n')
 print('PASS: native crop dialog, cancel, frame picker, keyboard pan, window draft restoration, saved crop, export and restart; no USB.')
if __name__=='__main__':main()
