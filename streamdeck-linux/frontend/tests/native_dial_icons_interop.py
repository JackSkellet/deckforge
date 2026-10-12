#!/usr/bin/env python3
"""Native Plus/Studio software dial icons with a generated WebSocket plugin.

Temporary state and simulated hardware only. The software icon and the encoder
LCD are distinct snapshot/DOM targets; no physical input, audio, or USB access.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / 'tests'))
from feedback_interop import make_plugin, PRIMARY
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const bridge=window.deckforge, model=__MODEL__;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<160;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
let snapshot=await bridge.request('snapshot');if(!snapshot.simulate||snapshot.device_model!==model)throw Error('Wrong simulation');
const command=payload=>bridge.request('command',{...payload,device:snapshot.device});
await until(async()=>(await bridge.request('snapshot')).plugins.some(p=>p.id===__PLUGIN__&&p.connected),'plugin registration');
await command({command:'assign',controller:'Encoder',index:0,uuid:__ACTION__});
const slot=async()=>(await bridge.request('snapshot')).slots.find(slot=>slot.controller==='Encoder'&&slot.index===0);
const dial=()=>document.querySelector('.deck-dial'), icon=()=>dial()?.querySelector('.dial-knob img.dial-software-icon');
const initial=await until(async()=>{const current=await slot();return current?.software_icon&&current},'manifest encoder icon');
await until(()=>icon()?.complete&&icon().naturalWidth>0,'native static icon decoding');dial().click();
const lcd=()=>dial()?.querySelector(':scope > img');
if(model==='studio'&&lcd())throw Error('Screenless dial has fake LCD');
if(model==='plus'&&!lcd())throw Error('Plus LCD missing');
const initialLcd=initial.preview, initialLcdSrc=lcd()?.getAttribute('src')??null;
let seq=0;
const send=async commands=>{const current=await slot();await command({command:'settings',context:current.context,settings:{...current.settings,feedbackProbe:{nonce:'native-dial-'+(++seq),commands}}});};
await send([{event:'setImage',payload:{image:'replacement.png'}}]);
const updated=await until(async()=>{const current=await slot();return current.software_icon!==initial.software_icon&&current},'software-only setImage');
await until(()=>icon()?.getAttribute('src')==='data:image/png;base64,'+updated.software_icon&&icon().complete&&icon().naturalWidth>0,'changed native software icon');
if(updated.preview!==initialLcd||(lcd()?.getAttribute('src')??null)!==initialLcdSrc)throw Error('setImage altered encoder LCD');
let feedbackIndependent=false;
if(model==='plus'){
 await send([{event:'setFeedback',payload:{title:'LCD feedback',value:72}}]);
 const feedback=await until(async()=>{const current=await slot();return current.preview!==initialLcd&&current},'separate LCD feedback');
 if(feedback.software_icon!==updated.software_icon)throw Error('LCD feedback changed software icon');
 await until(()=>lcd()?.getAttribute('src')==='data:image/png;base64,'+feedback.preview,'native LCD update');feedbackIndependent=true;
}
const rect=icon().getBoundingClientRect(), knob=dial().querySelector('.dial-knob').getBoundingClientRect();
if(rect.width<10||rect.height<10||rect.left<knob.left||rect.right>knob.right+1||rect.top<knob.top||rect.bottom>knob.bottom+1)throw Error('Software icon outside knob');
return{model,manifest_fallback_decoded:true,setImage_software_only:true,setFeedback_lcd_only:feedbackIndependent,screenless:model==='studio',icon_dimensions:[icon().naturalWidth,icon().naturalHeight],knob_bounds:{width:knob.width,height:knob.height},usb_access:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'ok': False, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'models': []}
    with tempfile.TemporaryDirectory(prefix='deckforge-native-dial-icons-') as temporary:
        root = Path(temporary)
        for model in ('plus', 'studio'):
            area = root / model
            area.mkdir()
            plugin, _ = make_plugin(area / (PRIMARY + '.sdPlugin'), PRIMARY, PROJECT.parent / 'evidence/official-samples/counter/package.json')
            manifest = json.loads((plugin / 'manifest.json').read_text())
            manifest['Actions'][1]['Encoder']['Icon'] = 'base.png'
            (plugin / 'manifest.json').write_text(json.dumps(manifest))
            script, proof = area / 'fixture.js', area / 'proof.json'
            script.write_text(SCRIPT.replace('__MODEL__', json.dumps(model)).replace('__PLUGIN__', json.dumps(PRIMARY)).replace('__ACTION__', json.dumps(PRIMARY + '.dial')))
            env = environment(area)
            env.update(DECKFORGE_CAPTURE=str(output / (model + '.png')), DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
            log_path = output / (model + '.log')
            with log_path.open('w') as log:
                process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--model', model, '--plugin', str(plugin), '--state', str(area / 'state')], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    assert process.wait(timeout=35) == 0, log_path.read_text()
                    value = result(proof)
                    assert value, log_path.read_text()
                    report['models'].append(value['result'])
                except Exception as error:
                    report['error'] = str(error)
                    raise
                finally:
                    cleanup(process)
                    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    report['ok'] = True
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print('PASS: native Plus/Studio software icons decode and update independently of LCD feedback; no USB or physical input.')


if __name__ == '__main__':
    main()
