#!/usr/bin/env python3
"""Native plugin-language preference using two isolated app lifetimes; no USB."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / "tests"))
from native_editor_interop import cleanup, environment, result
from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT

SCRIPT = r'''
const phase=__PHASE__, expectedName=__NAME__, bridge=window.deckforge;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<150;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name+'; crypto.randomUUID='+typeof crypto.randomUUID+'; UI='+JSON.stringify(await bridge.request('ui_state_get')))};
const snapshot=()=>bridge.request('snapshot');
const counter=s=>s.plugins.find(p=>p.id==='com.elgato.counter');
const initial=await until(async()=>{const s=await snapshot();if(!s.simulate)throw Error('Simulation required');return s.plugin_language&&counter(s)?.connected&&s},'language and Counter ready');
const pid=counter(initial).pid;
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
const open=async()=>{button('Preferences').click();await until(()=>button('Startup & about'),'preferences tabs');button('Startup & about').click();await until(()=>document.querySelector('select[aria-label="Plugin language"]'),'plugin language control')};
const select=()=>document.querySelector('select[aria-label="Plugin language"]');
const choose=value=>{Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(select(),value);select().dispatchEvent(new Event('input',{bubbles:true}));select().dispatchEvent(new Event('change',{bubbles:true}));};
if(phase==='mixer'){
 const before=initial.mixer.config;
 if(before.enabled)throw Error('Mixer fixture must start with routing disabled');
 button('Audio mixer').click();
 await until(()=>button('＋ Add input group')&&button('＋ Add mix'),'mixer creation controls');
 button('＋ Add input group').click();
 const draft=async()=>(await bridge.request('ui_state_get'))?.formDrafts?.['audio-mixer'];
 await until(async()=>(await draft())?.config?.channels?.length===before.channels.length+1,'actual native Add input group');
 button('＋ Add mix').click();
 const edited=await until(async()=>{const value=await draft();return value?.config?.mixes?.length===before.mixes.length+1&&value},'actual native Add mix');
 const group=edited.config.channels.at(-1),mix=edited.config.mixes.at(-1);
 const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
 if(!uuid.test(group.id)||!uuid.test(mix.id)||group.id===mix.id)throw Error('Native fallback did not create distinct UUIDv4 records');
 const after=await snapshot();
 if(JSON.stringify(after.mixer.config)!==JSON.stringify(before)||after.mixer.active||edited.config.enabled||group.apps.length||group.sources.length||mix.outputs.length)throw Error('Draft creation applied audio routing or chose real endpoints');
 return{phase,randomUUID_available:typeof crypto.randomUUID==='function',secure_random_available:typeof crypto.getRandomValues==='function',actual_add_input_group:true,actual_add_mix:true,created_group:group,created_mix:mix,unapplied_draft:edited.dirty,backend_config_unchanged:true,audio_routes_enabled:false,usb_access:false};
}
await open();
if(phase==='pending'){
 if(initial.plugin_language.active!=='en')throw Error('Initial fixture locale must resolve to English');
 choose('de');
 await until(async()=>(await bridge.request('ui_state_get'))?.formDrafts?.['plugin-language']?.dirty,'retained language draft');
 if((await snapshot()).plugin_language.preference!==initial.plugin_language.preference)throw Error('Preference changed before Save');
 document.querySelector('[aria-label="Close Preferences"]').click();
 await until(()=>!document.querySelector('dialog[open]'),'preferences closed');await open();
 if(select().value!=='de')throw Error('Language draft lost on modal recreation');
 await bridge.request('command',{command:'plugin_language',value:'fr',baseline:initial.plugin_language.preference});
 await until(()=>button('Reload language')&&button('Save plugin language').disabled,'stale preference guard');
 if(select().value!=='de')throw Error('Snapshot erased language draft');
 button('Reload language').click();await until(()=>select().value==='fr','explicit language reload');
 choose('de');await until(()=>!button('Save plugin language').disabled,'language Save enabled');button('Save plugin language').click();
 const saved=await until(async()=>{const s=await snapshot();return s.plugin_language.preference==='de'&&s},'German pending preference');
 if(saved.plugin_language.active!=='en'||saved.plugin_language.resolved!=='de'||!saved.plugin_language.restart_required)throw Error('Pending language changed the running language');
 if(counter(saved).pid!==pid||counter(saved).name!==counter(initial).name)throw Error('Saving restarted or live-translated the running plugin');
 await until(()=>document.querySelector('.plugin-language').textContent.includes('Plugin language preference saved'),'native language save acknowledgement');
 if(!document.querySelector('.plugin-language').textContent.includes('Restart Deckforge to apply'))throw Error('Full restart requirement missing');
 return {phase,language:saved.plugin_language,plugin_pid:pid,plugin_name:counter(saved).name,draft_retained:true,stale_blocked:true,explicit_save:true,plugin_not_restarted:true,menus_english:!!button('Startup & about'),usb_access:false};
}
if(initial.plugin_language.preference!=='de'||initial.plugin_language.active!=='de'||initial.plugin_language.restart_required)throw Error('Full restart did not activate saved German language');
if(counter(initial).name!==expectedName||initial.actions.find(a=>a.uuid==='com.elgato.counter.action')?.name!==expectedName)throw Error('Original Counter German manifest labels not exposed');
if(select().value!=='de'||!button('Save plugin language').disabled)throw Error('Native control did not reflect saved preference');
// Create a nested action through the real translated picker. The profile must
// receive original source fields even though the UI continues showing German.
const menusEnglish=!!button('Startup & about');
document.querySelector('[aria-label="Close Preferences"]').click();
await until(()=>!document.querySelector('dialog[open]'),'preferences closed for nested editor');
const originalAction=initial.actions.find(a=>a.uuid==='com.elgato.counter.action');
if(!originalAction.source_name||!('source_states' in originalAction))throw Error('Original action metadata missing');
await bridge.request('command',{command:'assign',controller:'Encoder',index:0,uuid:'com.elgato.streamdeck.keys.adaptor'});
const outer=await until(async()=>{const s=await snapshot();return s.slots.find(slot=>slot.controller==='Encoder'&&slot.index===0&&slot.uuid==='com.elgato.streamdeck.keys.adaptor')},'Action Trigger assignment');
document.querySelectorAll('.deck-dial')[0].click();
await until(()=>document.querySelector('.sequence[aria-label="Action trigger"]'),'native trigger editor');
document.querySelectorAll('.sequence-select')[1].click();
const actionSelect=document.querySelector('select[aria-label="Action to add"]');
Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(actionSelect,'com.elgato.counter.action');actionSelect.dispatchEvent(new Event('change',{bubbles:true}));
await until(()=>button('Assign')&&!button('Assign').disabled,'translated child Assign enabled');button('Assign').click();
const created=await until(async()=>{const child=(await bridge.request('ui_state_get'))?.drafts?.[outer.context]?.raw?.Actions?.[1];return child?.UUID==='com.elgato.counter.action'&&child},'nested Counter draft');
if(created.Name!==originalAction.source_name||JSON.stringify(created.States)!==JSON.stringify(originalAction.source_states??[]))throw Error('Translated presentation leaked into a newly created child');
await until(()=>button('Save changes')&&!button('Save changes').disabled,'nested Save enabled');button('Save changes').click();
const savedChild=await until(async()=>{const s=await snapshot();return s.slots.find(slot=>slot.context===outer.context)?.raw?.Actions?.[1]?.UUID==='com.elgato.counter.action'&&s.slots.find(slot=>slot.context===outer.context).raw.Actions[1]},'nested source record saved');
if(savedChild.Name!==originalAction.source_name||JSON.stringify(savedChild.States)!==JSON.stringify(originalAction.source_states??[]))throw Error('Saved child lost original source metadata');
await until(()=>document.querySelector('.child-editor .section-label')?.textContent===expectedName,'localized child heading');
if(document.querySelectorAll('.sequence-select strong')[1].textContent!==expectedName)throw Error('Nested list label not localized');
return {phase,language:initial.plugin_language,plugin_pid:pid,plugin_name:counter(initial).name,full_restart_applied:true,menus_english:menusEnglish,nested_source_name:savedChild.Name,nested_source_states:savedChild.States,nested_localized_heading:expectedName,usb_access:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=PROJECT / "target/release/deckforge")
    parser.add_argument("--samples", type=Path, default=PROJECT.parent / "evidence/official-samples")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, samples, output = args.binary.resolve(strict=True), args.samples.resolve(strict=True), args.output.resolve()
    plugin = samples / "counter/com.elgato.counter.sdPlugin"
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    assert subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip() == SAMPLE_COMMIT
    expected_name = json.loads((plugin / "de.json").read_text())["Name"]
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "sample_commit": SAMPLE_COMMIT, "runs": []}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-language-") as temporary:
        root = Path(temporary)
        state = root / "state"
        for phase in ("pending", "active", "mixer"):
            script, proof = root / f"{phase}.js", root / f"{phase}.json"
            script.write_text(SCRIPT.replace("__PHASE__", json.dumps(phase)).replace("__NAME__", json.dumps(expected_name)))
            env = environment(root)
            env.update(LC_ALL="C.UTF-8", DECKFORGE_CAPTURE=str(output / f"{phase}.png"), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
            log_path = output / f"{phase}.log"
            args = [str(binary), "desktop", "--simulate", "--state", str(state)]
            if phase == "pending":
                args.extend(["--plugin", str(plugin)])
            with log_path.open("w") as log:
                process = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    assert process.wait(timeout=40) == 0, log_path.read_text()
                    value = result(proof)
                    assert value, log_path.read_text()
                    report["runs"].append(value["result"])
                except Exception as error:
                    report["error"] = str(error)
                    raise
                finally:
                    cleanup(process)
                    (output / "language.json").write_text(json.dumps(report, indent=2) + "\n")
    report["ok"] = True
    (output / "language.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native language drafts/stale guard, explicit pending save, unchanged running plugin, full owned-app restart activates original German Counter labels, source child records and native mixer creation; no USB/user state.")


if __name__ == "__main__":
    main()
