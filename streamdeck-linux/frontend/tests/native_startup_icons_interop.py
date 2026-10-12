#!/usr/bin/env python3
"""Load 160 generated action icons in the native editor, with isolated state."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / 'tests'))
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
let count=0;
for(let n=0;n<300;n++){
 const error=document.querySelector('.banner.error');
 if(error)throw Error(error.textContent);
 const rows=[...document.querySelectorAll('.library-action')].filter(row=>row.textContent.includes('Startup icon'));
 count=rows.filter(row=>{const img=row.querySelector('img.action-icon');return img?.complete&&img.naturalWidth>0}).length;
 if(count===160)break;
 await pause(50);
}
if(count!==160)throw Error('Only '+count+'/160 library icons loaded');
const before=await window.deckforge.request('ui_state_get');
await window.deckforge.request('ui_state_set',{state:{...before,startupProbe:'retained'}});
if((await window.deckforge.request('ui_state_get')).startupProbe!=='retained')throw Error('Draft state did not round-trip');
return {icons_loaded:count,startup_error:false,draft_roundtrip:true,usb_access:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-startup-icons-') as temporary:
        root = Path(temporary)
        plugin = root / 'org.deckforge.startupprobe.sdPlugin'
        plugin.mkdir()
        # No plugin runtime or private user plugins needed for library icon reads.
        (plugin / 'plugin.js').write_text('setInterval(()=>{},1000);')
        (plugin / 'icon.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144"><rect width="144" height="144" fill="#80cdb5"/></svg>')
        (plugin / 'manifest.json').write_text(json.dumps({
            'UUID': 'org.deckforge.startupprobe', 'Name': 'Startup icons',
            'CodePath': 'plugin.js', 'SDKVersion': 2, 'Nodejs': {'Version': '20'},
            'Actions': [{'UUID': 'org.deckforge.startupprobe.key'+str(i),
                         'Name': 'Startup icon '+str(i), 'Icon': 'icon',
                         'Controllers': ['Keypad'], 'States': [{'Image': 'icon'}]}
                        for i in range(160)]}))
        script, proof = root / 'probe.js', root / 'proof.json'
        script.write_text(SCRIPT)
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(output / "startup.png"), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        with (output / 'native.log').open('w') as log:
            process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--plugin', str(plugin), '--state', str(root / 'state')], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=35)
                if proof.exists():
                    (output / 'result.json').write_text(proof.read_text())
                assert code == 0, (output / 'native.log').read_text()
                value = result(proof)
                assert value
                value['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
                (output / 'result.json').write_text(json.dumps(value, indent=2)+'\n')
            finally:
                cleanup(process)
    print('PASS: 160 native startup icons, no error banner, draft state round-trip.')


if __name__ == '__main__':
    main()
