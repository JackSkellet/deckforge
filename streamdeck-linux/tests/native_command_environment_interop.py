#!/usr/bin/env python3
"""Native environment rows, literal values/unset, restart and profile export."""
import argparse,hashlib,json,subprocess,tempfile,zipfile
from pathlib import Path
from native_editor_interop import cleanup,environment,result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const click=async name=>(await until(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);return b&&!b.disabled&&b},name)).click();
const set=(e,value)=>{const proto=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const field=name=>document.querySelector(`[aria-label="${name}"]`);
const value="  $DECKFORGE_FORM_VALUE 'literal'\n🌍  ";
if(__PHASE__==='edit'){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.deckforge.system.command'});
 const environment=await until(()=>document.querySelector('.command-environment'),'environment editor');environment.open=true;
 await click('Add environment variable');
 set(await until(()=>field('Variable 1 name'),'first name'),'DECKFORGE_FORM_VALUE');await pause(50);
 set(field('Variable 1 value'),value);await pause(50);
 await click('Add environment variable');set(field('Variable 2 name'),'DECKFORGE_FORM_UNSET');await pause(50);
 set(field('Variable 2 value'),'keep while unset');await pause(50);
 document.querySelectorAll('.command-environment input[type="checkbox"]')[1].click();
 await click('Save changes');
 await until(async()=>(await slot()).settings.environment?.[1]?.unset===true,'saved environment');
}
const saved=await slot();
if(saved.settings.environment?.[0]?.value!==value||saved.settings.environment?.[1]?.value!=='keep while unset'||saved.settings.environment?.[1]?.unset!==true)throw Error('Environment settings changed');
const editor=await until(()=>document.querySelector('.command-environment'),'restored editor');editor.open=true;
await until(()=>field('Variable 2 value'),'restored fields');
if(!field('Variable 2 value').disabled||field('Variable 1 value').value!==value)throw Error('Restored controls lost literal value or unset');
await cmd({command:'export_profile',path:__EXPORT__});
field('Variable 1 value').scrollIntoView({block:'center'});
return {phase:__PHASE__,environment:saved.settings.environment,actionsExecuted:false};
'''

def has_environment(value,expected):
    if isinstance(value,dict):
        if value.get('UUID')=='com.deckforge.system.command' and value.get('Settings',{}).get('environment')==expected:return True
        return any(has_environment(v,expected) for v in value.values())
    return isinstance(value,list) and any(has_environment(v,expected) for v in value)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/command-environment/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-native-environment-') as d:
        root=Path(d);exported=root/'environment.streamDeckProfile'
        for phase in ['edit','restart']:
            script=root/'test.js';script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)).replace('__EXPORT__',json.dumps(str(exported))));proof=root/'proof.json';proof.unlink(missing_ok=True);env=environment(root)
            env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=30);report=result(proof);assert code==0 and report and report.get('ok'),report;reports.append(report)
                finally:cleanup(process)
        with zipfile.ZipFile(exported) as archive:
            assert any(has_environment(json.loads(archive.read(name)),reports[-1]['result']['environment']) for name in archive.namelist() if name.endswith('manifest.json')),'Profile export lost environment overrides'
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'reports':reports,'profile_export_preserved':True,'actions_executed':False,'usb_access':False},indent=2)+'\n')
    print('PASS: native environment rows, literal Unicode/multiline values, unset retains its value, restart and profile export. No actions executed.')

if __name__=='__main__':main()
