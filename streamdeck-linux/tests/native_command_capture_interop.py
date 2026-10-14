#!/usr/bin/env python3
"""Native output controls, captured command Stop, safe preview and unsaved-draft preservation."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment,result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const slot=async()=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===0);
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,textarea,select');
const set=(e,value)=>{const proto=e.tagName==='SELECT'?HTMLSelectElement.prototype:e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,value);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));};
const click=async name=>(await until(()=>{const b=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);return b&&!b.disabled&&b},name)).click();
async function save(script){set(await until(()=>field('Shell script'),'script'),script);await click('Save changes');await until(async()=>(await slot()).settings.script===script,'saved script');}
await until(async()=>(await snapshot()).simulate,'simulation');
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.deckforge.system.command'});
set(await until(()=>field('Command mode'),'mode'),'shell');
await until(()=>field('Command output'),'output controls');
if(field('Command output').value!=='show'||field('Timeout (seconds)').value!=='30')throw Error('New command defaults are not captured');
await save('sleep 5 & wait');await click('Test press');
await until(async()=>(await slot()).command_run?.state==='running','running');
await click('Stop command');await until(async()=>(await slot()).command_run?.state==='cancelled','stopped');
await save("printf '<b>stdout 🌍</b>\\n'; printf 'stderr text' >&2");await click('Test press');
await until(async()=>(await slot()).command_run?.state==='succeeded','completed');
const done=await slot();
if(!done.command_run.can_copy)throw Error('Complete text cannot be copied');
set(field('Title'),'Keep this unsaved title');await pause(100);
await click('Show output');
const output=await until(()=>document.querySelector('[aria-label="Standard output"]'),'output preview');
if(output.textContent!=='<b>stdout 🌍</b>\n'||output.querySelector('b'))throw Error('Output is not exact escaped text');
if(document.querySelector('[aria-label="Standard error"]').textContent!=='stderr text')throw Error('stderr missing');
await click('Close output');
if(field('Title').value!=='Keep this unsaved title'||(await slot()).title==='Keep this unsaved title')throw Error('Viewing output changed the unsaved draft');
let stale=false;try{await b.request('command_output',{context:done.context,id:'old-run'})}catch{stale=true}
if(!stale)throw Error('Stale output request accepted');
await click('Show output');await until(()=>document.querySelector('[aria-label="Standard output"]'),'final preview');
return {stopControl:true,outputExact:true,stderrSeparate:true,escapedText:true,draftPreserved:true,staleRunRejected:true,clipboardUntouched:true,summary:done.command_run};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/command-capture/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='deckforge-native-command-capture-') as d:
        root=Path(d);script=root/'test.js';script.write_text(SCRIPT);proof=root/'proof.json';env=environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/'preview.png'))
        with (a.output/'native.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                code=process.wait(timeout=35);report=result(proof);assert code==0 and report and report.get('ok'),report
            finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'report':report,'usb_access':False},indent=2)+'\n')
    print('PASS: native command output settings, Stop, escaped stdout/stderr preview, unsaved draft preservation and stale-run rejection. Clipboard untouched; no USB.')

if __name__=='__main__':main()
