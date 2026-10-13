#!/usr/bin/env python3
"""Actual native Action Wheel UI and unchanged Counter, isolated from user state/USB."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import environment,result,cleanup
from sample_interop import COUNTER_LOCK_SHA256

SCRIPT=r'''
const b=window.deckforge, command=payload=>b.request('command',payload), snapshot=()=>b.request('snapshot');
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,label)=>{for(let i=0;i<180;i++){const v=await fn();if(v)return v;await pause(35)}throw Error('Timed out: '+label)};
const wheel=async()=>(await snapshot()).slots.find(s=>s.uuid==='com.elgato.streamdeck.keys.stack');
await until(async()=>{const s=await snapshot();if(!s.simulate)throw Error('Simulation required');return s.plugins.some(p=>p.id==='com.elgato.counter'&&p.connected)},'Counter registration');
if(__RESTART__){const w=await until(async()=>{const w=await wheel();return w?.multi_children[0][0]?.title==='4'&&w},'restored Counter');if(!w.settings.InvertRotation||!w.settings.ExtTitles||w.settings.CurrentIdx!==0)throw Error('Wheel options lost on restart');return {restart_preserved:true,counts:w.multi_children[0].map(c=>c.settings.count??0),usb_access:false};}
await command({command:'assign',controller:'Encoder',index:0,uuid:'com.elgato.streamdeck.keys.stack'});
await command({command:'wheel_save',context:(await wheel()).context,raw:{Settings:{CurrentIdx:0,ExtTitles:false,InvertRotation:false,ControlOffset:0},Actions:[3,7].map((count,i)=>({UUID:'com.elgato.counter.action',ActionID:'counter-'+i,Name:'Counter '+(i+1),Settings:{count,incrementBy:1},States:[{}]}))}});
await until(async()=>{const w=await wheel();return w.multi_children[0].every((c,i)=>c.title===String([3,7][i]))},'child titles');
(await until(()=>document.querySelector('.deck-dial'),'dial')).click();
const zone=await until(()=>document.querySelector('[aria-label="Action wheel"]'),'wheel editor');
const transfer=new DataTransfer();transfer.setData('application/x-deckforge-action','com.elgato.counter.action');
const over=new DragEvent('dragover',{bubbles:true,cancelable:true,dataTransfer:transfer});zone.dispatchEvent(over);if(!over.defaultPrevented)throw Error('Wheel did not accept library drag');
zone.dispatchEvent(new DragEvent('drop',{bubbles:true,cancelable:true,dataTransfer:transfer}));
(await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Save changes'&&!b.disabled),'save dropped child')).click();
await until(async()=>{const w=await wheel();return w.multi_children[0].length===3&&w.multi_children[0][2].title==='0'},'library drop saved');

const strip=()=>document.querySelector('.deck-dial > img');
const previews=()=>[...document.querySelectorAll('.sequence-preview')];
await until(()=>strip()?.complete&&strip().naturalWidth===200&&previews().length===3&&previews().every(i=>i.complete&&i.naturalWidth>0),'native strip and child previews');
const original=strip().src;
await command({command:'input',event:'dialRotate',index:0,ticks:1});
await until(async()=>{const w=await wheel();return w.settings.CurrentIdx===1&&strip().src!==original},'dial selection redraw');
(await until(()=>document.querySelector('[aria-label="Show action 1 on dial"]'),'Show control')).click();
await until(async()=>(await wheel()).settings.CurrentIdx===0,'Show selects saved child');
document.querySelector('.wheel-options summary').click();
const check=text=>[...document.querySelectorAll('label')].find(l=>l.textContent.includes(text))?.querySelector('input[type=checkbox]');
(await until(()=>check('Reverse rotation'),'reverse option')).click();
check('Show selected title across the dial display').click();
const save=await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Save changes'&&!b.disabled),'save wheel draft');save.click();
await until(async()=>{const w=await wheel();return w.settings.InvertRotation&&w.settings.ExtTitles},'wheel options saved');
document.querySelector('.wheel-options summary').click();
await command({command:'input',event:'dialDown',index:0});await command({command:'input',event:'dialUp',index:0});
const w=await until(async()=>{const w=await wheel();return w.multi_children[0][0].settings.count===4&&w.multi_children[0][0].title==='4'&&w},'Counter consumed wheel push').catch(async error=>{const w=await wheel();throw Error(String(error)+' '+JSON.stringify({selected:w.settings.CurrentIdx,children:w.multi_children[0].map(c=>({count:c.settings.count,title:c.title,appearance:c.appearance}))}))});
if(w.multi_children[0][1].settings.count!==7)throw Error('Sibling Counter changed');
await until(()=>strip().src==='data:image/png;base64,'+w.preview&&strip().complete,'extended title frame');
return{native_strip:[strip().naturalWidth,strip().naturalHeight],child_previews:previews().length,physical_input_simulated:true,show_control:true,options_saved:true,counter_counts:[4,7,0],library_drop:true,usb_access:false};
'''

def main():
 project=Path(__file__).resolve().parents[1]; ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge');ap.add_argument('--output',type=Path,required=True);args=ap.parse_args();binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
 sample=project.parent/'evidence/official-samples/counter';assert hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
 report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'phases':[]}
 with tempfile.TemporaryDirectory(prefix='deckforge-native-wheel-') as directory:
  root=Path(directory)
  for restart in [False,True]:
   name='restart' if restart else 'wheel';script=root/(name+'.js');script.write_text(SCRIPT.replace('__RESTART__',json.dumps(restart)));proof=root/(name+'.json');env=environment(root)
   env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(output/(name+'.png')))
   with (output/(name+'.log')).open('w') as log:
    process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0','--plugin',str(sample/'com.elgato.counter.sdPlugin')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     assert process.wait(timeout=35)==0,(output/(name+'.log')).read_text()[-4000:]
     value=result(proof);assert value;report['phases'].append(value['result'])
    finally:cleanup(process)
 report['ok']=True;(output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
 print('PASS: native Action Wheel cards, selection/Show control, saved options, unchanged Counter input and restart; no USB.')
if __name__=='__main__':main()
