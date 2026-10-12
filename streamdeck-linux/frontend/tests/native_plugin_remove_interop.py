#!/usr/bin/env python3
"""Native Remove controls against owned managed/external sample plugin copies.

All folders, profiles and private-data sentinels are temporary. Counter's backend
is unchanged; its PI is a local inert fixture. No USB/account/network operation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / "tests"))
from native_editor_interop import cleanup, environment, result
from native_multi_device_interop import saved_slots
from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT

COUNTER = "com.elgato.counter"
HELLO = "com.elgato.hello-world"
PI = '''<!doctype html><body>Owned removal inspector<script>window.connectElgatoStreamDeckSocket=(port,uuid,event)=>{const ws=new WebSocket('ws://127.0.0.1:'+port);ws.onopen=()=>ws.send(JSON.stringify({event,uuid}));};</script>'''
SCRIPT = r'''
const phase=__PHASE__, bridge=window.deckforge, counter='com.elgato.counter',hello='com.elgato.hello-world';
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<180;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const read=()=>bridge.request('snapshot'),plugin=(s,id)=>s.plugins.find(plugin=>plugin.id===id);
const initial=await until(async()=>{const s=await read();if(!s.simulate)throw Error('Simulation required');return plugin(s,hello)?.connected&&(phase==='detach'||plugin(s,counter)?.connected)&&s},'sample plugin registration');
const otherPid=plugin(initial,hello).pid;
const command=payload=>bridge.request('command',{...payload,device:initial.device});
const button=name=>[...document.querySelectorAll('button')].find(button=>button.textContent.trim()===name);
const open=async()=>{[...document.querySelectorAll('.header-tools button')].find(button=>button.textContent.trim().startsWith('Plugins')).click();await until(()=>document.querySelector('dialog[open] .plugin-row'),'Plugins panel')};
const row=id=>[...document.querySelectorAll('.plugin-row')].find(row=>row.textContent.includes(id));
const rowButton=(id,name)=>row(id)&&[...row(id).querySelectorAll('button')].find(button=>button.textContent.trim()===name);
if(phase==='detach'){
 if(plugin(initial,counter))throw Error('Normal startup restored removed managed plugin');
 const old=initial.slots.find(slot=>slot.uuid==='com.elgato.counter.action');
 if(!old||old.available!==false||old.settings.incrementBy!==2||!old.settings.foreign?.keep)throw Error('Restart lost missing plugin assignment/settings');
 if(plugin(initial,hello).removal_mode!=='detach')throw Error('External plugin not marked detach');
 await open();rowButton(hello,'Remove').click();
 await until(()=>document.querySelector('.plugin-removal')?.textContent.includes('external plugin folder will be left untouched'),'external confirmation');
 button('Remove plugin').click();
 await until(async()=>!(await read()).plugins.length&&!document.querySelector('.plugin-removal'),'external removal');
 const after=await read(),helloSlot=after.slots.find(slot=>slot.uuid==='com.elgato.hello-world.newstring');
 if(!helloSlot||helloSlot.available!==false||!helloSlot.settings.foreign?.keep)throw Error('Detach lost the external plugin assignment/settings');
 return{phase,external_detached:true,normal_startup_keeps_archive_removed:true,counter_context:old.context,counter_settings:old.settings,hello_context:helloSlot.context,hello_settings:helloSlot.settings,plugins_left:0,usb_access:false};
}
if(plugin(initial,counter).removal_mode!=='archive')throw Error('Managed plugin not marked archive');
await command({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.counter.action'});
await command({command:'assign',controller:'Keypad',index:1,uuid:'com.elgato.hello-world.newstring'});
const assigned=await until(async()=>{const s=await read();return s.slots.some(slot=>slot.uuid==='com.elgato.counter.action')&&s.slots.some(slot=>slot.uuid==='com.elgato.hello-world.newstring')&&s},'both assigned slots'),first=assigned.slots.find(slot=>slot.uuid==='com.elgato.counter.action'),second=assigned.slots.find(slot=>slot.uuid==='com.elgato.hello-world.newstring');
await command({command:'settings',context:first.context,settings:{count:10,incrementBy:2,foreign:{keep:true}}});
await command({command:'settings',context:second.context,settings:{language:'en',titleKey:'__deckforge_removal_probe__',foreign:{keep:true}}});
document.querySelector('.deck-key').click();
const field=()=>[...document.querySelectorAll('label.field')].find(label=>label.querySelector('span')?.textContent==='Increment by')?.querySelector('input');
await until(()=>field()?.value==='2','Counter settings');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field(),'4');field().dispatchEvent(new Event('input',{bubbles:true}));field().dispatchEvent(new Event('change',{bubbles:true}));
await until(async()=>(await bridge.request('ui_state_get'))?.drafts?.[first.context]?.settings.incrementBy===4,'unsaved Counter draft');
button('Plugin settings ↗').click();await until(async()=>(await read()).inspectors.some(pi=>pi.context===first.context&&pi.connected),'owned Counter PI');
await open();rowButton(counter,'Remove').click();
await until(()=>document.querySelector('.plugin-removal')?.textContent.includes('private removed-plugins backup'),'managed confirmation');
if(!document.querySelector('.plugin-removal').textContent.includes(plugin(initial,counter).path))throw Error('Confirmation lacks captured exact plugin folder');
if((await read()).plugins.length!==2)throw Error('Opening confirmation removed plugin before confirmation');
button('Remove plugin').click();
const removed=await until(async()=>{const s=await read();return !plugin(s,counter)&&!row(counter)&&s},'managed removal');
if(plugin(removed,hello)?.pid!==otherPid||!plugin(removed,hello)?.connected)throw Error('Removing first plugin restarted survivor');
if(removed.inspectors.some(pi=>pi.context===first.context))throw Error('Removed plugin left its inspector open');
const retained=removed.slots.find(slot=>slot.context===first.context);
if(!retained||retained.available!==false||retained.settings.incrementBy!==2||!retained.settings.foreign?.keep)throw Error('Removal lost assignment/settings');
document.querySelector('[aria-label="Close Plugins"]').click();
await until(()=>button('Open Plugins'),'clear reinstall guidance');
if(field()?.value!=='4'||button('Save changes').disabled)throw Error('Removal erased or locked the local draft');
if(document.querySelector('[aria-label="Assign Counter"]')||document.querySelector('[aria-label="Assign Count"]'))throw Error('Removed plugin remained assignable in catalog');
// Only the inert official HelloWorld sample receives simulated input. This proves
// the surviving process still receives events after vector ownership changes.
await command({command:'settings',context:second.context,settings:{language:'en',titleKey:'__deckforge_removal_probe__',foreign:{keep:true}}});
await until(async()=>(await read()).slots.find(slot=>slot.context===second.context)?.settings.titleKey==='__deckforge_removal_probe__','survivor probe settings');
await command({command:'input',event:'keyDown',index:1});await command({command:'input',event:'keyUp',index:1});
await until(async()=>{const s=await read(),slot=s.slots.find(slot=>slot.context===second.context);return slot.settings.titleKey&&slot.settings.titleKey!=='__deckforge_removal_probe__'},'surviving SDK event route');
await open();rowButton(hello,'Remove').click();
await until(()=>document.querySelector('.plugin-removal')?.textContent.includes('external plugin folder will be left untouched'),'external confirmation capture');
return{phase,managed_archived:true,counter_context:first.context,counter_settings:retained.settings,draft_preserved:true,inspector_closed:true,surviving_plugin_pid:otherPid,surviving_sdk_received_input:true,reinstall_guidance:true,external_confirmation_visible:true,usb_access:false};
'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary",type=Path,default=PROJECT / "target/release/deckforge")
    parser.add_argument("--samples",type=Path,default=PROJECT.parent / "evidence/official-samples")
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    binary,samples,output=args.binary.resolve(strict=True),args.samples.resolve(strict=True),args.output.resolve()
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
    assert subprocess.check_output(["git","-C",str(samples),"rev-parse","HEAD"],text=True).strip()==SAMPLE_COMMIT
    output.mkdir(parents=True,exist_ok=True)
    digest=lambda path:hashlib.sha256(path.read_bytes()).hexdigest()
    report={"ok":False,"binary_sha256":digest(binary),"sample_commit":SAMPLE_COMMIT,"runs":[]}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-remove-") as temporary:
        root=Path(temporary);state=root / "state";managed=state / "plugins" / f"{COUNTER}.sdPlugin";external=root / "external" / f"{HELLO}.sdPlugin"
        shutil.copytree(samples / "counter/com.elgato.counter.sdPlugin",managed)
        shutil.copytree(samples / "hello-world/com.elgato.hello-world.sdPlugin",external)
        (managed / "ui/removal-probe.html").write_text(PI)
        manifest=json.loads((managed / "manifest.json").read_text())
        for action in manifest["Actions"]:action["PropertyInspectorPath"]="ui/removal-probe.html"
        (managed / "manifest.json").write_text(json.dumps(manifest))
        original_counter=digest(managed / "bin/plugin.js");original_external=digest(external / "bin/plugin.js")
        sentinels=[]
        for identity in (COUNTER,HELLO):
            private=state / "plugin-data" / hashlib.sha256(identity.encode()).hexdigest();private.mkdir(parents=True,exist_ok=True);private.chmod(0o700)
            sentinel=private / "retained-fixture.txt";sentinel.write_text(identity);sentinels.append(sentinel)
        for phase in ("archive","detach"):
            script,proof=root / f"{phase}.js",root / f"{phase}.json";script.write_text(SCRIPT.replace("__PHASE__",json.dumps(phase)))
            env=environment(root);env.update(LC_ALL="C.UTF-8",DECKFORGE_CAPTURE=str(output / f"{phase}.png"),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
            command=[str(binary),"desktop","--simulate","--state",str(state)]
            if phase=="archive":command += ["--plugin",str(managed),"--plugin",str(external)]
            log_path=output / f"{phase}.log"
            with log_path.open("w") as log:
                process=subprocess.Popen(command,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    assert process.wait(timeout=45)==0,log_path.read_text();value=result(proof);assert value,log_path.read_text();report["runs"].append(value["result"])
                    assert not managed.exists()
                    backups=list((state / "removed-plugins").rglob("bin/plugin.js"));assert len(backups)==1 and digest(backups[0])==original_counter
                    assert stat.S_IMODE((state / "removed-plugins").stat().st_mode)==0o700
                    assert external.is_dir() and digest(external / "bin/plugin.js")==original_external
                    assert all(sentinel.is_file() for sentinel in sentinels)
                    saved=json.loads((state / "session.json").read_text());matches=list(saved_slots(saved,report["runs"][0]["counter_context"]));assert matches and all(slot["settings"]["incrementBy"]==2 and slot["settings"]["foreign"]["keep"] for slot in matches)
                    if phase=="detach":
                        matches=list(saved_slots(saved,value["result"]["hello_context"]));assert matches and all(slot["settings"]["foreign"]["keep"] for slot in matches)
                except Exception as error:report["error"]=str(error);raise
                finally:cleanup(process);(output / "removal.json").write_text(json.dumps(report,indent=2)+"\n")
        report.update(ok=True,managed_backup_verified=True,backup_private=True,external_folder_untouched=True,private_plugin_data_retained=True,profile_settings_persisted=True)
        (output / "removal.json").write_text(json.dumps(report,indent=2)+"\n")
    print("PASS: native confirmed managed archive/external detach, retained profiles/private data/drafts, stopped PI and live surviving SDK route; owned temporary state only.")


if __name__=="__main__":main()
