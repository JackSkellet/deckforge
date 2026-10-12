#!/usr/bin/env python3
"""Exercise native immediate mixer controls with isolated disabled-routing state."""
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

SCRIPT = r'''
const bridge=window.deckforge;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<180;n++){const value=await fn();if(value)return value;await pause(30)}throw Error('Timed out: '+name)};
const initial=await bridge.request('snapshot');if(!initial.simulate||initial.mixer.config.enabled)throw Error('Requires simulated, disabled-routing fixture');
const config={enabled:false,fade_ms:0,channels:[{id:'fixture-voice',name:'Voice',apps:[],sources:[],linked:false}],mixes:[{id:'monitor',name:'Monitor',outputs:['fixture.output'],volume:90,muted:false,output_controls:{}},{id:'stream',name:'Stream',outputs:[],volume:100,muted:false,output_controls:{}}],routes:[{channel:'fixture-voice',mix:'monitor',volume:40,muted:false},{channel:'fixture-voice',mix:'stream',volume:60,muted:false}]};
await bridge.request('command',{command:'mixer_config',settings:config,expected:initial.mixer.config});
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
const input=label=>document.querySelector(`[aria-label="${label}"]`)||[...document.querySelectorAll('label')].find(l=>l.textContent.trim()===label)?.querySelector('input');
const set=(node,value)=>{if(!node)throw Error('Missing control');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(node,String(value));node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new Event('change',{bubbles:true}));};
const saved=async()=>(await bridge.request('snapshot')).mixer;
const draft=async()=>(await bridge.request('ui_state_get'))?.formDrafts?.['audio-mixer'];
button('Audio mixer').click();await until(()=>input('Voice → Monitor volume'),'mixer visible');
if((await saved()).revision===undefined)throw Error('Host has no graph revision');
// A new draft mix must survive every live update without becoming saved.
button('＋ Add mix').click();await until(async()=>(await draft())?.config.mixes.length===3,'draft mix');
set(input('Voice → Monitor volume'),52);await until(async()=>(await saved()).config.routes[0].volume===52,'immediate route volume');
input('Link Voice mix levels').click();await until(async()=>(await saved()).config.channels[0].linked,'immediate link');
set(input('Voice → Monitor volume'),62);await until(async()=>(await saved()).config.routes[1].volume===70,'linked percentage delta');
input('Mute Voice → Monitor').click();await until(async()=>(await saved()).config.routes[0].muted,'route mute');
if((await saved()).config.routes[1].muted)throw Error('Linked mutes were not independent');
set(input('Monitor master volume'),77);await until(async()=>(await saved()).config.mixes[0].volume===77,'master level');
[...document.querySelectorAll('summary')].find(s=>s.textContent.startsWith('Monitor outputs')).click();
set(input('Monitor output · fixture.output volume'),33);await until(async()=>(await saved()).config.mixes[0].output_controls['fixture.output']?.volume===33,'output level');
input('Mute Monitor output · fixture.output').click();await until(async()=>(await saved()).config.mixes[0].output_controls['fixture.output'].muted,'output mute');
set(input('Default fade (ms)'),250);await until(async()=>(await saved()).config.fade_ms===250,'default fade');
for(const value of [63,64,65,66])set(input('Voice → Monitor volume'),value);
await until(async()=>(await saved()).config.routes[0].volume===66,'rapid final value');
const pending=await draft();if(!pending.dirty||pending.config.mixes.length!==3||(await saved()).config.mixes.length!==2)throw Error('Live updates lost or saved the draft mix');
document.querySelector('[aria-label="Close Audio mixer"]').click();await pause(100);button('Audio mixer').click();await until(()=>input('Voice → Monitor volume'),'reopened mixer');
if((await draft()).config.mixes.length!==3)throw Error('Reopening lost topology draft');
const snapshot=await saved();if(snapshot.active||snapshot.config.enabled)throw Error('Inactive preset controls enabled audio');
if(!document.querySelector('[aria-label="Voice: meter unavailable"]'))throw Error('Unavailable meter must not display invented silence');
const dialog=document.querySelector('dialog[open]');if(dialog.scrollWidth>dialog.clientWidth+2)throw Error('Dialog has horizontal overflow');
return{actual_controls:['route volume','linked levels','independent route mute','master volume','output gain','output mute','default fade','rapid fader updates'],saved:snapshot.config,revision:snapshot.revision,draft_retained:true,draft_mixes:3,saved_mixes:2,meters_unavailable:true,dialog_no_horizontal_overflow:true,routing_enabled:false,usb_access:false};
'''



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-mixer-controls-") as temporary:
        root = Path(temporary)
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT)
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(output / "mixer.png"), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = output / "mixer.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary), "desktop", "--simulate", "--state", str(root / "state")], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=50) == 0, log_path.read_text()
                value = result(proof)
                assert value, log_path.read_text()
                report.update(ok=True, native=value["result"])
            except Exception as error:
                report["error"] = str(error)
                raise
            finally:
                cleanup(process)
                (output / "mixer.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native mixer controls saved immediately, routing draft retained, audio routing stayed disabled.")


if __name__ == "__main__":
    main()
