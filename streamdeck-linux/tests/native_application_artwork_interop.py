#!/usr/bin/env python3
"""Native rendering/restart of automatic app artwork, with one owned process."""
import argparse,hashlib,json,shutil,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f)=>{for(let i=0;i<180;i++){const v=await f();if(v)return v;await pause(50)}throw Error('Application artwork timed out')};
const bridge=window.deckforge, snap=()=>bridge.request('snapshot'), cmd=p=>bridge.request('command',p);
const slot=async()=>(await snap()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
if(!(await snap()).simulate)throw Error('Simulation required');
const prior=await slot();
if(!prior){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.openapp'});
 const s=await slot();await cmd({command:'settings',context:s.context,settings:CONFIG});
}
const active=await until(async()=>{const s=await slot();return s?.application_running===true&&s});
if(active.appearance.image_custom)throw Error('Automatic icon incorrectly marked custom');
const image=await until(()=>document.querySelector('.deck-key img'));
await until(()=>image.complete&&image.naturalWidth>0);
const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;const c=canvas.getContext('2d');c.drawImage(image,0,0);
await until(()=>{c.drawImage(image,0,0);const pixel=c.getImageData(11,11,1,1).data;return pixel[0]===141&&pixel[1]===216&&pixel[2]===188});
const preview = await bridge.request('appearance_preview',{context:active.context,expected_state:active.state,state:active.state,reset_image:true});
const probe = new Image();probe.src=preview.preview;await until(()=>probe.complete&&probe.naturalWidth>0);canvas.width=probe.naturalWidth;canvas.height=probe.naturalHeight;c.drawImage(probe,0,0);
let pixel=c.getImageData(11,11,1,1).data;if(pixel[0]!==141||pixel[1]!==216||pixel[2]!==188)throw Error('Appearance reset preview lost automatic artwork');
const overridden=await bridge.request('appearance_preview',{context:active.context,expected_state:active.state,state:active.state,image_path:CUSTOM});probe.src=overridden.preview;await until(()=>probe.complete&&probe.naturalWidth>0);c.drawImage(probe,0,0);pixel=c.getImageData(11,11,1,1).data;if(pixel[0]!==255||pixel[1]!==0||pixel[2]!==0||!overridden.image_custom)throw Error('Custom preview overwritten by automatic artwork');
await pause(120);
return {phase:prior?'restored':'saved',automatic_icon:true,running_indicator_pixels:true,settings:active.settings};
'''
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/application-artwork/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);runs=[]
 with tempfile.TemporaryDirectory(prefix='deckforge-app-art-native-') as d:
  root=Path(d);custom=root/'custom.svg';custom.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#ff0000"/></svg>');exe=root/'owned-sleep';shutil.copy2('/usr/bin/sleep',exe);child=subprocess.Popen([str(exe),'90'])
  apps=root/'data/applications';apps.mkdir(parents=True);icon=root/'owned.svg';icon.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="8" fill="#514e92"/><path d="M16 23h32v24H16z" fill="#acb7ed"/><path d="M20 28l8 6-8 6m12 0h10" fill="none" stroke="#22263e" stroke-width="3"/></svg>');desktop=apps/'owned.desktop';desktop.write_text(f'[Desktop Entry]\nType=Application\nName=Owned artwork\nExec={exe} 90\nIcon={icon}\n')
  config={'exec':str(exe),'source':str(desktop),'args':'','bring_to_front':True,'long_press':'quit'};script=root/'probe.js';script.write_text('const CONFIG='+json.dumps(config)+';const CUSTOM='+json.dumps(str(custom))+';\n'+SCRIPT)
  try:
   for phase in ['saved','restored']:
    proof=root/(phase+'.json');env=environment(root);env['XDG_DATA_HOME']=str(root/'data');env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
    with (a.output/(phase+'.log')).open('w') as log:
     proc=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
     try:
      code=proc.wait(timeout=35);data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No report'};assert code==0 and data.get('ok'),data;assert data['result']['phase']==phase;runs.append(data['result'])
     finally:cleanup(proc)
   assert child.poll() is None,'Observation executed or closed the application'
  finally:
   if child.poll() is None:child.terminate()
   child.wait(timeout=3)
 (a.output/'proof.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':runs,'user_state_used':False,'usb_access':False},indent=2)+'\n')
 print('PASS: native automatic application icon/running pixels and restart, without executing the action.')
if __name__=='__main__':main()
