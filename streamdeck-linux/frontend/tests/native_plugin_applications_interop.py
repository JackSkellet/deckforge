#!/usr/bin/env python3
"""Native application-mapping editor, isolated state and unchanged official SDK.

No monitored application is launched. All mappings use nonexistent fixture names.
"""
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

PLUGIN = "com.deckforge.application-ui-probe"
SCRIPT = r'''
const bridge=window.deckforge, plugin='com.deckforge.application-ui-probe';
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,name)=>{for(let n=0;n<150;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const snapshot=()=>bridge.request('snapshot');
const current=async()=>{const s=await snapshot();if(!s.simulate)throw Error('Simulation required');return s.plugins.find(p=>p.id===plugin)};
await until(async()=>{const p=await current();return p?.connected&&p.applications.length===3},'declared applications');
const open=()=>[...document.querySelectorAll('.header-tools button')].find(b=>b.textContent.trim().startsWith('Plugins')).click();
open();
await until(()=>document.querySelector('.plugin-applications'),'application mappings section');
document.querySelector('.plugin-applications summary').click();
const input=name=>document.querySelector(`input[aria-label="Linux executable for ${name}"]`);
const set=(element,value)=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(element,value);element.dispatchEvent(new Event('input',{bubbles:true}));element.dispatchEvent(new Event('change',{bubbles:true}));};
const button=name=>[...document.querySelectorAll('.plugin-applications button')].find(b=>b.textContent.trim()===name);
const win='DeckforgeFixture.exe', linux='deckforge-unlaunched-native-fixture';
await until(()=>input(win)&&input(linux),'application inputs');
if(input(win).value!==''||input(linux).value!==linux)throw Error('Windows/Linux default mapping policy changed');
set(input(win),'deckforge-unsaved-fixture');
await until(async()=>(await bridge.request('ui_state_get'))?.formDrafts?.[`plugin-applications:${plugin}`]?.dirty,'persisted native mapping draft');
if(Object.keys((await current()).application_mappings).length)throw Error('Mapping changed before Apply');
document.querySelector('[aria-label="Close Plugins"]').click();
await until(()=>!document.querySelector('dialog[open]'),'closed plugins');open();
await until(()=>input(win)?.value==='deckforge-unsaved-fixture','draft restored after modal recreation');
const external={ [win]:'deckforge-external-fixture' };
await bridge.request('command',{command:'plugin_applications',id:plugin,mappings:external,baseline:{}});
await until(()=>button('Reload mappings')&&button('Apply mappings')?.disabled,'stale draft protection');
if(input(win).value!=='deckforge-unsaved-fixture')throw Error('Snapshot overwrote unsaved mapping');
button('Reload mappings').click();
await until(()=>input(win).value==='deckforge-external-fixture','explicit reload');
set(input(win),'deckforge-ui-mapped-fixture');set(input(linux),'');
await until(()=>button('Apply mappings')&&!button('Apply mappings').disabled,'Apply enabled');
const submittedBaseline=(await bridge.request('ui_state_get')).formDrafts[`plugin-applications:${plugin}`].baseline;
if(JSON.stringify(submittedBaseline)!==JSON.stringify(external))throw Error('Reload did not capture the current baseline');
button('Apply mappings').scrollIntoView({block:'nearest'});button('Apply mappings').click();
const expected={ [win]:'deckforge-ui-mapped-fixture',[linux]:'' };
await until(async()=>JSON.stringify((await current()).application_mappings)===JSON.stringify(expected),'saved explicit mappings');
await until(()=>document.querySelector('.plugin-applications').textContent.includes('Application mappings saved'),'native Apply status');
const savedDraft=(await bridge.request('ui_state_get')).formDrafts[`plugin-applications:${plugin}`];
if(savedDraft.dirty||JSON.stringify(savedDraft.baseline)!==JSON.stringify(expected))throw Error('Saved native draft was not acknowledged');
if(input(linux).value!=='')throw Error('Blank Linux override was replaced by default');
return{plugin,explicit_apply:true,modal_draft_retained:true,native_state_draft_retained:true,stale_blocked:true,reload_explicit:true,submitted_baseline:submittedBaseline,saved_baseline:savedDraft.baseline,applications:(await current()).applications,mappings:(await current()).application_mappings,launched_monitored_applications:false,usb_access:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=PROJECT / "target/release/deckforge")
    parser.add_argument("--samples", type=Path, default=PROJECT.parent / "evidence/official-samples")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, samples, output = args.binary.resolve(strict=True), args.samples.resolve(strict=True), args.output.resolve()
    sdk = samples / "counter/node_modules/@elgato/streamdeck"
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    assert subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip() == SAMPLE_COMMIT
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "sample_commit": SAMPLE_COMMIT}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-application-ui-") as temporary:
        root = Path(temporary)
        plugin = root / "application-ui-probe.sdPlugin"
        plugin.mkdir()
        (plugin / "plugin.mjs").write_text("import streamDeck from " + json.dumps((sdk / "dist/plugin/index.js").as_uri()) + "; await streamDeck.connect();\n")
        (plugin / "manifest.json").write_text(json.dumps({"UUID": PLUGIN, "Name": "Application monitoring probe", "Version": "1.0.0", "SDKVersion": 2, "Software": {"MinimumVersion": "7.1"}, "CodePath": "plugin.mjs", "Nodejs": {"Version": "20"}, "ApplicationsToMonitor": {"windows": ["DeckforgeFixture.exe"], "mac": ["com.deckforge.fixture"], "linux": ["deckforge-unlaunched-native-fixture"]}, "Actions": [{"UUID": PLUGIN + ".key", "Name": "Application probe", "Controllers": ["Keypad"], "States": [{}]}]}))
        script, proof = root / "ui.js", root / "proof.json"
        script.write_text(SCRIPT)
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(output / "applications.png"), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = output / "applications.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary), "desktop", "--simulate", "--state", str(root / "state"), "--plugin", str(plugin)], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=40) == 0, log_path.read_text()
                value = result(proof)
                assert value, log_path.read_text()
                report.update(ok=True, native=value["result"], capture_sha256=hashlib.sha256((output / "applications.png").read_bytes()).hexdigest())
            except Exception as error:
                report["error"] = str(error)
                raise
            finally:
                cleanup(process)
                (output / "applications.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native application mapping defaults, retained drafts, stale guard, explicit reload/Apply and saved baseline; no monitored process launches or USB.")


if __name__ == "__main__":
    main()
