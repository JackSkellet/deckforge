#!/usr/bin/env python3
"""Edit and save real native built-in forms without executing any action."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,label)=>{for(let i=0;i<160;i++){const x=await fn();if(x)return x;await pause(40)}throw Error('Timed out: '+label)};
const bridge=window.deckforge;
const snapshot=()=>bridge.request('snapshot');
const cmd=(p)=>bridge.request('command',p);
const field=(name)=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select,textarea');
const check=(name)=>[...document.querySelectorAll('label.check')].find(l=>l.textContent.trim()===name)?.querySelector('input');
const set=(el,value)=>{Object.getOwnPropertyDescriptor(el.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(el,String(value));el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));};
const slot=()=>snapshot().then(s=>s.slots.find(x=>x.controller==='Keypad'&&x.index===0));
async function assign(uuid){await cmd({command:'assign',controller:'Keypad',index:0,uuid});return until(async()=>{const a=await slot();return a?.uuid===uuid&&a},uuid)}
async function save(predicate,label){const button=await until(()=>{const b=document.querySelector('.inspector-footer button[type="submit"]');return b&&!b.disabled&&b},'save enabled '+label);button.click();return until(async()=>{const a=await slot();return predicate(a)&&a},'saved '+label)}
let state=await snapshot();if(!state.simulate)throw Error('Simulation required');
await until(async()=>{const s=await snapshot();return ['com.elgato.counter','com.elgato.hello-world'].every(id=>s.plugins.some(p=>p.id===id&&p.connected))},'official sample registration');
const originalPage=state.library.active_page;
await cmd({command:'page_create',name:'Controls fixture'});
let a=await assign('com.elgato.streamdeck.system.website');
set(await until(()=>field('Website address'),'website field'),'https://example.invalid/never-opened');
(await until(()=>check('Open in a browser'),'browser check')).click();
a=await save(a=>a.settings.path==='https://example.invalid/never-opened'&&a.settings.openInBrowser===false,'website');
const website=a.settings;
a=await assign('com.elgato.streamdeck.profile.openchild');
set(await until(()=>field('Folder page'),'folder picker'),originalPage);
a=await save(a=>a.settings.ProfileUUID===originalPage,'folder target');
const folder=a.settings;
a=await assign('com.elgato.streamdeck.system.hotkey');
await cmd({command:'settings',context:a.context,settings:{Future:{keep:true},Hotkeys:[{QTKeyCode:48,VKeyCode:96,NativeCode:96,KeyModifiers:0x4002,KeyCtrl:true,FutureChord:7}]}});
const key=await until(()=>{const e=field('Key');return e?.selectedOptions[0]?.textContent==='Numpad 0'&&e},'imported numpad field');
set(key,'none');
a=await save(a=>a.settings.Hotkeys[0].QTKeyCode===0x01ffffff&&a.settings.Hotkeys[0].KeyModifiers===2,'modifier-only shortcut');
if(a.settings.Hotkeys[0].NativeCode!==undefined||a.settings.Hotkeys[0].FutureChord!==7||!a.settings.Future.keep)throw Error('Shortcut edit lost unknown data or retained native identity');
const hotkey=a.settings;
a=await assign('com.elgato.streamdeck.soundboard.playaudio');
await cmd({command:'settings',context:a.context,settings:{path:'/not-opened.wav',actionType:1,volume:50,fadeType:0,fadeLen:1,outputType:'foreign-windows-device',Future:99}});
await until(()=>document.querySelector('.settings-fields .warning')?.textContent.includes('no Linux mapping'),'foreign output warning');
set(field('Audio output'),'');
a=await save(a=>a.settings.linuxOutput==='','explicit default audio output');
if(a.settings.outputType!=='foreign-windows-device'||a.settings.Future!==99)throw Error('Audio mapping lost source settings');
const audio=a.settings;
a=await assign('com.elgato.streamdeck.multiactions.routine');
await cmd({command:'multi_save',context:a.context,raw:{Actions:[{Actions:[{ActionID:'fixture-step',UUID:'com.elgato.streamdeck.page.next',Name:'Fixture next',Settings:{},States:[{},{}],OverrideState:1,FutureChild:'keep'}]},{Actions:[]}]}});
(await until(()=>[...document.querySelectorAll('.sequence-select')].find(b=>b.textContent.includes('Fixture next')),'sequence child')).click();
set(await until(()=>field('State'),'child state'),'');
(await until(()=>check('Custom action timing'),'timing check')).click();
set(await until(()=>field('Hold duration (milliseconds)'),'hold field'),25);
await pause(60);
set(field('Wait after action (milliseconds)'),40);
a=await save(a=>{const c=a.raw.Actions[0].Actions[0];return c.MultiActionDelays?.Delay1===25&&c.MultiActionDelays?.Delay2===40&&c.OverrideState===undefined},'sequence timing/default state');
const child=a.raw.Actions[0].Actions[0];if(child.FutureChild!=='keep')throw Error('Timing edit lost child metadata');
a=await assign('com.elgato.counter.action');
const increment=await until(()=>field('Increment by'),'counter empty-settings control');
if(increment.value!=='1'||field('count'))throw Error('Counter defaults/runtime field mismatch');
set(increment,3);
a=await save(a=>a.settings.incrementBy===3,'counter increment');
const counter=a.settings;
a=await assign('com.elgato.hello-world.newstring');
const initialHello=await until(async()=>{const a=await slot();return a.settings.titleKey==='clickme'&&a.title&&a},'official Hello World initialization');
const helloTrace={initialSettings:initialHello.settings,initialTitle:initialHello.title};
set(await until(()=>field('Sample language'),'Hello World language'),'fr');
helloTrace.editedDraft=(await bridge.request('ui_state_get'))?.drafts?.[initialHello.context];
a=await save(a=>a.settings.language==='fr','sample explicit language');
helloTrace.savedSettings=a.settings;
helloTrace.savedDraft=(await bridge.request('ui_state_get'))?.drafts?.[a.context];
helloTrace.runtimeFieldExposed=!!field('titleKey');
if(a.settings.titleKey!=='clickme')throw Error('Hello World runtime key LOST: '+JSON.stringify(helloTrace));
if(helloTrace.runtimeFieldExposed)throw Error('Hello World runtime field EXPOSED: '+JSON.stringify(helloTrace));
const french=a.settings;
const frenchTitle=await until(async()=>{const a=await slot();return a.title.startsWith('Français\n')&&a.title},'official localized title update');
await until(async()=>{const ui=await bridge.request('ui_state_get');return ui?.drafts?.[a.context]?.dirty===false&&field('Sample language')?.value==='fr'},'sample language saved draft');
set(field('Sample language'),'');
a=await save(a=>a.settings.language===undefined,'sample host language');
const restoredTitle=await until(async()=>{const a=await slot();return a.title===initialHello.title&&a.title},'official host-language title restore');
return {website,folder,hotkey,audio,sequence:child,counter,helloWorld:{french,host:a.settings,frenchTitle,restoredTitle,trace:helloTrace},actionsExecuted:0,hardwareWrites:0};
'''

def main():
    project=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/debug/deckforge')
    p.add_argument('--report',type=Path)
    p.add_argument('--samples',type=Path,default=project.parent/'evidence/official-samples')
    args=p.parse_args()
    binary=args.binary.resolve(strict=True)
    plugins=[args.samples/'counter/com.elgato.counter.sdPlugin',args.samples/'hello-world/com.elgato.hello-world.sdPlugin']
    assert all((p/'bin/plugin.js').is_file() for p in plugins), 'Build the unchanged official sample plugins first'
    with tempfile.TemporaryDirectory(prefix='deckforge-native-controls-') as d:
        root=Path(d);script=root/'controls.js';proof=root/'result.json'
        script.write_text(SCRIPT)
        env=environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(root/'controls.png'))
        log_path=root/'host.log'
        with log_path.open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0',*[arg for plugin in plugins for arg in ('--plugin',str(plugin.resolve()))]],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=35)==0,log_path.read_text()
                if proof.exists() and not json.loads(proof.read_text()).get('ok'):
                    failure={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'proof':json.loads(proof.read_text()),'log':log_path.read_text()}
                    if args.report:
                        args.report.parent.mkdir(parents=True,exist_ok=True)
                        args.report.write_text(json.dumps(failure,indent=2)+'\n')
                    raise AssertionError(failure['proof'])
                output=result(proof)
                assert output,log_path.read_text()
            finally:cleanup(process)
        report={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'proof':output,'boundary':'Actual native WebKit forms and Rust settings persistence. Isolated simulation; no actions executed, no keyboard/audio/USB operations.'}
        if args.report:
            args.report.parent.mkdir(parents=True,exist_ok=True)
            args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: native website, folder target, modifier-only hotkey, explicit soundboard output, Multi Action timing/default state, official Counter increment and Hello World language saves. No action execution.')

if __name__=='__main__':main()
