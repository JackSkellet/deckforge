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
const port=label=>[...document.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")===label);
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
const sinks=(await bridge.request('snapshot')).mixer.sinks;
const listening=sinks.find(s=>s.description==='AirPods Pro'&&!s.owned)||sinks.find(s=>!s.owned&&!s.name.startsWith('deckforge.')&&s.channels?.length===2);
if(listening){const name=listening.description||listening.name;for(const mix of ['Personal','Stream']){port('Connect from '+mix).click();await pause(30);port('Connect to '+name).click();await pause(50);}await until(async()=>(await draft()).config.mixes.every(m=>m.outputs.includes(listening.name)),'multiple mixes on same output');}
const header=port('Move System'),node=header.closest('article');const oldX=parseFloat(node.style.left);
header.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',shiftKey:true,bubbles:true}));
await until(()=>parseFloat(node.style.left)===oldX+50,'movable node');
await until(async()=>(await bridge.request('ui_state_get')).formDrafts['audio-mixer-canvas']?.positions['channel:system']?.x===oldX+50,'retained canvas position');
port('Show individual channels for System').click();port('Show individual channels for Personal').click();
await until(()=>port('Connect from System Left')&&port('Connect to Personal Right'),'individual stereo ports');
port('Connect from System Left').click();await pause(30);port('Connect to Personal Right').click();
await until(async()=>(await draft()).config.connections?.some(c=>c.source_channel==='front-left'&&c.target_channel==='front-right'),'single-channel route');
if(__COMPACT__)document.querySelector('dialog').style.width='760px';
const zoom=document.querySelector('select[aria-label="Routing graph zoom"]');zoom.value='0.8';zoom.dispatchEvent(new Event('change',{bubbles:true}));await pause(100);
const viewport=document.querySelector('.patchbay-viewport');if(viewport.scrollWidth<=viewport.clientWidth&&__COMPACT__)throw Error('Compact canvas must scroll horizontally');
const beforeApply=await bridge.request('snapshot');if(beforeApply.mixer.config.routes.length||beforeApply.mixer.active)throw Error('Draft changed live routing');
button('Apply routes').click();await until(async()=>(await bridge.request('snapshot')).mixer.config.routes.length===2&&(await bridge.request('snapshot')).mixer.config.connections.length===1,'saved graph');
const after=await bridge.request('snapshot');if(after.mixer.config.enabled||after.mixer.active)throw Error('Disabled graph enabled audio');
button('Expand canvas').click();await pause(80);
if(!document.querySelector('.mixer-patchbay.expanded'))throw Error('Expanded canvas missing');
return{multiple_mixes_one_output:!!listening,saved_canvas_x:oldX+50,stereo_ports:true,individual_channel_route:true,expanded:true,cables:document.querySelectorAll('.patchbay-cable-hit').length,keyboard_disconnect:true,draft_retained:true,applied_disabled_graph:true,explicit_listen_mix:true,compact:__COMPACT__,usb_access:false,audio_enabled:false};
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
                stored=json.loads((root / "state/audio-canvas.json").read_text())
                assert stored['positions']['channel:system']['x']==value['result']['saved_canvas_x']
                assert stored['split']['channel:system'] and stored['split']['mix:monitor']
                restore_script = r'''
const bridge=window.deckforge,pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn)=>{for(let n=0;n<150;n++){const result=await fn();if(result)return result;await pause(40)}throw Error('Canvas restore timeout')};
const state=await bridge.request('ui_state_get'),canvas=state.formDrafts?.['audio-mixer-canvas'];if(!canvas?.positions['channel:system']||!canvas.split['mix:monitor'])throw Error('Canvas not loaded on restart');
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
button('Audio mixer').click();await until(()=>button('Advanced routing'));button('Advanced routing').click();
const header=await until(()=>document.querySelector('[aria-label="Move System"]'));
if(parseFloat(header.closest('article').style.left)!==canvas.positions['channel:system'].x)throw Error('Restored node position differs');
await until(()=>document.querySelector('[aria-label="Connect from System Left"]'));
const config=(await bridge.request('snapshot')).mixer.config;if(config.enabled||config.connections.length!==1)throw Error('Saved channel routing not restored');
return{node_position_restored:true,split_ports_restored:true,routing_restored:true,audio_enabled:false};
'''
                script.write_text(restore_script)
                proof.unlink()
                env['DECKFORGE_CAPTURE']=str(output/'restart.png')
                restarted=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    assert restarted.wait(timeout=35)==0,log_path.read_text()
                    report['restart']=result(proof)['result']
                finally:cleanup(restarted)
            except Exception as error:
                report["error"] = str(error)
                raise
            finally:
                cleanup(process)
                (output / "mixer.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native patchbay connections, keyboard disconnect, draft preservation and disabled Apply; no audio routing/USB.")


if __name__ == "__main__":
    main()
