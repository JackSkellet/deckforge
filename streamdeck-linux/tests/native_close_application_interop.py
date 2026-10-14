#!/usr/bin/env python3
"""Native Close forms and restart, without executing configured actions."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<150;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const bridge=window.deckforge,snapshot=()=>bridge.request('snapshot'),cmd=p=>bridge.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(e,value)=>{Object.getOwnPropertyDescriptor(e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const check=()=>[...document.querySelectorAll('label')].find(l=>l.textContent.trim()==='Force close')?.querySelector('input');
async function save(method,force){
 (await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b&&!b.disabled&&b},'Save')).click();
 const saved=await until(async()=>{const s=await slot();return s.settings.linuxCloseMethod===method&&s.settings.forceQuit===force&&s},'saved '+method+' '+force);
 await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b?.disabled&&b.textContent==='Save changes'&&!document.querySelector('.inspector-fields')?.disabled},'save completed');
 return saved;
}
if(!(await snapshot()).simulate)throw Error('Simulation required');
const initial=await slot();
if(initial?.settings?.Future?.keep===9){
 if(initial.settings.linuxCloseMethod!=='terminate'||initial.settings.forceQuit!==false||initial.settings.linuxMatchArgs!=="'owned script' '$HOME'")throw Error('Restart lost settings');
 await until(()=>field('Close behavior')?.value==='terminate','restored form');
 field('Close behavior').scrollIntoView({block:'center'});await pause(80);
 return {phase:'restored',settings:initial.settings};
}
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.close'});
const fresh=await until(async()=>{const s=await slot();return s?.uuid?.endsWith('.close')&&s},'Close');
if(fresh.settings.linuxCloseMethod!=='windows')throw Error('New Close must request windows');
await cmd({command:'settings',context:fresh.context,settings:{...fresh.settings,Future:{keep:9}}});
await until(()=>[...document.querySelectorAll('.settings-fields textarea')].some(e=>e.value.includes('Future')),'external settings rendered');
set(await until(()=>field('Application executable'),'path'),'/not-executed/owned-app');await pause(60);
set(field('Match arguments'),"'owned script' '$HOME'");await pause(60);
for(const method of ['automatic','windows','terminate']){set(field('Close behavior'),method);await pause(60);await save(method,false);}
check().click();await pause(60);await save('terminate',true);
if(field('Close behavior'))throw Error('Dormant method should be hidden');
check().click();await until(()=>field('Close behavior')?.value==='terminate','retained method');
const saved=await save('terminate',false);
if(saved.settings.Future.keep!==9)throw Error('Opaque setting lost');
field('Close behavior').scrollIntoView({block:'center'});await pause(80);
return {phase:'saved',settings:saved.settings,all_methods_saved:true,force_preserves_dormant_method:true};
'''
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/system-close/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);runs=[]
 with tempfile.TemporaryDirectory(prefix='deckforge-close-native-') as d:
  root=Path(d);script=root/'probe.js';script.write_text(SCRIPT)
  for phase in ['saved','restored']:
   proof=root/(phase+'.json');env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
   with (a.output/(phase+'.log')).open('w') as log:
    process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
     code=process.wait(timeout=35);data=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No report'}
     assert code==0 and data.get('ok'),data
     assert data['result']['phase']==phase,data
     runs.append(data['result'])
    finally:cleanup(process)
 (a.output/'proof.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'runs':runs,'actions_executed':False},indent=2)+'\n')
 print('PASS: native Close modes, argument scope, dormant force settings, opaque settings and restart. No actions executed.')
if __name__=='__main__':main()
