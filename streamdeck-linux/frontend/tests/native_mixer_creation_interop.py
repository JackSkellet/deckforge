#!/usr/bin/env python3
"""Click native mixer creation buttons in isolated disabled-routing state."""
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
const expectFailure=__FAILURE__, bridge=window.deckforge;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<150;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const initial=await bridge.request('snapshot');if(!initial.simulate||initial.mixer.config.enabled)throw Error('Requires simulated, disabled-routing fixture');
const errors=[];window.addEventListener('error',event=>errors.push(String(event.message)));
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
button('Audio mixer').click();
const draft=async()=>(await bridge.request('ui_state_get'))?.formDrafts?.['audio-mixer'];
const before=await until(async()=>button('＋ Add input group')&&button('＋ Add mix')&&await draft(),'mixer controls');
if(button('＋ Add input group').disabled||button('＋ Add mix').disabled)throw Error('Creation buttons unexpectedly disabled');
button('＋ Add input group').click();await pause(250);
button('＋ Add mix').click();await pause(250);
const after=await draft();
const counts=config=>({groups:config.channels.length,mixes:config.mixes.length});
if(expectFailure){
 if(typeof crypto.randomUUID!=='undefined')throw Error('Negative control has randomUUID available');
 if(JSON.stringify(counts(before.config))!==JSON.stringify(counts(after.config)))throw Error('Expected old native creation failure was not reproduced');
}else{
 if(after.config.channels.length!==before.config.channels.length+1||after.config.mixes.length!==before.config.mixes.length+1||!after.dirty)throw Error('Real native buttons did not create and retain both drafts');
 const group=after.config.channels.at(-1),mix=after.config.mixes.at(-1);
 const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
 if(!uuid.test(group.id)||!uuid.test(mix.id)||group.id===mix.id)throw Error('Created records need distinct UUIDv4 identities');
 if(group.apps.length||group.sources.length||mix.outputs.length)throw Error('Draft chose real audio endpoints');
 document.querySelector('[aria-label="Close Audio mixer"]').click();
 await until(()=>!document.querySelector('dialog[open]'),'mixer closed');button('Audio mixer').click();
 await until(async()=>JSON.stringify(counts((await draft()).config))===JSON.stringify(counts(after.config)),'creation drafts retained after reopening');
}
const backend=await bridge.request('snapshot');
if(JSON.stringify(backend.mixer.config)!==JSON.stringify(initial.mixer.config)||backend.mixer.active||after.config.enabled)throw Error('Creation modified actual audio routing');
return{expected_creation_failure:expectFailure,actual_buttons_clicked:['Add input group','Add mix'],before:counts(before.config),after:counts(after.config),dirty:after.dirty,randomUUID_available:typeof crypto.randomUUID==='function',secure_random_available:typeof crypto.getRandomValues==='function',errors,backend_config_unchanged:true,audio_routes_enabled:false,applied:false,usb_access:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expect-creation-failure", action="store_true")
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-mixer-create-") as temporary:
        root = Path(temporary)
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT.replace("__FAILURE__", json.dumps(args.expect_creation_failure)))
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(output / "mixer.png"), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = output / "mixer.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary), "desktop", "--simulate", "--state", str(root / "state")], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=35) == 0, log_path.read_text()
                value = result(proof)
                assert value, log_path.read_text()
                report.update(ok=True, native=value["result"])
            except Exception as error:
                report["error"] = str(error)
                raise
            finally:
                cleanup(process)
                (output / "mixer.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: expected native mixer creation outcome verified through actual buttons; no Apply/audio routing/USB.")


if __name__ == "__main__":
    main()
