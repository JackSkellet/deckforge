#!/usr/bin/env python3
"""Drive Forget device in the actual embedded window, with an isolated offline session.

The native host is filtered to a test-owned absent Mini serial. Plus hardware is
excluded by model before opening any HID path. No physical input is injected.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from native_editor_interop import cleanup, environment, result
from native_multi_device_interop import SeedHost
from sample_interop import COUNTER

B = 'deckforge-forget-test-absent-mini'
SCRIPT = r'''
const expected=__EXPECTED__,bridge=window.deckforge;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,message)=>{for(let i=0;i<150;i++){const v=await fn();if(v)return v;await pause(40)}throw Error(message)};
const button=name=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);
const read=()=>bridge.request('snapshot');
const first=await until(async()=>{const s=await read();return s.plugins.length===1&&s.plugins[0].connected&&s},'Counter did not connect');
if(first.simulate||first.connected||first.device!==expected.b)throw Error('Expected offline serial-filtered native host');
const pid=first.plugins[0].pid;
button('Preferences').click();
await until(()=>button('Forget device…'),'Forget button missing');
button('Forget device…').scrollIntoView({block:'center'});
if(button('Forget device…').disabled)throw Error('Offline device cannot be forgotten');
button('Forget device…').click();
await until(()=>button('Cancel')&&document.activeElement===button('Cancel'),'Confirmation must focus Cancel');
if(!document.querySelector('.forget-device [role=alert]').textContent.includes('saved'))throw Error('Deletion scope is not explained');
button('Cancel').click();
await until(()=>document.activeElement===button('Forget device…'),'Cancel did not restore focus');
if((await read()).devices.length!==2)throw Error('Cancel removed a device');
button('Forget device…').click();
await until(()=>button('Forget device and profiles'),'Confirmation missing');
button('Forget device and profiles').click();
const remaining=await until(async()=>{const s=await read();return s.devices.length===1&&s.device===expected.a&&s},'Device remained in picker');
if(remaining.plugins[0].pid!==pid||remaining.slots[0].context!==expected.a_context)throw Error('Other device/plugin changed');
await until(()=>document.activeElement===document.querySelector('[aria-label="Close Preferences"]'),'Successful removal lost dialog focus');
await until(()=>document.querySelector('.device-picker select').options.length===1,'Picker retained forgotten device');
const ui=await bridge.request('ui_state_get');
if(ui?.formDrafts?.['preferences:'+expected.b]||ui?.formDrafts?.['device-name:'+expected.b])throw Error('Forgotten device preferences survived in UI drafts');
await until(()=>button('Forget device…'),'Remaining device Preferences missing');
button('Forget device…').click();
await until(()=>button('Forget device and profiles'),'Last-device confirmation missing');
button('Forget device and profiles').click();
const empty=await until(async()=>{const s=await read();return s.device.startsWith('offline-streamdeck-plus-')&&s.slots.length===0&&s},'Last device was not replaced with an empty workspace');
if(empty.connected||empty.device_online||empty.plugins[0].pid!==pid)throw Error('Empty workspace changed plugin or claims physical connection');
await until(()=>!document.querySelector('.forget-device'),'Empty placeholder exposes device removal');
return {forgotten:[expected.a,expected.b],remaining:empty.device,confirmation_and_cancel:true,picker_updated:true,other_device_preserved:true,last_device_empty:true,plugin_pid:pid,physical_input_injected:false};
'''


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--capture', type=Path, required=True)
    args = parser.parse_args()
    binary, report_path, capture = args.binary.resolve(), args.report.resolve(), args.capture.resolve()
    report_path.parent.mkdir(parents=True, exist_ok=True)
    counter = project.parent/'evidence/official-samples/counter/com.elgato.counter.sdPlugin'
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'boundary': 'Native WebKit with temporary state, absent Mini serial/model filter; no physical controls or user state.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-native-forget-') as directory:
        root = Path(directory)
        state, seed_log, native_log = root/'state', root/'seed.log', root/'native.log'
        host = SeedHost(binary, counter, state, seed_log, environment(root))
        try:
            host.ready()
            initial = host.wait(lambda s: s['plugins'] and s['plugins'][0]['connected'], 'seed plugin')
            a = initial['device']
            host.post({'command': 'device_add', 'device': B, 'model': 'mini'})
            host.post({'command': 'assign', 'device': a, 'controller': 'Keypad', 'index': 0, 'uuid': COUNTER})
            expected = {'a': a, 'b': B, 'a_context': host.state()['slots'][0]['context']}
        finally:
            host.stop()
        script, proof = root/'test.js', root/'proof.json'
        script.write_text(SCRIPT.replace('__EXPECTED__', json.dumps(expected)))
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(capture), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        with native_log.open('w') as log:
            process = subprocess.Popen([str(binary), 'desktop', '--state', str(state), '--port', '0', '--serial', B, '--model', 'mini'], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                assert process.wait(timeout=50) == 0, native_log.read_text()
                report['native'] = result(proof)['result']
            finally:
                cleanup(process)
                report_path.with_suffix('.native.log').write_text(native_log.read_text())
                report_path.with_suffix('.seed.log').write_text(seed_log.read_text())
        saved = json.loads((state/'session.json').read_text())
        assert all(r['device'] not in (a, B) for r in saved['devices'])
        backups = list((state/'forgotten-devices').glob('*.json'))
        assert len(backups) == 2
        inventories = [set(d['device'] for d in json.loads(p.read_text())['devices']) for p in backups]
        assert {a, B} in inventories and {a} in inventories
        report.update(persisted_removal=True, both_recovery_copies_verified=True, capture_sha256=hashlib.sha256(capture.read_bytes()).hexdigest())
    report_path.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: native confirmation/cancel/focus, device picker removal, last-device empty workspace and recovery copies.')


if __name__ == '__main__':
    main()
