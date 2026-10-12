#!/usr/bin/env python3
"""Choose installed applications by name in a native, disabled-routing fixture."""
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
const until=async(fn,name)=>{for(let n=0;n<160;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
const input=label=>[...document.querySelectorAll('label')].find(l=>l.querySelector('span')?.textContent===label)?.querySelector('input,select');
const set=(node,value)=>{if(!node)throw Error('Missing control');Object.getOwnPropertyDescriptor(node.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(node,String(value));node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new Event('change',{bubbles:true}));};
const saved=async()=>(await bridge.request('snapshot')).mixer;
const draft=async()=>(await bridge.request('ui_state_get'))?.formDrafts?.['audio-mixer'];
const initial=await bridge.request('snapshot');if(!initial.simulate||initial.mixer.config.enabled)throw Error('Requires simulated disabled-routing state');
button('Audio mixer').click();
await until(async()=>(await saved()).applications?.some(app=>app.id==='deckforge-fixture-chat.desktop'),'isolated desktop catalog');
button('＋ Create channel').click();await pause(100);
const firstMenu=document.querySelector('#mixer-create-menu').getBoundingClientRect(),firstDialog=document.querySelector('dialog[open]').getBoundingClientRect();
const emptyMenuBounds={top:firstMenu.top,bottom:firstMenu.bottom,height:firstMenu.height};
if(firstMenu.top<firstDialog.top+50||firstMenu.bottom>firstDialog.bottom-45)throw Error('Empty mixer menu is clipped: '+JSON.stringify(emptyMenuBounds));
set(input('Search apps and inputs'),'Aurora');await until(()=>[...document.querySelectorAll('.mixer-create-results button')].some(b=>b.textContent.startsWith('Aurora Chat')),'friendly search');
if([...document.querySelectorAll('.mixer-create-results button')].some(b=>b.textContent.includes('Fixture Editor')))throw Error('Search did not filter catalog');
[...document.querySelectorAll('.mixer-create-results button')].find(b=>b.textContent.startsWith('Aurora Chat')).click();
await until(async()=>(await draft())?.config.channels[0]?.apps[0]?.field==='desktop_application','auto-bound desktop channel');
let edited=await draft();if(edited.config.channels[0].name!=='Aurora Chat'||edited.config.channels[0].apps[0].value!=='deckforge-fixture-chat.desktop')throw Error('Wrong friendly name or desktop identity');
document.querySelector('[aria-label="Add route Aurora Chat to Monitor"]').click();
button('Routing setup').click();await pause(50);
document.querySelector('.mixer-input-details summary').click();await pause(80);
if(!document.querySelector('.selector-list').textContent.includes('Pending audio'))throw Error('Nonplaying app should be pending');
if(document.querySelector('.manual-selector').open)throw Error('Manual identity should remain Advanced and closed');
input('Default for other apps · Aurora Chat').click();await until(async()=>(await draft()).config.channels[0].capture_unassigned,'first default group');
button('Mixer').click();await pause(40);button('＋ Create channel').click();await pause(40);
[...document.querySelectorAll('#mixer-create-menu > button')].find(b=>b.textContent.startsWith('All audio')).click();
await until(async()=>{const channels=(await draft()).config.channels;return channels[1]?.capture_unassigned&&!channels[0].capture_unassigned},'unique catch-all');
if((await saved()).config.channels.length!==initial.mixer.config.channels.length)throw Error('Discovery or channel creation applied routing automatically');
document.querySelector('[aria-label="Close Audio mixer"]').click();await pause(80);button('Audio mixer').click();await until(async()=>(await draft())?.config.channels[0].apps[0]?.value==='deckforge-fixture-chat.desktop','draft retained');
button('Apply routes').click();await until(async()=>(await saved()).config.channels[0]?.apps[0]?.value==='deckforge-fixture-chat.desktop','explicit apply');
await until(()=>!document.querySelector('.mixer-editable').disabled,'Apply UI completion');
const final=await saved();if(final.active||final.config.enabled)throw Error('Application chooser enabled audio');
if(final.config.channels.filter(channel=>channel.capture_unassigned).length!==1||!final.config.channels[1].capture_unassigned)throw Error('Default group did not persist uniquely');
let chooserWidth=null;
if(__VIEW__==='setup'){button('Routing setup').click();await pause(50);document.querySelector('.mixer-input-details summary').click();await pause(50);const chooser=input('Installed application for Aurora Chat');chooser.scrollIntoView({block:'center'});await pause(100);chooserWidth=chooser.getBoundingClientRect().width;}
let menuBounds=null;
if(__VIEW__==='create'){button('＋ Create channel').click();const menu=await until(()=>document.querySelector('#mixer-create-menu'),'visible Create menu');await pause(100);const rect=menu.getBoundingClientRect();menuBounds={top:rect.top,bottom:rect.bottom,height:rect.height};const dialogRect=document.querySelector('dialog[open]').getBoundingClientRect();if(rect.top<dialogRect.top+50||rect.bottom>dialogRect.bottom-45)throw Error('Create menu is clipped outside usable dialog bounds: '+JSON.stringify(menuBounds));}
const dialog=document.querySelector('dialog[open]');if(dialog.scrollWidth>dialog.clientWidth+2)throw Error('Horizontal dialog overflow');
return{installed:final.applications.filter(app=>app.id.startsWith('deckforge-fixture-')),selected:final.config.channels[0].apps[0],friendly_search:true,identity_autofilled:true,pending_audio:true,draft_retained:true,default_group_unique:true,explicit_apply:true,empty_menu_bounds:emptyMenuBounds,menu_bounds:menuBounds,chooser_width:chooserWidth,view:__VIEW__,routing_enabled:false,applications_launched:false,usb_access:false};
'''



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--view", choices=["mixer", "setup", "create"], default="mixer")
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-mixer-applications-") as temporary:
        root = Path(temporary)
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT.replace("__VIEW__", json.dumps(args.view)))
        desktop_dir = root / "data" / "applications"
        desktop_dir.mkdir(parents=True)
        (desktop_dir / "deckforge-fixture-chat.desktop").write_text("[Desktop Entry]\nType=Application\nName=Aurora Chat\nExec=/usr/bin/false\n")
        (desktop_dir / "deckforge-fixture-editor.desktop").write_text("[Desktop Entry]\nType=Application\nName=Fixture Editor\nExec=/usr/bin/true\n")
        env = environment(root)
        (root / "home").mkdir()
        env.update(HOME=str(root / "home"), XDG_DATA_HOME=str(root / "data"), XDG_DATA_DIRS=str(root / "empty-data"))
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
    print("PASS: native installed-app search selected stable desktop identity, retained draft, and applied only explicitly; audio stayed disabled.")


if __name__ == "__main__":
    main()
