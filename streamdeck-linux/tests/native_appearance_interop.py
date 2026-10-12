#!/usr/bin/env python3
"""Native React appearance preview/save against the unchanged Counter sample.
App-owned WebKit instrumentation; no OS input, USB or real user state.
"""
import argparse
import hashlib
import base64
import io
import json
from pathlib import Path
import subprocess
import tempfile
from PIL import Image
from native_editor_interop import environment, cleanup, result
from sample_interop import COUNTER_LOCK_SHA256

SCRIPT=r'''
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async fn=>{for(let i=0;i<200;i++){const v=await fn();if(v)return v;await pause(40)}throw Error('Appearance fixture condition timed out')};
const bridge=window.deckforge;
await until(async()=>{const s=await bridge.request('snapshot');if(!s.simulate)throw Error('Simulation required');return s.plugins.some(p=>p.id==='com.elgato.counter'&&p.connected)});
await bridge.request('command',{command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.counter.action'});
const counter=()=>bridge.request('snapshot').then(s=>s.slots.find(v=>v.index===0&&v.controller==='Keypad'));
let slot=await until(async()=>{const s=await counter();return s?.title==='0'&&s});
await bridge.request('command',{command:'settings',context:slot.context,settings:{count:4,incrementBy:3}});
const press=async()=>{await bridge.request('command',{command:'input',event:'keyDown',index:0});await bridge.request('command',{command:'input',event:'keyUp',index:0});};
await press();slot=await until(async()=>{const s=await counter();return s.title==='7'&&s});
const before=slot.preview;
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const options=await until(()=>document.querySelector('.appearance-options summary'));options.click();
const fonts=await bridge.request('fonts');if(!Array.isArray(fonts)||!fonts.length)throw Error('Font inventory unavailable');
await until(()=>field('Font size'));
const set=(input,value)=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,value);input.dispatchEvent(new Event('input',{bubbles:true}));input.dispatchEvent(new Event('change',{bubbles:true}));};
set(document.querySelector('[aria-label="Title colour value"]'),'#00ff00');
set(field('Font size'),'20');
await until(()=>document.querySelector('img[alt="Draft appearance preview"]')?.complete);
slot=await counter();if(slot.preview!==before||slot.appearance.title_parameters.titleColor==='#00ff00')throw Error('Preview wrote live appearance');
// Runtime settings change while the appearance draft is open. Saving style must
// retain this new count, not the settings captured when editing began.
await press();await until(async()=>{const s=await counter();return s.title==='10'&&s.settings.count===10});
const save=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Save changes');
await until(()=>!save.disabled);save.click();
slot=await until(async()=>{const s=await counter();return s.appearance.title_parameters.titleColor==='#00ff00'&&s});
if(slot.settings.count!==10||slot.title!=='10'||slot.appearance.title_custom)throw Error('Appearance save changed live plugin settings/title ownership');
if(slot.appearance.title_parameters.fontSize!==20)throw Error('Font size did not save');
return {previewReadOnly:true,livePluginCountPreserved:true,titleOwnedByPlugin:true,settings:slot.settings,title:slot.title,style:slot.appearance.title_parameters,fontFamilies:fonts.length,baseline_preview:before,final_preview:slot.preview};
'''

def main():
    project=Path(__file__).resolve().parents[1]
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    ap.add_argument('--capture',type=Path)
    ap.add_argument('--report',type=Path)
    args=ap.parse_args();binary=args.binary.resolve(strict=True)
    counter=project.parent/'evidence/official-samples/counter/com.elgato.counter.sdPlugin'
    assert hashlib.sha256((counter.parent/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
    with tempfile.TemporaryDirectory(prefix='deckforge-native-style-') as directory:
        root=Path(directory);script=root/'fixture.js';script.write_text(SCRIPT);proof=root/'result.json'
        capture=args.capture.resolve() if args.capture else root/'native.png';capture.parent.mkdir(parents=True,exist_ok=True)
        env=environment(root);env.update(DECKFORGE_CAPTURE=str(capture),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path=root/'native.log'
        with log_path.open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0','--plugin',str(counter)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=35)==0,log_path.read_text()
                report=result(proof);assert report,log_path.read_text()
                for key in ('baseline_preview','final_preview'):
                    image=Image.open(io.BytesIO(base64.b64decode(report['result'].pop(key)))).convert('RGB')
                    bright=sum(all(c>180 for c in image.getpixel((x,y))) for y in range(image.height) if y<25 or y>95 for x in range(image.width))
                    assert bright>50, 'Counter manifest icon disappeared outside the title area'
                report['result']['manifest_icon_preserved']=True
                saved=json.loads((root/'state/session.json').read_text())['slots'][0]
                assert saved['settings']['count']==10
                report.update(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),capture_sha256=hashlib.sha256(capture.read_bytes()).hexdigest())
                if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
            finally:cleanup(process)
    print('PASS: native appearance preview is read-only; explicit style save preserves live Counter updates and plugin title ownership.')

if __name__=='__main__':main()
