#!/usr/bin/env python3
"""Exercise action Copy/Paste through React in the native desktop window.

The app-owned WebKit fixture uses an isolated simulated session and official
Counter sample. It does not inject OS input or access the physical Stream Deck.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from native_editor_interop import environment, cleanup, result
from sample_interop import COUNTER_LOCK_SHA256

SCRIPT = r'''
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async fn=>{for(let i=0;i<150;i++){const found=await fn();if(found)return found;await pause(40)}throw Error('Native transfer condition timed out')};
const bridge=window.deckforge;
await until(async()=>{const s=await bridge.request('snapshot');if(!s.simulate)throw Error('Simulation required');return s.plugins.some(p=>p.id==='com.elgato.counter'&&p.connected)});
await bridge.request('command',{command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.counter.action'});
const source=await until(async()=>{const s=await bridge.request('snapshot');return s.slots.find(s=>s.controller==='Keypad'&&s.index===0&&s.title==='0')});
await bridge.request('command',{command:'settings',context:source.context,settings:{count:4,incrementBy:3}});
await bridge.request('command',{command:'input',event:'keyDown',index:0});
await bridge.request('command',{command:'input',event:'keyUp',index:0});
await until(async()=>{const s=await bridge.request('snapshot');return s.slots.some(slot=>slot.context===source.context&&slot.title==='7')});
await until(()=>document.querySelector('[aria-label="Key 1: Count"]'));
const edit=()=>document.querySelector('[aria-label="Edit selected control"]');
const menuButton=label=>[...document.querySelectorAll('.control-edit-options button')].find(button=>button.textContent.trim().startsWith(label));
await until(()=>edit());edit().click();
const copy=await until(()=>menuButton('Copy'));copy.click();
await until(async()=>{const s=await bridge.request('snapshot');return s.clipboard?.context===source.context});
const destination=await until(()=>document.querySelector('[aria-label="Key 2: Unassigned"]'));destination.click();
await until(()=>document.body.innerText.includes('KEY 2'));edit().click();
const paste=await until(()=>menuButton('Paste'));if(paste.disabled)throw Error('Compatible paste disabled');paste.click();
const copied=await until(async()=>{const s=await bridge.request('snapshot');return s.slots.find(s=>s.controller==='Keypad'&&s.index===1&&s.settings.count===7)});
if(copied.context===source.context)throw Error('Copy reused runtime identity');
await until(()=>document.querySelector('[aria-label="Key 2: Count"] img')?.complete);
edit().click();await until(()=>document.body.innerText.includes('Copied: Count'));
return {source:source.context,copy:copied.context,settings:copied.settings,copyAndPasteThroughReact:true,menuVisible:true,simulated:true};
'''


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--samples', type=Path, default=project.parent/'evidence/official-samples')
    parser.add_argument('--capture', type=Path)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    counter = args.samples.resolve()/'counter/com.elgato.counter.sdPlugin'
    assert hashlib.sha256((counter.parent/'package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    with tempfile.TemporaryDirectory(prefix='deckforge-native-transfer-') as temporary:
        root = Path(temporary)
        script = root/'fixture.js'
        script.write_text(SCRIPT)
        proof = root/'result.json'
        capture = args.capture.resolve() if args.capture else root/'native.png'
        capture.parent.mkdir(parents=True, exist_ok=True)
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(capture), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),
                   DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = root/'native.log'
        with log_path.open('w') as log:
            process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--state', str(root/'state'),
                                        '--port', '0', '--plugin', str(counter)], env=env,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=35) == 0, log_path.read_text()
                report = result(proof)
                assert report, log_path.read_text()
                saved = json.loads((root/'state/session.json').read_text())
                assert len(saved['slots']) == 2
                assert all(s['settings']['count'] == 7 and s['settings']['incrementBy'] == 3 for s in saved['slots'])
                assert 'clipboard' not in saved
            finally:
                cleanup(process)
        report.update(binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                      capture_sha256=hashlib.sha256(capture.read_bytes()).hexdigest(),
                      boundary='Actual native WebKit/React and official Counter, isolated simulation; no OS input or USB.')
        if args.report:
            args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: native React Copy/Paste, distinct plugin contexts, saved settings and rendered clipboard menu; simulation only.')


if __name__ == '__main__':
    main()
