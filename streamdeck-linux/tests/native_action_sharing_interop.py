#!/usr/bin/env python3
"""Verify native sharing menus and real host operations; OS picker is not automated."""
import argparse
import io
import json
import subprocess
import tempfile
import zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timeout: '+label)};
const b=window.deckforge,phase=__PHASE__;
const cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot');
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
if(phase==='share'){
 await cmd({command:'assign',controller:'Encoder',index:0,uuid:'com.elgato.streamdeck.system.hotkey'});
 s=await snapshot();const slot=s.slots.find(s=>s.controller==='Encoder'&&s.index===0);
 const source=await b.request('image_crop',{mode:'load',context:slot.context,path:__IMAGE__});
 await b.request('image_crop',{mode:'save',context:slot.context,path:__IMAGE__,source_hash:source.source_hash,expected:null,rect:{x:0,y:0,width:200,height:100}});
 await cmd({command:'slot_copy',context:slot.context});
 const clipboard=(await snapshot()).clipboard;
 (await until(()=>document.querySelectorAll('.deck-dial')[0],'first dial')).click();
 const menu=async name=>{
  (await until(()=>document.querySelector('[aria-label="Edit selected control"]'),'edit menu')).click();
  return until(()=>[...document.querySelectorAll('.control-edit-options button')].find(b=>b.textContent===name&&!b.disabled),name);
 };
 await menu('Export Action…');
 await cmd({command:'slot_export',context:slot.context,path:__EXPORT__});
 document.querySelector('[aria-label="Edit selected control"]').click();
 document.querySelectorAll('.deck-dial')[1].click();await pause(80);
 await menu('Import Action…');
 s=await snapshot();
 await cmd({command:'slot_import',path:__IMPORT__,controller:'Encoder',index:1,expected_target:null,
   expected_profile:s.library.active_profile,expected_page:s.library.active_page});
 s=await snapshot();
 if(JSON.stringify(s.clipboard)!==JSON.stringify(clipboard))throw Error('Import changed clipboard');
 if(s.library.profiles.length!==1)throw Error('Import added a profile');
 const incoming=s.slots.find(v=>v.controller==='Encoder'&&v.index===1);
 if(!incoming||incoming.context===slot.context||incoming.uuid!==slot.uuid)throw Error('Action identity not independent');
 if(!incoming.raw.Encoder?.background)throw Error('Imported artwork missing');

 await pause(50);
 const rect=document.querySelector('.control-edit-options').getBoundingClientRect();
 if(rect.top<0||rect.bottom>innerHeight+1)throw Error('Action menu extends outside window');
 return {phase,menus_available:true,host_import:true,host_export:true,clipboard_preserved:true,independent_context:true,os_picker_automated:false,usb_access:false};
}
const incoming=s.slots.find(v=>v.controller==='Encoder'&&v.index===1);
if(!incoming?.raw.Encoder?.background)throw Error('Saved imported artwork missing after restart');
await cmd({command:'slot_export',context:incoming.context,path:__EXPORT__});
return {phase,reexport_without_original:true,usb_access:false};
'''


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--output', type=Path, default=project/'target/action-sharing/native')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='deckforge-action-sharing-') as directory:
        root = Path(directory)
        source = root/'image.png'
        Image.new('RGB', (200, 100), '#28b48c').save(source)
        archive = root/'action.streamDeckAction'
        second = root/'second.streamDeckAction'
        reports = []
        for phase in ['share', 'restart']:
            script, proof = root/'test.js', root/'proof.json'
            proof.unlink(missing_ok=True)
            text = SCRIPT
            for key, value in {'PHASE': phase, 'IMAGE': str(source),
                               'EXPORT': str(archive if phase == 'share' else second),
                               'IMPORT': str(archive)}.items():
                text = text.replace(f'__{key}__', json.dumps(value))
            script.write_text(text)
            env = environment(root)
            env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),
                       DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),
                       DECKFORGE_CAPTURE=str(args.output.resolve()/(phase+'.png')))
            with (args.output/(phase+'.log')).open('w') as log:
                process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--model', 'plus',
                                            '--state', str(root/'state')], env=env, stdout=log,
                                           stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    code = process.wait(timeout=40)
                    report = result(proof)
                    assert code == 0 and report and report['result']['phase'] == phase, report
                    reports.append(report)
                finally:
                    cleanup(process)
            if phase == 'share':
                archive.unlink()
                source.unlink()
        with zipfile.ZipFile(second) as bundle:
            package = json.loads(bundle.read('package.json'))
            assert package['FormatVersion'] == 1 and package['DeviceModel'] == 'Fake/Storage'
            assert package['OSType'] == 'Linux'
            found = False
            for name in bundle.namelist():
                if not name.endswith('/manifest.json'):
                    continue
                page = json.loads(bundle.read(name))
                for controller in page.get('Controllers', []):
                    for action in controller.get('Actions', {}).values():
                        path = action.get('Encoder', {}).get('background')
                        if path:
                            pixels = Image.open(io.BytesIO(bundle.read(name.removesuffix('manifest.json')+path))).convert('RGBA')
                            assert pixels.size == (200, 100) and pixels.getpixel((10, 10)) == (40, 180, 140, 255)
                            found = True
            assert found
        (args.output/'native.json').write_text(json.dumps({'runs': reports, 'portable_artwork': True,
            'restart_without_source': True, 'os_picker_automated': False, 'usb_access': False}, indent=2)+'\n')
    print('PASS: native sharing menus, host import/export, clipboard preservation, restart and portable artwork; OS picker not automated; no USB.')


if __name__ == '__main__':
    main()
