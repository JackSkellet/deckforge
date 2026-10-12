#!/usr/bin/env python3
"""Exercise the real embedded device picker and two concurrent Counter contexts.

The headless seed and native window use a temporary simulated session. JavaScript
runs through the application's own WebKit instrumentation, not OS or browser
control. No USB, audio action, or real user session is accessed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

from native_editor_interop import cleanup, environment, result
from sample_interop import COUNTER, COUNTER_LOCK_SHA256, SAMPLE_COMMIT
from system_interop import Host

COUNTER_PLUGIN = "com.elgato.counter"
SECOND_DEVICE = "native-multi-mini-B"
COPY_DEVICE = "secondary-streamdeck-plus"

SCRIPT = r'''
const expected=__EXPECTED__;
const bridge=window.deckforge;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,description)=>{
 for(let i=0;i<200;i++){const value=await fn();if(value)return value;await pause(40)}
 const s=await bridge.request('snapshot');
 throw Error('Timed out: '+description+'; selected='+s.device+'; devices='+JSON.stringify(s.devices)+'; slots='+JSON.stringify(s.slots.map(v=>({context:v.context,controller:v.controller,index:v.index,uuid:v.uuid,settings:v.settings,title:v.title}))));
};
let pid=null;
const read=async()=>{
 const state=await bridge.request('snapshot');
 if(state.simulate!==true)throw Error('Simulation is required');
 const plugins=state.plugins.filter(plugin=>plugin.id==='com.elgato.counter');
 if(plugins.length!==1)throw Error('Expected one Counter plugin record, got '+plugins.length);
 if(pid!==null&&(!plugins[0].connected||plugins[0].pid!==pid))throw Error('Counter restarted/disconnected during editor selection');
 if(!Array.isArray(state.devices)||!expected.devices.every(id=>state.devices.some(device=>device.id===id)))throw Error('Associated device inventory was not restored');
 return state;
};
const counter=state=>state.slots.find(slot=>slot.uuid==='com.elgato.counter.action'&&slot.controller==='Keypad'&&slot.index===0);
const initial=await until(async()=>{
 const state=await read(),plugin=state.plugins.find(plugin=>plugin.id==='com.elgato.counter');
 return plugin.connected&&Number.isInteger(plugin.pid)&&plugin.pid>0&&state;
},'one official Counter process to register after restart');
pid=initial.plugins.find(plugin=>plugin.id==='com.elgato.counter').pid;
if(initial.device!==expected.a)throw Error('Selected device A was not persisted');
const inventory=initial.devices.map(({id,model,name,connected,online,firmware,selected})=>({id,model,name,connected,online,firmware,selected}));
const picker=()=>document.querySelector('.device-picker select');
await until(()=>picker()&&expected.devices.every(id=>[...picker().options].some(option=>option.value===id)),'both devices in the real picker');
const pick=async(id,keys,dials)=>{
 const element=picker();
 Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(element,id);
 element.dispatchEvent(new Event('input',{bubbles:true}));
 element.dispatchEvent(new Event('change',{bubbles:true}));
 return until(async()=>{
  const state=await read();
  return state.device===id&&picker().value===id&&!picker().disabled&&document.querySelectorAll('.deck-key').length===keys&&document.querySelectorAll('.deck-dial').length===dials&&state;
 },'picker selection '+id+' with '+keys+' keys and '+dials+' dials');
};
const field=name=>[...document.querySelectorAll('label.field')].find(label=>label.querySelector('span')?.textContent===name)?.querySelector('input,textarea');
const set=(element,value)=>{
 const proto=element.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;
 Object.getOwnPropertyDescriptor(proto,'value').set.call(element,String(value));
 element.dispatchEvent(new Event('input',{bubbles:true}));element.dispatchEvent(new Event('change',{bubbles:true}));
};
let b=await pick(expected.b,6,0);
b=await until(async()=>{const s=await read(),slot=counter(s);return slot?.settings.count===expected.b_count&&slot.settings.incrementBy===expected.b_increment&&slot.title===String(expected.b_count)&&s},'restored B Counter');
const bSlot=counter(b),bSettings=structuredClone(bSlot.settings);
let a=await pick(expected.a,8,4);
a=await until(async()=>{const s=await read(),slot=counter(s);return slot?.settings.count===expected.a_count&&slot.settings.incrementBy===expected.a_increment&&slot.title===String(expected.a_count)&&s},'restored A Counter');
const aSlot=counter(a);
if(aSlot.context===bSlot.context)throw Error('Two devices share an action context');
document.querySelector('.deck-key').click();
const increment=await until(()=>field('incrementBy')?.value===String(expected.a_increment)&&field('incrementBy'),'A settings editor');
set(increment,expected.draft_increment);
await until(async()=>{const ui=await bridge.request('ui_state_get'),draft=ui?.drafts?.[aSlot.context];return draft?.dirty&&draft.settings.incrementBy===expected.draft_increment},'retained A increment draft');
if(counter(await read()).settings.incrementBy!==expected.a_increment)throw Error('Draft applied before Save');
await pick(expected.b,6,0);
const retainedOnB=(await bridge.request('ui_state_get'))?.drafts?.[aSlot.context];
if(!retainedOnB?.dirty||retainedOnB.settings.incrementBy!==expected.draft_increment)throw Error('Switching to B erased A draft');
const press=async device=>{
 await bridge.request('command',{command:'input',device,event:'keyDown',index:0});
 await bridge.request('command',{command:'input',device,event:'keyUp',index:0});
};
await press(expected.a);
// Observe B for several publisher cycles: a misrouted asynchronous SDK update
// must not be hidden by immediately leaving this device again.
for(let i=0;i<8;i++){
 const state=await read();
 if(state.device!==expected.b||JSON.stringify(counter(state).settings)!==JSON.stringify(bSettings))throw Error('Background A input changed selected B');
 await pause(50);
}
await pick(expected.a,8,4);
const firstCount=expected.a_count+expected.a_increment;
a=await until(async()=>{const s=await read(),slot=counter(s);return slot?.settings.count===firstCount&&slot.title===String(firstCount)&&s},'background A Counter update');
await until(()=>field('incrementBy')?.value===String(expected.draft_increment),'A draft restored in the native editor');
const draftBeforeSave=(await bridge.request('ui_state_get')).drafts[aSlot.context];
if(draftBeforeSave.settings.count!==expected.a_count)throw Error('The fixture did not retain a genuinely stale count baseline');
const save=()=>[...document.querySelectorAll('button')].find(button=>button.textContent.trim()==='Save changes');
await until(()=>save()&&!save().disabled,'Save enabled for the A draft');save().click();
a=await until(async()=>{const s=await read(),slot=counter(s);return slot?.settings.incrementBy===expected.draft_increment&&s},'A-only increment save');
if(counter(a).settings.count!==firstCount)throw Error('Increment-only Save overwrote the Counter update that happened while B was selected');
await until(async()=>!(await bridge.request('ui_state_get'))?.drafts?.[aSlot.context]?.dirty,'A draft acknowledged');
await until(()=>field('count')?.value===String(firstCount),'native form receives the merged live count');
await pick(expected.b,6,0);
if(JSON.stringify(counter(await read()).settings)!==JSON.stringify(bSettings))throw Error('Saving A changed B settings');
await press(expected.a);
for(let i=0;i<8;i++){
 const state=await read();
 if(state.device!==expected.b||JSON.stringify(counter(state).settings)!==JSON.stringify(bSettings))throw Error('Second background A input changed selected B');
 await pause(50);
}
await pick(expected.a,8,4);
const finalCount=firstCount+expected.draft_increment;
await until(async()=>{const s=await read(),slot=counter(s);return slot?.settings.count===finalCount&&slot.title===String(finalCount)&&s},'saved A increment used by the same Counter process');
await until(()=>field('count')?.value===String(finalCount),'merged live count in the actual form');
// A real edit to count must still replace that field; the patch is not a special
// rule that simply ignores Counter.count.
set(field('count'),expected.explicit_count);
await until(async()=>{const draft=(await bridge.request('ui_state_get'))?.drafts?.[aSlot.context];return draft?.dirty&&draft.settings.count===expected.explicit_count},'explicit count draft');
await until(()=>save()&&!save().disabled,'explicit count Save enabled');save().click();
await until(async()=>counter(await read()).settings.count===expected.explicit_count,'explicit count saved to A');
await until(async()=>!(await bridge.request('ui_state_get'))?.drafts?.[aSlot.context]?.dirty,'explicit count draft acknowledged');
await pick(expected.b,6,0);
if(JSON.stringify(counter(await read()).settings)!==JSON.stringify(bSettings))throw Error('Explicit A count edit changed B');
await press(expected.a);
await pick(expected.a,8,4);
const final=await until(async()=>{const s=await read(),slot=counter(s);return slot?.settings.count===expected.explicit_count+expected.draft_increment&&slot.title===String(slot.settings.count)&&s},'explicit count remains editable');
await until(()=>field('count')?.value===String(counter(final).settings.count),'final live count in the actual form');
const image=document.querySelector('.deck-key img');await until(()=>image?.complete&&image.naturalWidth>0,'rendered final Counter image');
// Exercise the actual profile-options dialog after all ownership/draft checks.
const sourceProfile=final.library.active_profile;
const sourceProfiles=final.library.profiles.map(profile=>profile.id);
const options=[...document.querySelectorAll('summary')].find(element=>element.textContent.trim()==='Profile options');
if(!options)throw Error('Profile options are missing');
if(!options.parentElement.open)options.click();
const copyButton=[...document.querySelectorAll('button')].find(button=>button.textContent.trim()==='Copy to device…');
if(!copyButton||copyButton.disabled)throw Error('Copy to device is unavailable');
copyButton.click();
const dialog=await until(()=>[...document.querySelectorAll('dialog[open]')].find(element=>element.querySelector('h2')?.textContent==='Copy profile to device'),'profile copy dialog');
const submit=()=>[...dialog.querySelectorAll('button')].find(button=>button.textContent.trim()==='Copy profile');
const target=dialog.querySelector('select');
let profileCopy;
if(!expected.copy_target){
 if(!dialog.textContent.includes('No other compatible devices are associated with this profile.'))throw Error('Incompatible device explanation is missing');
 if(!submit()?.disabled||[...target.options].some(option=>option.value))throw Error('Copy offered an incompatible target');
 profileCopy={mode:'no-eligible',explanation_visible:true,submit_disabled:true,incompatible_target_excluded:true};
 dialog.querySelector('[aria-label="Close Copy profile to device"]').click();
 await until(()=>!dialog.open,'copy dialog closed');
}else{
 const targets=[...target.options].map(option=>option.value);
 if(JSON.stringify(targets)!==JSON.stringify([expected.copy_target]))throw Error('Copy target list did not follow backend compatibility metadata');
 for(const id of [expected.a,expected.copy_target]){
  const option=[...picker().options].find(option=>option.value===id);
  if(!option?.textContent.includes(id)||!option.title.includes(id))throw Error('Same-model picker label does not include full ID '+id);
 }
 const destinationOption=[...target.options].find(option=>option.value===expected.copy_target);
 if(!destinationOption.textContent.includes(expected.copy_target)||!destinationOption.title.includes(expected.copy_target))throw Error('Copy destination lacks its full distinguishing ID');
 const copyName='Native copied profile';
 set(field('Copy name'),copyName);
 await until(async()=>(await bridge.request('ui_state_get'))?.copyProfile?.name===copyName,'copy name retained');
 if(submit()?.disabled)throw Error('Compatible copy submit is disabled');
 submit().click();
 await until(()=>!dialog.open,'successful native copy submission');
 const stillSource=await read();
 if(stillSource.device!==expected.a||stillSource.library.active_profile!==sourceProfile)throw Error('Copy moved the editor or replaced its source profile');
 const copied=await pick(expected.copy_target,8,4);
 const copiedProfile=copied.library.profiles.find(profile=>profile.id===copied.library.active_profile);
 const copiedSlot=counter(copied);
 if(copiedProfile?.name!==copyName||copiedProfile.id===sourceProfile)throw Error('Destination copy was not independently activated');
 if(!copiedSlot||[aSlot.context,bSlot.context].includes(copiedSlot.context)||JSON.stringify(copiedSlot.settings)!==JSON.stringify(counter(final).settings))throw Error('Copied action identity/settings were not preserved independently');
 profileCopy={mode:'compatible',device:expected.copy_target,profile:copiedProfile.id,context:copiedSlot.context,settings:copiedSlot.settings,source_profile:sourceProfile,distinct_context:true,full_device_ids_visible:true,source_editor_preserved:true};
 await pick(expected.a,8,4);
}
if(options.parentElement.open)options.click();
const afterCopy=await read();
if(afterCopy.device!==expected.a||afterCopy.library.active_profile!==sourceProfile||JSON.stringify(afterCopy.library.profiles.map(profile=>profile.id))!==JSON.stringify(sourceProfiles)||JSON.stringify(counter(afterCopy).settings)!==JSON.stringify(counter(final).settings))throw Error('Copy dialog changed the original device profile/settings');
return {
 profile_copy_ui:profileCopy,
 device_inventory:inventory,
 counter_process:{id:'com.elgato.counter',pid,shared_across_devices:true,unchanged_across_picker_switches:true},
 a:{device:expected.a,context:aSlot.context,before:{count:expected.a_count,incrementBy:expected.a_increment},background_count:firstCount,increment_after_save:expected.draft_increment,count_after_increment_probe:finalCount,explicit_count_edit:expected.explicit_count,final:counter(final).settings},
 b:{device:expected.b,context:bSlot.context,settings:bSettings,unchanged:true,keys:6,dials:0},
 draft_preserved_across_switch:true,explicit_input_owner_preserved:true,live_count_preserved_by_settings_patch:true,explicit_count_edit_supported:true,
 selected:final.device,usb_access:false,audio_actions:false,viewport:[innerWidth,innerHeight]
};
'''


class SeedHost(Host):
    def __init__(self, binary, counter, state, log, env):
        self.log = log
        self.output = log.open("w")
        self.process = subprocess.Popen(
            [str(binary), "serve", "--simulate", "--model", "plus", "--port", "0",
             "--state", str(state), "--plugin", str(counter)],
            env=env, stdout=self.output, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.base = self.token = None


def counter_slot(state):
    return next((slot for slot in state["slots"]
                 if slot["uuid"] == COUNTER and slot["controller"] == "Keypad"
                 and slot["index"] == 0), None)


def saved_slots(value, context):
    if isinstance(value, dict):
        if value.get("context") == context and isinstance(value.get("settings"), dict):
            yield value
        for child in value.values():
            yield from saved_slots(child, context)
    elif isinstance(value, list):
        for child in value:
            yield from saved_slots(child, context)


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/release/deckforge")
    parser.add_argument("--samples", type=Path, default=project.parent / "evidence/official-samples")
    parser.add_argument("--capture", type=Path, help="Keep the app-owned native WebKit PNG")
    parser.add_argument("--report", type=Path, help="Save checks, hashes and seed/native log sidecars")
    parser.add_argument("--compatible-copy", action="store_true", help="Also seed a third Plus and submit the native profile-copy dialog")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    samples = args.samples.resolve(strict=True)
    counter = samples / "counter/com.elgato.counter.sdPlugin"
    assert (counter / "bin/plugin.js").is_file(), "Build the official Counter fixture first"
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    if (samples / ".git").exists():
        assert subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip() == SAMPLE_COMMIT
    report_path = args.report.resolve() if args.report else None
    if report_path:
        report_path.parent.mkdir(parents=True, exist_ok=True)
    report = {
        "ok": False,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "counter_bundle_sha256": hashlib.sha256((counter / "bin/plugin.js").read_bytes()).hexdigest(),
        "counter_lock_sha256": COUNTER_LOCK_SHA256,
        "sample_commit": SAMPLE_COMMIT,
        "boundary": "App-owned WebKit instrumentation; isolated simulated session; no USB, audio actions or OS/browser automation",
        "phase": "seed",
    }
    with tempfile.TemporaryDirectory(prefix="deckforge-native-multi-") as temporary:
        root = Path(temporary)
        state = root / "state"
        seed_log, native_log = root / "seed.log", root / "native.log"
        try:
            host = SeedHost(binary, counter, state, seed_log, environment(root))
            try:
                host.ready()
                initial = host.wait(lambda s: len(s["plugins"]) == 1 and s["plugins"][0]["id"] == COUNTER_PLUGIN and s["plugins"][0]["connected"], "Counter seed registration")
                first_device = initial["device"]
                host.post({"command": "device_add", "device": SECOND_DEVICE, "model": "mini"})
                expected = {"a": first_device, "b": SECOND_DEVICE,
                            "devices": [first_device, SECOND_DEVICE],
                            "a_count": 10, "a_increment": 2,
                            "b_count": 20, "b_increment": 3,
                            "draft_increment": 7, "explicit_count": 100}
                if args.compatible_copy:
                    host.post({"command": "device_add", "device": COPY_DEVICE, "model": "plus"})
                    expected["devices"].append(COPY_DEVICE)
                    expected["copy_target"] = COPY_DEVICE
                seeded = []
                for device, count, increment in [(first_device, 10, 2), (SECOND_DEVICE, 20, 3)]:
                    host.post({"command": "device_select", "device": device})
                    host.post({"command": "assign", "device": device, "controller": "Keypad", "index": 0, "uuid": COUNTER})
                    slot = counter_slot(host.wait(lambda s: counter_slot(s) is not None, "seeded Counter slot"))
                    host.post({"command": "settings", "device": device, "context": slot["context"], "settings": {"count": count, "incrementBy": increment}})
                    configured = host.wait(lambda s: counter_slot(s)["settings"] == {"count": count, "incrementBy": increment}, "saved seed settings")
                    seeded.append({"device": device, "context": slot["context"], "settings": counter_slot(configured)["settings"]})
                host.post({"command": "device_select", "device": first_device})
                report["seed"] = {"selected": first_device, "slots": seeded, "devices": host.state().get("devices", [])}
            finally:
                host.stop()
            assert host.process.returncode == 0, seed_log.read_text()
            persisted = json.loads((state / "session.json").read_text())
            assert {record["device"] for record in persisted["devices"]} == set(expected["devices"]), "Both associated devices must persist before native startup"
            report["phase"] = "native"
            script, proof = root / "fixture.js", root / "proof.json"
            script.write_text(SCRIPT.replace("__EXPECTED__", json.dumps(expected)))
            capture = args.capture.resolve() if args.capture else root / "native.png"
            capture.parent.mkdir(parents=True, exist_ok=True)
            env = environment(root)
            env.update(DECKFORGE_CAPTURE=str(capture), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
            with native_log.open("w") as log:
                process = subprocess.Popen(
                    [str(binary), "desktop", "--simulate", "--state", str(state), "--port", "0"],
                    env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
                )
                try:
                    assert process.wait(timeout=50) == 0, native_log.read_text()
                    native = result(proof)
                    assert native, native_log.read_text()
                    report["native"] = native["result"]
                    png = capture.read_bytes()
                    assert png[:8] == b"\x89PNG\r\n\x1a\n"
                    width, height = struct.unpack(">II", png[16:24])
                    assert width >= 760 and height >= 620
                    report["capture"] = {"sha256": hashlib.sha256(png).hexdigest(), "width": width, "height": height}
                finally:
                    cleanup(process)
            report["phase"] = "persisted-result"
            persisted = json.loads((state / "session.json").read_text())
            for name in ("a", "b"):
                item = report["native"][name]
                matches = list(saved_slots(persisted, item["context"]))
                wanted = item["final"] if name == "a" else item["settings"]
                assert matches and all(slot["settings"] == wanted for slot in matches), f"Saved {name.upper()} settings differ from native verification"
            copied = report["native"]["profile_copy_ui"]
            if copied["mode"] == "compatible":
                matches = list(saved_slots(persisted, copied["context"]))
                assert matches and all(slot["settings"] == copied["settings"] for slot in matches), "Copied destination settings did not persist"
            report["persisted_result"] = True
            report["phase"] = "complete"
            report["ok"] = True
        except Exception as error:
            report["error"] = str(error)
            for path in (seed_log, native_log):
                if path.exists():
                    print(f"{path.name}:\n{path.read_text()[-5000:]}")
            raise
        finally:
            if report_path:
                for source, suffix in ((seed_log, ".seed.log"), (native_log, ".native.log")):
                    if source.exists():
                        report_path.with_suffix(suffix).write_text(source.read_text())
                report_path.write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native picker, Mini geometry, retained A draft, one Counter process, background A input and count-preserving A-only Save and native profile-copy dialog; no USB/audio.")


if __name__ == "__main__":
    main()
