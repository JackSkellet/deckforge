#!/usr/bin/env python3
"""Native folder settings, Counter execution, cancellation, export and restart. No USB."""
import argparse,hashlib,json,subprocess,tempfile,zipfile
from pathlib import Path
from native_editor_interop import environment,result,cleanup
from sample_interop import COUNTER_LOCK_SHA256

SCRIPT=r'''
const b=window.deckforge, command=p=>b.request('command',p), snapshot=()=>b.request('snapshot');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,label)=>{for(let i=0;i<160;i++){const v=await fn();if(v)return v;await pause(35)}throw Error('Timed out: '+label)};
const counter=async()=>(await snapshot()).slots.find(s=>s.uuid==='com.elgato.counter.action');
const back=async()=>(await snapshot()).slots.find(s=>s.uuid==='com.elgato.streamdeck.profile.backtoparent');
const key=(index,pressed)=>command({command:'input',event:pressed?'keyDown':'keyUp',index});
const tap=async i=>{await key(i,true);await key(i,false)};
const depth=async n=>until(async()=>(await snapshot()).library.folder_depth===n,'folder depth '+n);
await until(async()=>{const s=await snapshot();if(!s.simulate)throw Error('Simulation required');return s.plugins.some(p=>p.id==='com.elgato.counter'&&p.connected)},'Counter registration');
if(__RESTART__){
 await tap(0);await depth(1);const a=await until(async()=>{const c=await counter();return c?.title===String(c.settings.count)&&c},'restored Counter');
 if((await back()).raw.AutoExit.Delay!==0||!a.raw.AutoExit.Enabled||a.raw.AutoExit.Delay!==1)throw Error('AutoExit settings lost');
 document.querySelector('.deck-key').click();
 const summary=await until(()=>document.querySelector('.folder-exit-options summary'),'restored settings');if(!summary.parentElement.open)summary.click();
 await pause(100);
 return {restart_preserved:true,count:a.settings.count,usb_access:false};
}
await command({command:'folder_create',name:'Sound effects',index:0});await tap(0);await depth(1);
await command({command:'assign',controller:'Keypad',index:1,uuid:'com.elgato.counter.action'});
await until(async()=>(await counter())?.title==='0','Counter appeared');
const check=name=>[...document.querySelectorAll('.folder-exit-options label')].find(l=>l.textContent.trim()===name)?.querySelector('input');
const field=()=>document.querySelector('.folder-exit-options input[type=number]');
const set=(el,v)=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,String(v));el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));};
const select=async i=>{(await until(()=>document.querySelectorAll('.deck-key')[i],'key editor')).click();const summary=await until(()=>document.querySelector('.folder-exit-options summary'),'auto-exit editor');if(!summary.parentElement.open)summary.click();await pause(60)};
const save=async()=>{const btn=await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Save changes'&&!b.disabled),'Save');btn.click();await until(()=>btn.disabled,'save complete')};
await select(0);check('Automatically exit folder').click();await pause(50);set(field(),0);await save();
await until(async()=>(await back()).raw.AutoExit?.Enabled&&(await back()).raw.AutoExit.Delay===0,'folder default');
await tap(1);await depth(0);await tap(0);await depth(1);
await until(async()=>(await counter()).settings.count===1,'Counter retained after immediate exit');
await select(1);check('Follow folder settings').click();await pause(50);check('Automatically exit folder').click();await save();
await until(async()=>(await counter()).raw.AutoExit?.Enabled===false,'disabled override');
await tap(1);await pause(200);if((await snapshot()).library.folder_depth!==1)throw Error('Disabled override exited');
await select(1);check('Automatically exit folder').click();await pause(50);set(field(),1);await save();
await until(async()=>(await counter()).raw.AutoExit?.Enabled&&(await counter()).raw.AutoExit.Delay===1,'delayed override');
await key(1,true);await pause(1100);if((await snapshot()).library.folder_depth!==1)throw Error('Held press started timer');
await key(1,false);await pause(350);await key(1,true);await pause(1000);
if((await snapshot()).library.folder_depth!==1)throw Error('New press did not cancel timer');
const start=performance.now();await key(1,false);await depth(0);const elapsed=performance.now()-start;
if(elapsed<900||elapsed>1800)throw Error('Wrong auto-exit delay '+elapsed);
await command({command:'export_profile',path:__EXPORT__});
return {native_settings:true,immediate_exit:true,disabled_override:true,held_cancellation:true,delay_ms:elapsed,usb_access:false};
'''

def main():
 project=Path(__file__).resolve().parents[1]; ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge');ap.add_argument('--output',type=Path,required=True);args=ap.parse_args();binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
 sample=project.parent/'evidence/official-samples/counter';assert hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
 report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'phases':[]}
 with tempfile.TemporaryDirectory(prefix='deckforge-native-folder-exit-') as directory:
  root=Path(directory);export=root/'folders.streamDeckProfile'
  for restart in [False,True]:
   name='restart' if restart else 'folder';script=root/(name+'.js');script.write_text(SCRIPT.replace('__RESTART__',json.dumps(restart)).replace('__EXPORT__',json.dumps(str(export))));proof=root/(name+'.json');env=environment(root)
   env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(output/(name+'.png')))
   with (output/(name+'.log')).open('w') as log:
    process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0','--plugin',str(sample/'com.elgato.counter.sdPlugin')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     assert process.wait(timeout=40)==0,(output/(name+'.log')).read_text()[-4000:]
     value=result(proof);assert value;report['phases'].append(value['result'])
    finally:cleanup(process)
  with zipfile.ZipFile(export) as z:
   records=[a for n in z.namelist() if n.endswith('manifest.json') for c in json.loads(z.read(n)).get('Controllers',[]) for a in (c.get('Actions') or {}).values()]
  assert any(a.get('UUID')=='com.elgato.counter.action' and a.get('AutoExit')=={'Enabled':True,'Delay':1} for a in records)
  assert any(a.get('UUID')=='com.elgato.streamdeck.profile.backtoparent' and a.get('AutoExit')=={'Enabled':True,'Delay':0} for a in records)
 report['export_preserved']=True;report['ok']=True;(output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
 print('PASS: native folder auto-exit controls, unchanged Counter, release timing/cancellation, profile export and restart; no USB.')
if __name__=='__main__':main()
