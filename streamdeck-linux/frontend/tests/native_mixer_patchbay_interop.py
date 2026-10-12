#!/usr/bin/env python3
"""Connect and disconnect the native routing graph in isolated disabled state."""
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
const until=async(fn,name)=>{for(let n=0;n<150;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const initial=await bridge.request('snapshot');if(!initial.simulate||initial.mixer.config.enabled)throw Error('Requires simulated disabled state');
const config={...initial.mixer.config,channels:[{id:'system',name:'System',apps:[],sources:[],capture_unassigned:true},{id:'voice',name:'Voice',apps:[],sources:[]}],mixes:[{id:'monitor',name:'Personal',outputs:[],volume:100,muted:false},{id:'stream',name:'Stream',outputs:[],volume:100,muted:false}],routes:[]};
await bridge.request('command',{command:'mixer_config',settings:config,expected:initial.mixer.config});
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
const port=label=>document.querySelector('button[aria-label="'+label+'"]');
button('Audio mixer').click();await until(()=>button('Advanced routing'),'advanced tab');button('Advanced routing').click();
await until(()=>port('Connect from System'),'graph');port('Connect from System').click();await pause(30);port('Connect to Personal').click();await pause(50);
port('Connect from Voice').click();await pause(30);port('Connect to Stream').click();await pause(50);
const draft=async()=>(await bridge.request('ui_state_get')).formDrafts['audio-mixer'];
await until(async()=>(await draft()).config.routes.length===2,'draft connections');
const link=[...document.querySelectorAll('.patchbay-cable-hit')].find(path=>path.getAttribute('aria-label').startsWith('Voice to Stream:'));
link.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));await until(()=>button('Disconnect'),'keyboard cable selection');button('Disconnect').click();
await until(async()=>(await draft()).config.routes.length===1,'disconnect');
port('Connect from Voice').click();await pause(30);port('Connect to Stream').click();await pause(80);
button('Mixer').click();await pause(80);if(![...document.querySelectorAll('label')].some(l=>l.textContent.includes('Listen to mix')))throw Error('Explicit listening mix selector missing');
button('Advanced routing').click();await until(()=>document.querySelectorAll('.patchbay-cable-hit').length>=5,'graph retained');
if(__COMPACT__)document.querySelector('dialog').style.width='760px';
const zoom=document.querySelector('select[aria-label="Routing graph zoom"]');zoom.value='0.8';zoom.dispatchEvent(new Event('change',{bubbles:true}));await pause(100);
const viewport=document.querySelector('.patchbay-viewport');if(viewport.scrollWidth<=viewport.clientWidth&&__COMPACT__)throw Error('Compact canvas must scroll horizontally');
const beforeApply=await bridge.request('snapshot');if(beforeApply.mixer.config.routes.length||beforeApply.mixer.active)throw Error('Draft changed live routing');
button('Apply routes').click();await until(async()=>(await bridge.request('snapshot')).mixer.config.routes.length===2,'saved graph');
const after=await bridge.request('snapshot');if(after.mixer.config.enabled||after.mixer.active)throw Error('Disabled graph enabled audio');
return{cables:document.querySelectorAll('.patchbay-cable-hit').length,keyboard_disconnect:true,draft_retained:true,applied_disabled_graph:true,explicit_listen_mix:true,compact:__COMPACT__,usb_access:false,audio_enabled:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compact", action="store_true")
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-patchbay-") as temporary:
        root = Path(temporary)
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT.replace("__COMPACT__", json.dumps(args.compact)))
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
    print("PASS: native patchbay connections, keyboard disconnect, draft preservation and disabled Apply; no audio routing/USB.")


if __name__ == "__main__":
    main()
