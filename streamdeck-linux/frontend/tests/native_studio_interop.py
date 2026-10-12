#!/usr/bin/env python3
"""Verify Studio in the app-owned WebKit fixture using isolated simulated state."""
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
const until=async(fn,name)=>{for(let i=0;i<160;i++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const initial=await bridge.request('snapshot');
if(!initial.simulate||initial.device_model!=='studio')throw Error('Studio simulation required');
const device=initial.device;
const command=payload=>bridge.request('command',{...payload,device});
const keys=()=>[...document.querySelectorAll('.deck-key')];
const dials=()=>[...document.querySelectorAll('.deck-dial')];
await until(()=>keys().length===32&&dials().length===2,'Studio controls');
if(initial.device_geometry.strip_width>0||initial.device_geometry.strip_height>0||initial.device_capabilities.touchscreen)throw Error('Unexpected Studio touch display geometry');
const gridColumns=getComputedStyle(document.querySelector('.key-grid')).gridTemplateColumns.split(' ').length;
if(gridColumns!==16)throw Error('Studio must have16 key columns');
const bounds=keys().map(key=>{const r=key.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}});
if(bounds.some(r=>r.width<18||r.height<18||r.x<0||r.right>innerWidth+1||r.y<0||r.bottom>innerHeight+1))throw Error('Studio key bounds invalid: '+JSON.stringify(bounds));
if(Math.abs(bounds[0].width/bounds[0].height-144/112)>.08)throw Error('Studio key aspect ratio is wrong');
await command({command:'assign',controller:'Keypad',index:31,uuid:'com.elgato.streamdeck.page.next'});
await until(()=>keys()[31].getAttribute('aria-label').includes('Next'),'last key assignment');
keys()[31].click();
await until(async()=>{const ui=await bridge.request('ui_state_get');return ui?.selected?.controller==='Keypad'&&ui.selected.index===31},'last key selection');
// An empty dial stack has no external side effects. This test never sends input.
await command({command:'assign',controller:'Encoder',index:1,uuid:'com.elgato.streamdeck.dial.stack'});
await until(()=>!dials()[1].getAttribute('aria-label').includes('Unassigned'),'dial assignment');
dials()[1].click();
await until(()=>document.body.textContent.includes('Studio LED and ring feedback is not supported.'),'screenless feedback boundary');
if(dials().some(dial=>dial.querySelector(':scope > img')))throw Error('Studio dial rendered a fake screen');
if(document.querySelector('.appearance-preview'))throw Error('Studio dial offered display appearance');
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
if(document.body.textContent.includes('Show stack icon'))throw Error('Screenless dial offered stack display option');
if(!keys()[31].title.includes('Next')||!dials()[1].title.includes('Dial Stack'))throw Error('Native control tooltips lack action labels');
if(button('Tap screen')||button('Hold screen'))throw Error('Studio dial offered touch input');
if(!button('Test push')||!button('Turn right ↷'))throw Error('Studio dial lacks push/rotation controls');
dials()[1].focus();
if(document.activeElement!==dials()[1])throw Error('Studio dial is not focusable');
dials()[1].dispatchEvent(new KeyboardEvent('keydown',{key:'F10',shiftKey:true,bubbles:true}));
const copy=await until(()=>[...document.querySelectorAll('.control-edit-menu[open] button')].find(b=>b.textContent.includes('Copy')),'keyboard edit menu');
copy.click();
const clipboard=await until(async()=>{const s=await bridge.request('snapshot');return s.clipboard?.controller==='Encoder'&&s.clipboard},'copied encoder');
dials()[0].click();
await until(async()=>{const ui=await bridge.request('ui_state_get');return ui?.selected?.controller==='Encoder'&&ui.selected.index===0},'paste destination selected');
dials()[0].dispatchEvent(new KeyboardEvent('keydown',{key:'v',ctrlKey:true,bubbles:true}));
await until(async()=>{const s=await bridge.request('snapshot');return s.slots.some(slot=>slot.controller==='Encoder'&&slot.index===0&&slot.uuid==='com.elgato.streamdeck.dial.stack'&&slot.context!==clipboard.context)},'keyboard paste to other dial');
const dialBounds=dials().map(dial=>{const r=dial.getBoundingClientRect();return{x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:r.width,height:r.height}});
if(dialBounds.some(r=>r.width<44||r.height<44||r.x<0||r.right>innerWidth+1||r.y<0||r.bottom>innerHeight+1))throw Error('Studio dial bounds invalid');
return{model:'studio',keys:32,columns:gridColumns,dials:2,key_bounds:bounds,dial_bounds:dialBounds,last_key_selected:true,keyboard_menu_and_paste:true,dial_display:false,touch_controls:false,led_feedback_unsupported_notice:true,display_stack_option_hidden:true,native_control_tooltips:true,viewport:[innerWidth,innerHeight],usb_access:false,audio_actions:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=PROJECT / "target/release/deckforge")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-studio-") as temporary:
        root = Path(temporary)
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT)
        capture, log_path = output / "studio.png", output / "studio.log"
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(capture), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary), "desktop", "--simulate", "--model", "studio", "--state", str(root / "state"), "--port", "0"], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=40) == 0, log_path.read_text()
                value = result(proof)
                assert value, log_path.read_text()
                report["native"] = value["result"]
                report["capture_sha256"] = hashlib.sha256(capture.read_bytes()).hexdigest()
                report["ok"] = True
            except Exception as error:
                report["error"] = str(error)
                raise
            finally:
                cleanup(process)
                (output / "studio.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: native Studio geometry, screenless dials, keyboard menu/copy/paste; no USB/audio.")


if __name__ == "__main__":
    main()
