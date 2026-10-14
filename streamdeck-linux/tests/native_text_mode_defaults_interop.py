#!/usr/bin/env python3
"""Native editor shows native defaults on import and preserves existing Linux paste settings."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment,result
from text_mode_defaults_interop import fixture

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<180;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const slot=async(index=0)=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===index);
const mode=()=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent==='Text mode')?.querySelector('select');
if(!(await snapshot()).simulate)throw Error('Private simulation required');
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
const original=await slot(),legacyProfile=(await snapshot()).library.active_profile;
const legacy={pastedText:'Existing Linux paste',Future:{keep:9}};
await cmd({command:'settings',context:original.context,settings:legacy});
await until(()=>mode()?.value==='paste','legacy paste mode');
await cmd({command:'import_profile',path:__SOURCE__});
await until(()=>mode()?.value==='typing','native imported mode');
const imported=await slot(),importedProfile=(await snapshot()).library.active_profile;
if(imported.settings.isTypingMode!==true||imported.settings.FutureSetting!==7)throw Error('Native import did not preserve settings/default');
if((await snapshot()).typing_status.phase!=='disabled')throw Error('Import requested keyboard permission');
await until(()=>[...document.querySelectorAll('button')].some(b=>b.textContent==='Enable desktop typing'),'explicit enable control');
document.querySelectorAll('.deck-key')[7].click();await until(()=>mode()?.value==='invalid','unsupported saved mode');
if((await slot(7)).settings.isTypingMode!==null||mode().getAttribute('aria-invalid')!=='true')throw Error('Invalid mode was coerced or not identified');
document.querySelectorAll('.deck-key')[0].click();await until(()=>mode()?.value==='typing','native mode selected again');
await cmd({command:'profile_select',id:legacyProfile});await until(()=>mode()?.value==='paste','original paste restored');
if(JSON.stringify((await slot()).settings)!==JSON.stringify(legacy)){
 const s=(await slot()).settings;if(s.pastedText!==legacy.pastedText||s.Future.keep!==9||'isTypingMode' in s)throw Error('Existing settings changed');
}
await cmd({command:'export_profile',path:__EXPORT__});
if('isTypingMode' in (await slot()).settings)throw Error('Export mutated existing settings');
await cmd({command:'profile_select',id:importedProfile});await until(()=>mode()?.value==='typing','final imported mode');
return {nativeMode:true,existingModePreserved:true,exportDidNotMutate:true,invalidModePreservedAndIdentified:true,permissionExplicit:true,actionsExecuted:false};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/text-defaults/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='deckforge-native-text-defaults-') as d:
        root=Path(d);source=root/'native.streamDeckProfile';fixture(source)
        script=root/'test.js';script.write_text(SCRIPT.replace('__SOURCE__',json.dumps(str(source))).replace('__EXPORT__',json.dumps(str(root/'export.streamDeckProfile'))));proof=root/'proof.json';env=environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/'imported.png'))
        with (a.output/'native.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                code=process.wait(timeout=30);report=result(proof);assert code==0 and report and report.get('ok'),report
            finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'report':report,'actions_executed':False,'usb_access':False},indent=2)+'\n')
    print('PASS: native import displays typing, existing legacy settings still display paste, export does not mutate settings, and permission remains explicit. No actions executed.')

if __name__=='__main__':main()
