#!/usr/bin/env python3
"""Capture the real embedded WebKit model canvases against the Rust host; no USB."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from native_editor_interop import environment, result, cleanup
from models_interop import MODELS

SCRIPT = r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async fn=>{for(let i=0;i<120;i++){const value=await fn();if(value)return value;await pause(50)}throw Error('Model canvas timed out')};
const bridge=window.deckforge, expected=__EXPECTED__;
const initial=await bridge.request('snapshot');
if(initial.simulate!==true||initial.device_model!==expected.model)throw Error('Wrong test device');
const keys=()=>[...document.querySelectorAll('.deck-key')];
const dials=()=>[...document.querySelectorAll('.deck-dial')];
await until(()=>keys().length===expected.keys&&dials().length===expected.dials);
const last=expected.keys-1;
await bridge.request('command',{command:'assign',controller:'Keypad',index:last,uuid:'com.elgato.streamdeck.page.next'});
await until(()=>keys()[last].getAttribute('aria-label')?.includes('Next'));
keys()[last].click();
await until(async()=>{const s=await bridge.request('ui_state_get');return s?.selected?.index===last&&s?.selected?.controller==='Keypad'});
const rects=keys().map(n=>{const r=n.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}});
if(rects.some(r=>r.width<18||r.height<18||r.x<0||r.right>innerWidth+1||r.y<0||r.bottom>innerHeight+1))throw Error('Key canvas clipped: '+JSON.stringify({viewport:[innerWidth,innerHeight],rects}));
const stage=document.querySelector('.device-stage');
const columns=getComputedStyle(document.querySelector('.key-grid')).gridTemplateColumns.split(' ').length;
if(expected.model!=='pedal'&&columns!==expected.columns)throw Error('Wrong key grid columns: '+columns);
if(expected.model==='pedal'&&stage.dataset.keyImages!=='false')throw Error('Pedal incorrectly advertises a display');
let galleonGeometry=false;
if(expected.model==='galleon'){
 const grid=document.querySelector('.dial-grid');
 if(getComputedStyle(grid).gridTemplateColumns.split(' ').length!==2||document.querySelectorAll('.galleon-dials > span').length!==2||document.querySelectorAll('.dial-knob').length)throw Error('Galleon logical and physical dials conflated');
 const rects=dials().map(d=>d.getBoundingClientRect());
 if(rects[0].y!==rects[1].y||rects[2].y!==rects[3].y||rects[2].y<=rects[0].y||rects[3].bottom>keys()[0].getBoundingClientRect().top)throw Error('Galleon widgets must be two rows above the keys');
 await bridge.request('command',{command:'assign',controller:'Encoder',index:3,uuid:'com.elgato.streamdeck.system.hotkey'});
 await until(()=>dials()[3].getAttribute('aria-label')?.includes('Hotkey'));
 dials()[3].click();
 await until(async()=>{const s=await bridge.request('ui_state_get');return s?.selected?.index===3&&s?.selected?.controller==='Encoder'});
 const image=dials()[3].querySelector('img');await until(()=>image?.complete&&image.naturalWidth===320&&image.naturalHeight===160);
 if(!initial.device_capabilities.transport_supported||initial.device_capabilities.touchscreen)throw Error('Galleon transport/touch capabilities incorrect');
 keys()[last].click();galleonGeometry=true;
}
let neoNavigation=false;
if(expected.model==='neo'){
 const left=()=>document.querySelector('.neo-sensor[aria-label="Previous page"]');
 const right=()=>document.querySelector('.neo-sensor[aria-label="Next page"]');
 await until(()=>left()?.disabled&&right()?.disabled);
 await bridge.request('command',{command:'page_create',name:'Second page'});
 const second=(await bridge.request('snapshot')).library.active_page;
 await until(()=>!left()?.disabled&&right()?.disabled);
 left().click();
 await until(async()=>{const s=await bridge.request('snapshot');return s.library.active_page!==second&&left()?.disabled&&!right()?.disabled});
 right().click();
 await until(async()=>{const s=await bridge.request('snapshot');return s.library.active_page===second&&!left()?.disabled&&right()?.disabled});
 left().click();await until(()=>left()?.disabled&&!right()?.disabled);neoNavigation=true;
}
return {model:initial.device_model,device_name:initial.device_name,keys:keys().length,dials:dials().length,columns,last_key_selected:true,neo_navigation_verified:neoNavigation,galleon_geometry_verified:galleonGeometry,viewport:[innerWidth,innerHeight],key_bounds:rects,usb_access:false};
'''

def main():
    root_project=Path(__file__).resolve().parents[1]
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=root_project/'target/release/deckforge')
    ap.add_argument('--output',type=Path,required=True)
    ap.add_argument('--models',nargs='+',default=list(MODELS))
    args=ap.parse_args();binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-native-models-') as directory:
        root=Path(directory)
        for model in args.models:
            columns,rows,dials,_,_=MODELS[model]
            area=root/model;area.mkdir();proof=area/'proof.json';script=area/'fixture.js'
            script.write_text(SCRIPT.replace('__EXPECTED__',json.dumps({'model':model,'keys':columns*rows,'columns':columns,'dials':dials})))
            capture=output/(model+'.png');env=environment(area)
            env.update(DECKFORGE_CAPTURE=str(capture),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
            log_path=output/(model+'.log')
            with log_path.open('w') as log:
                process=subprocess.Popen([str(binary),'desktop','--simulate','--model',model,'--state',str(area/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    assert process.wait(timeout=35)==0,log_path.read_text()
                    report=result(proof);assert report,log_path.read_text()
                    report['capture_sha256']=hashlib.sha256(capture.read_bytes()).hexdigest();reports.append(report)
                finally:cleanup(process)
    (output/'native.json').write_text(json.dumps({'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'models':reports},indent=2)+'\n')
    print('PASS: native model canvases, correct key/dial counts, final-key selection and unclipped controls; no USB.')

if __name__=='__main__':main()
