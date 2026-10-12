#!/usr/bin/env python3
"""Native plugin enable/disable controls with two official sample backends.

Uses temporary simulated state and a local inert Counter PI. No USB, user state,
external network, action input, or installed application process is touched.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / "tests"))
from native_editor_interop import cleanup, environment, result
from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT

PI = '''<!doctype html><html><body>Owned lifecycle inspector fixture<script>
window.connectElgatoStreamDeckSocket=(port,uuid,event)=>{const ws=new WebSocket('ws://127.0.0.1:'+port);ws.onopen=()=>ws.send(JSON.stringify({event,uuid}));};
</script></body></html>'''
SCRIPT = r'''
const bridge=window.deckforge, counterId='com.elgato.counter', otherId='com.elgato.hello-world';
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<180;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const read=()=>bridge.request('snapshot'),plugin=(s,id)=>s.plugins.find(p=>p.id===id);
const initial=await until(async()=>{const s=await read();if(!s.simulate)throw Error('Simulation required');return plugin(s,counterId)?.connected&&plugin(s,otherId)?.connected&&s},'both real sample backends');
const originalCounterPid=plugin(initial,counterId).pid,otherPid=plugin(initial,otherId).pid;
const command=payload=>bridge.request('command',{...payload,device:initial.device});
await command({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.counter.action'});
const slot=await until(async()=>{const s=await read();return s.slots.find(slot=>slot.controller==='Keypad'&&slot.index===0&&slot.uuid==='com.elgato.counter.action')},'Counter assignment');
await command({command:'settings',context:slot.context,settings:{count:10,incrementBy:2,foreign:{keep:true}}});
document.querySelector('.deck-key').click();
const field=label=>[...document.querySelectorAll('label.field')].find(item=>item.querySelector('span')?.textContent===label)?.querySelector('input');
await until(()=>field('Increment by')?.value==='2','Counter native editor');
const increment=field('Increment by');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(increment,'3');increment.dispatchEvent(new Event('input',{bubbles:true}));increment.dispatchEvent(new Event('change',{bubbles:true}));
await until(async()=>(await bridge.request('ui_state_get'))?.drafts?.[slot.context]?.settings.incrementBy===3,'unsaved native draft');
const buttons=()=>[...document.querySelectorAll('button')];
const button=name=>buttons().find(b=>b.textContent.trim()===name);
button('Plugin settings ↗').click();
await until(async()=>(await read()).inspectors.some(pi=>pi.context===slot.context&&pi.connected),'owned native Counter inspector');
const open=async()=>{buttons().find(b=>b.textContent.trim().startsWith('Plugins')).click();await until(()=>document.querySelector('dialog[open] .plugin-row'),'Plugins dialog')};
const row=id=>[...document.querySelectorAll('.plugin-row')].find(node=>node.textContent.includes(id));
const rowButton=(id,label)=>row(id)&&[...row(id).querySelectorAll('button')].find(button=>button.textContent.trim()===label);
await open();rowButton(counterId,'Disable').click();
const disabled=await until(async()=>{const s=await read();return plugin(s,counterId).enabled===false&&!plugin(s,counterId).connected&&!plugin(s,counterId).pid&&s},'Counter stopped by native Disable');
if(!plugin(disabled,otherId).connected||plugin(disabled,otherId).pid!==otherPid)throw Error('Disabling Counter changed the other plugin process');
if(disabled.inspectors.some(pi=>pi.context===slot.context))throw Error('Disabled plugin kept its inspector');
await until(()=>rowButton(counterId,'Enable'),'Enable button after disable');
if(!rowButton(counterId,'Restart').disabled||!row(counterId).textContent.includes('Disabled'))throw Error('Disabled row status/Restart incorrect');
document.querySelector('[aria-label="Close Plugins"]').click();
await until(()=>button('Enable plugin in Plugins'),'selected disabled action explanation');
if(field('Increment by')?.value!=='3'||button('Save changes').disabled)throw Error('Disable erased or locked the action draft');
const assign=document.querySelector('[aria-label="Assign '+slot.name+'"]')||document.querySelector('[aria-label="Assign Counter"]')||document.querySelector('[aria-label="Assign Count"]');
if(!assign?.disabled||assign.closest('.library-action').draggable||!assign.closest('.library-action').textContent.includes('Disabled'))throw Error('Disabled catalog still allows assignment/drag');
if(button('Plugin settings ↗')||!button('Test press').disabled)throw Error('Disabled action offers PI/execution');
const savedSlot=disabled.slots.find(value=>value.context===slot.context);
if(!savedSlot.plugin_disabled||savedSlot.available!==false||savedSlot.settings.incrementBy!==2||savedSlot.settings.foreign?.keep!==true)throw Error('Disabled action/settings were not retained');
button('Enable plugin in Plugins').click();await until(()=>rowButton(counterId,'Enable'),'Plugins reopened');rowButton(counterId,'Enable').click();
const enabled=await until(async()=>{const s=await read();const p=plugin(s,counterId);return p.enabled!==false&&p.connected&&p.pid&&p.pid!==originalCounterPid&&s},'Counter restarted by native Enable');
if(plugin(enabled,otherId).pid!==otherPid||!plugin(enabled,otherId).connected)throw Error('Enable restarted the other plugin');
await until(()=>rowButton(counterId,'Disable')&&!rowButton(counterId,'Restart').disabled,'enabled row');
document.querySelector('[aria-label="Close Plugins"]').click();
await until(()=>button('Plugin settings ↗')&&!button('Test press').disabled,'restored action controls');
if(field('Increment by')?.value!=='3')throw Error('Enable erased draft');
const enabledDraft=(await bridge.request('ui_state_get')).drafts[slot.context];
if(!enabledDraft.dirty||enabledDraft.settings.incrementBy!==3)throw Error('Retained native draft lost dirty intent');
// Leave the isolated screenshot with both lifecycle states visible.
await open();rowButton(counterId,'Disable').click();
await until(async()=>{const s=await read();return plugin(s,counterId).enabled===false&&!plugin(s,counterId).pid&&rowButton(counterId,'Enable')},'final disabled presentation');
return{counter_original_pid:originalCounterPid,counter_enabled_pid:plugin(enabled,counterId).pid,other_pid:otherPid,other_plugin_unchanged:true,inspector_closed:true,slot_context:slot.context,saved_settings:savedSlot.settings,draft:enabledDraft.settings,draft_preserved:true,catalog_assignment_and_drag_blocked:true,selected_pi_and_test_blocked:true,actual_disable_enable_buttons:true,final_counter_disabled:true,usb_access:false,action_input:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=PROJECT / "target/release/deckforge")
    parser.add_argument("--samples", type=Path, default=PROJECT.parent / "evidence/official-samples")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, samples, output = args.binary.resolve(strict=True), args.samples.resolve(strict=True), args.output.resolve()
    counter = samples / "counter/com.elgato.counter.sdPlugin"
    other = samples / "hello-world/com.elgato.hello-world.sdPlugin"
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    assert subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip() == SAMPLE_COMMIT
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "sample_commit": SAMPLE_COMMIT, "counter_backend_sha256": hashlib.sha256((counter / "bin/plugin.js").read_bytes()).hexdigest(), "boundary": "Real official sample backends, owned local inert PI, temporary simulated state; no external network/USB/action input"}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-plugin-lifecycle-") as temporary:
        root = Path(temporary)
        copied = root / "counter.sdPlugin"
        shutil.copytree(counter, copied)
        (copied / "ui/lifecycle-probe.html").write_text(PI)
        manifest = json.loads((copied / "manifest.json").read_text())
        for action in manifest["Actions"]:
            action["PropertyInspectorPath"] = "ui/lifecycle-probe.html"
        (copied / "manifest.json").write_text(json.dumps(manifest))
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT)
        env = environment(root)
        env.update(LC_ALL="C.UTF-8", DECKFORGE_CAPTURE=str(output / "plugins.png"), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = output / "plugins.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary), "desktop", "--simulate", "--state", str(root / "state"), "--plugin", str(copied), "--plugin", str(other)], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=45) == 0, log_path.read_text()
                value = result(proof)
                assert value, log_path.read_text()
                report.update(ok=True,native=value["result"],capture_sha256=hashlib.sha256((output / "plugins.png").read_bytes()).hexdigest())
            except Exception as error:
                report["error"] = str(error)
                raise
            finally:
                cleanup(process)
                (output / "plugins.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native plugin Disable/Enable, independent process/PI lifecycle, retained settings/drafts and guarded disabled actions; no USB/user state.")


if __name__ == "__main__":
    main()
