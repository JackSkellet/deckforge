#!/usr/bin/env python3
"""Exercise the embedded TypeScript editor through its own native WebKit bridge.

No browser/computer automation, USB, or real user state. The Counter fixture is
Elgato's unchanged official sample. Run window_interop.py separately for the SDK
heartbeat/process-ownership matrix across window generations.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time

from sample_interop import SAMPLE_COMMIT, COUNTER_LOCK_SHA256

COUNTER_SCRIPT = r'''
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async fn=>{for(let i=0;i<150;i++){const result=await fn();if(result)return result;await pause(50)}throw Error('Native fixture condition timed out')};
const bridge=window.deckforge;
await until(async()=>{const s=await bridge.request('snapshot');if(s.simulate!==true)throw Error('Fixture requires simulation');return s.plugins.some(p=>p.id==='com.elgato.counter'&&p.connected)});
await bridge.request('command',{command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.counter.action'});
const counter=()=>bridge.request('snapshot').then(s=>s.slots.find(slot=>slot.controller==='Keypad'&&slot.index===0&&slot.uuid==='com.elgato.counter.action'));
let slot=await until(async()=>{const slot=await counter();return slot?.title==='0'&&slot});
await bridge.request('command',{command:'settings',context:slot.context,settings:{count:0,incrementBy:7}});
await bridge.request('command',{command:'input',event:'keyDown',index:0});
await bridge.request('command',{command:'input',event:'keyUp',index:0});
slot=await until(async()=>{const slot=await counter();return slot?.title==='7'&&slot.settings.count===7&&slot.settings.incrementBy===7&&slot});
const image=await until(()=>{const key=document.querySelector('.deck-key');const image=key?.querySelector('img');return key?.querySelector('.sr-only')?.textContent==='7'&&image?.complete&&image.naturalWidth>0&&image});
let unsafeUrlRejected=false,unknownMethodRejected=false;
try{await bridge.request('open_url',{url:'javascript:alert(1)'})}catch{unsafeUrlRejected=true}
try{await bridge.request('unknown_fixture_method')}catch{unknownMethodRejected=true}
if(!unsafeUrlRejected||!unknownMethodRejected)throw Error('Bridge boundary rejected neither unsafe URL nor unknown method');
return {counter:{uuid:slot.uuid,title:slot.title,settings:slot.settings},dom:{keyLabel:document.querySelector('.deck-key').getAttribute('aria-label'),renderedTitle:document.querySelector('.deck-key .sr-only').textContent,imageWidth:image.naturalWidth,imageHeight:image.naturalHeight,title:document.title},unsafeUrlRejected,unknownMethodRejected};
'''

DRAFT_SCRIPT = r'''
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async fn=>{for(let i=0;i<100;i++){const value=await fn();if(value)return value;await pause(50)}throw Error('Draft fixture condition timed out')};
const title='Unsaved across native close';const bridge=window.deckforge;
const state=await bridge.request('ui_state_get');
const previous=Object.values(state?.drafts||{}).find(d=>d.title===title&&d.dirty);
const field=()=>[...document.querySelectorAll('label.field')].find(label=>label.querySelector('span')?.textContent==='Title')?.querySelector('input');
if(previous){const input=await until(()=>field());if(input.value!==title)throw Error('React did not restore the retained Rust draft');return {phase:'restored',title:input.value,dirty:true};}
await bridge.request('command',{command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.page.next'});
const input=await until(()=>field());
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,title);
input.dispatchEvent(new Event('input',{bubbles:true}));input.dispatchEvent(new Event('change',{bubbles:true}));
await until(async()=>Object.values((await bridge.request('ui_state_get'))?.drafts||{}).some(d=>d.title===title&&d.dirty));
return {phase:'edited',title:input.value,dirty:true};
'''


MIXER_SCRIPT = r"""
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async fn=>{for(let i=0;i<150;i++){const value=await fn();if(value)return value;await pause(50)}throw Error('Mixer fixture condition timed out')};
const bridge=window.deckforge;const draftCase=__DRAFT_CASE__;
const config={enabled:false,channels:[{id:'music',name:'Music',apps:[],sources:[]},{id:'voice',name:'Voice',apps:[],sources:[]}],mixes:[{id:'monitor',name:'Monitor',outputs:[],volume:85,muted:false},{id:'stream',name:'Stream',outputs:[],volume:100,muted:false}],routes:[{channel:'music',mix:'monitor',volume:55,muted:false},{channel:'music',mix:'stream',volume:35,muted:true},{channel:'voice',mix:'monitor',volume:75,muted:false},{channel:'voice',mix:'stream',volume:90,muted:false}]};
const route=(config,channel,mix)=>config.routes.find(r=>r.channel===channel&&r.mix===mix);
const gain=(channel,mix)=>document.querySelector(`input[aria-label="${channel} → ${mix} volume"]`);
const state=await bridge.request('ui_state_get');
const retained=state?.formDrafts?.['audio-mixer'];
if(draftCase&&retained?.dirty&&route(retained.config,'voice','monitor')?.volume===78){
 const input=await until(()=>gain('Voice','Monitor'));
 if(input.value!=='78')throw Error('Mixer draft was not restored into native DOM');
 const host=await bridge.request('snapshot');
 if(host.mixer.config.enabled||route(host.mixer.config,'voice','monitor').volume!==75)throw Error('Unapplied mixer draft changed saved routing');
 return {phase:'restored',draftVolume:78,savedVolume:75,enabled:false};
}
const before=await bridge.request('snapshot');if(before.simulate!==true)throw Error('Mixer fixture must be simulated');
await bridge.request('command',{command:'mixer_config',settings:config,expected:before.mixer.config});
if(!document.querySelector('.audio-mixer'))[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Audio mixer').click();
const input=await until(()=>gain('Music','Monitor'));
if(input.value!=='55')throw Error('Routing matrix did not load bridge config');
const enabled=[...document.querySelectorAll('.audio-mixer label')].find(label=>label.textContent.includes('Enable audio routing'))?.querySelector('input');
if(!enabled||enabled.checked)throw Error('Routing must remain disabled');
const set=(input,value)=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,String(value));input.dispatchEvent(new Event('input',{bubbles:true}));input.dispatchEvent(new Event('change',{bubbles:true}));};
set(input,62);
await until(async()=>{const state=await bridge.request('ui_state_get');return route(state?.formDrafts?.['audio-mixer']?.config||{routes:[]},'music','monitor')?.volume===62});
const unapplied=await bridge.request('snapshot');if(route(unapplied.mixer.config,'music','monitor').volume!==55)throw Error('Mixer changed without explicit Apply');
const apply=document.querySelector('.audio-mixer button[type="submit"]');await until(()=>!apply.disabled);apply.click();
const applied=await until(async()=>{const s=await bridge.request('snapshot');return route(s.mixer.config,'music','monitor')?.volume===62&&s});
if(applied.mixer.config.enabled||applied.mixer.active||applied.mixer.sinks.some(s=>s.owned))throw Error('Disabled fixture created an audio graph');
if(applied.mixer.config.channels.some(c=>c.apps.length||c.sources.length)||applied.mixer.config.mixes.some(m=>m.outputs.length))throw Error('Fixture selected real audio members');
await until(()=>document.querySelector('.mixer-footer [role="status"]')?.textContent.includes('Routing settings saved'));
const applyRect=document.querySelector('.audio-mixer button[type="submit"]').getBoundingClientRect();
const dialogRect=document.querySelector('dialog[open]').getBoundingClientRect();
if(applyRect.bottom>dialogRect.bottom+1||applyRect.top<dialogRect.top)throw Error('Mixer Apply button is clipped outside its dialog');
if(draftCase){
 set(gain('Voice','Monitor'),78);
 await until(async()=>{const d=(await bridge.request('ui_state_get'))?.formDrafts?.['audio-mixer'];return d?.dirty&&route(d.config,'voice','monitor')?.volume===78});
 return {phase:'edited',draftVolume:78,savedVolume:75,enabled:false};
}
return {phase:'applied',channels:[...document.querySelectorAll('.mixer-matrix tbody th strong')].map(n=>n.textContent),savedMusicMonitor:62,otherRoutes:applied.mixer.config.routes,enabled:false,active:false,ownedAudioNodes:0,explicitApplyRequired:true,applyButtonBottom:applyRect.bottom,dialogBottom:dialogRect.bottom};
"""


def wait(predicate, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError('Timed out: ' + description)


def result(path, phase=None):
    if not path.exists():
        return None
    try:
        data = json.loads(path.read_text())
    except json.JSONDecodeError:
        return None
    assert data['ok'], data
    return data if phase is None or data['result']['phase'] == phase else None


def command(binary, state, name, required=True):
    call = subprocess.run([str(binary), 'window', name, '--state', str(state)],
                          capture_output=True, text=True, timeout=4)
    if not required and call.returncode:
        return None
    assert call.returncode == 0, call.stderr
    return json.loads(call.stdout)


def cleanup(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def environment(root):
    env = os.environ.copy()
    for key in ('DECKFORGE_CAPTURE', 'DECKFORGE_WEBVIEW_TEST_SCRIPT',
                'DECKFORGE_WEBVIEW_TEST_RESULT'):
        env.pop(key, None)
    env['XDG_CONFIG_HOME'] = str(root / 'config')
    env['XDG_STATE_HOME'] = str(root / 'private-state')
    return env


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/release/deckforge')
    parser.add_argument('--samples', type=Path, default=project.parent / 'evidence/official-samples')
    parser.add_argument('--capture', type=Path, help='Keep the actual native WebKit PNG')
    parser.add_argument('--report', type=Path, help='Save fixture and verification metadata')
    parser.add_argument('--mixer-capture', type=Path, help='Keep the disabled native mixer matrix PNG')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    samples = args.samples.resolve(strict=True)
    counter = samples / 'counter/com.elgato.counter.sdPlugin'
    assert (counter / 'bin/plugin.js').is_file(), 'Build the unchanged official Counter sample first'
    assert hashlib.sha256((samples / 'counter/package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    if (samples / '.git').exists():
        assert subprocess.check_output(['git', '-C', str(samples), 'rev-parse', 'HEAD'], text=True).strip() == SAMPLE_COMMIT
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'sample_commit': SAMPLE_COMMIT,
              'counter_lock_sha256': COUNTER_LOCK_SHA256,
              'counter_bundle_sha256': hashlib.sha256((counter / 'bin/plugin.js').read_bytes()).hexdigest(),
              'boundary': 'App-owned native WebKit JavaScript fixtures; simulation and temporary session/config only; no OS input/browser automation/USB'}
    with tempfile.TemporaryDirectory(prefix='deckforge-native-editor-') as temporary:
        root = Path(temporary)
        capture = args.capture.resolve() if args.capture else root / 'counter.png'
        capture.parent.mkdir(parents=True, exist_ok=True)
        script = root / 'counter.js'
        script.write_text(COUNTER_SCRIPT)
        proof = root / 'counter-result.json'
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(capture), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),
                   DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = root / 'counter.log'
        with log_path.open('w') as log:
            process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--state', str(root / 'counter-state'),
                                        '--port', '0', '--plugin', str(counter)], env=env,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=35) == 0, log_path.read_text()
                report['counter'] = result(proof)
                assert report['counter'], log_path.read_text()
                png = capture.read_bytes()
                assert png[:8] == b'\x89PNG\r\n\x1a\n'
                width, height = struct.unpack('>II', png[16:24])
                assert width >= 760 and height >= 620
                report['capture'] = {'width': width, 'height': height, 'sha256': hashlib.sha256(png).hexdigest()}
                saved = json.loads((root / 'counter-state/session.json').read_text())
                slot = next(s for s in saved['slots'] if s['uuid'] == 'com.elgato.counter.action')
                assert slot['settings']['count'] == 7 and slot['settings']['incrementBy'] == 7
            finally:
                cleanup(process)

        state = root / 'draft-state'
        script = root / 'draft.js'
        script.write_text(DRAFT_SCRIPT)
        proof = root / 'draft-result.json'
        env = environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = root / 'draft.log'
        with log_path.open('w') as log:
            process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--state', str(state), '--port', '0'],
                                       env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                first = wait(lambda: result(proof, 'edited'), 'draft edit')
                command(binary, state, 'close')
                def closed():
                    status = command(binary, state, 'status')
                    return status if not status['window_open'] else None
                before = wait(closed, 'native window destroyed')
                command(binary, state, 'show')
                second = wait(lambda: result(proof, 'restored'), 'draft restored in recreated WebKit')
                after = command(binary, state, 'status')
                assert before['pid'] == after['pid'] == process.pid
                assert before['editor_generation'] == 1 and after['editor_generation'] == 2
                assert after['window_open']
                command(binary, state, 'quit')
                assert process.wait(timeout=10) == 0, log_path.read_text()
                report['draft'] = {'first': first, 'closed': before, 'reopened': after, 'second': second}
            except Exception:
                print(log_path.read_text())
                raise
            finally:
                cleanup(process)

        # Mixer UI checks keep enabled=false and select no actual app/device.
        # Opening Audio mixer may read the audio inventory; it creates no graph.
        for retain_draft in (False, True):
            suffix = 'mixer-draft' if retain_draft else 'mixer-capture'
            state = root / suffix
            script = root / (suffix + '.js')
            script.write_text(MIXER_SCRIPT.replace('__DRAFT_CASE__', 'true' if retain_draft else 'false'))
            proof = root / (suffix + '-result.json')
            env = environment(root)
            env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
            mixer_capture = args.mixer_capture.resolve() if args.mixer_capture else root / 'mixer.png'
            if not retain_draft:
                mixer_capture.parent.mkdir(parents=True, exist_ok=True)
                env['DECKFORGE_CAPTURE'] = str(mixer_capture)
            log_path = root / (suffix + '.log')
            with log_path.open('w') as log:
                process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--state', str(state), '--port', '0'],
                                           env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    if retain_draft:
                        first = wait(lambda: result(proof, 'edited'), 'unapplied mixer draft')
                        command(binary, state, 'close')
                        def mixer_closed():
                            status = command(binary, state, 'status')
                            return status if not status['window_open'] else None
                        before = wait(mixer_closed, 'mixer native window destruction')
                        command(binary, state, 'show')
                        second = wait(lambda: result(proof, 'restored'), 'mixer draft restored after reopen')
                        after = command(binary, state, 'status')
                        assert before['pid'] == after['pid'] == process.pid
                        assert after['editor_generation'] == 2
                        command(binary, state, 'quit')
                        assert process.wait(timeout=10) == 0, log_path.read_text()
                        report['mixer_draft'] = {'edited': first, 'restored': second, 'closed': before, 'reopened': after}
                    else:
                        assert process.wait(timeout=35) == 0, log_path.read_text()
                        report['mixer'] = result(proof)
                        assert report['mixer'], log_path.read_text()
                        png = mixer_capture.read_bytes()
                        assert png[:8] == b'\x89PNG\r\n\x1a\n'
                        width, height = struct.unpack('>II', png[16:24])
                        report['mixer_capture'] = {'width': width, 'height': height, 'sha256': hashlib.sha256(png).hexdigest()}
                    saved = json.loads((state / 'session.json').read_text())['mixer']
                    assert not saved['enabled']
                    assert next(r for r in saved['routes'] if r['channel'] == 'music' and r['mix'] == 'monitor')['volume'] == 62
                    assert next(r for r in saved['routes'] if r['channel'] == 'voice' and r['mix'] == 'monitor')['volume'] == 75
                    assert all(not c['apps'] and not c['sources'] for c in saved['channels'])
                    assert all(not m['outputs'] for m in saved['mixes'])
                except Exception:
                    print(log_path.read_text())
                    raise
                finally:
                    cleanup(process)

        state = root / 'headless-state'
        env = environment(root)
        env.pop('DISPLAY', None)
        env.pop('WAYLAND_DISPLAY', None)
        with (root / 'headless.log').open('w') as log:
            process = subprocess.Popen([str(binary), 'desktop', '--background', '--simulate', '--state', str(state), '--port', '0'],
                                       env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                status = wait(lambda: command(binary, state, 'status', False), 'headless background IPC')
                assert not status['window_open'] and status['editor_generation'] == 0
                command(binary, state, 'quit')
                assert process.wait(timeout=10) == 0
                report['headless'] = status
            finally:
                cleanup(process)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + '\n')
    print('PASS: actual native WebKit + unchanged official Counter count7/settings persistence; rendered preview; bridge rejection; unsaved draft survives destruction/recreation; headless background exits cleanly; disabled mixer matrix edits require Apply and drafts survive reopen. No USB, OS input, or audio routing enabled.')


if __name__ == '__main__':
    main()
