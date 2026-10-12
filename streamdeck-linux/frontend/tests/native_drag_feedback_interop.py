#!/usr/bin/env python3
"""Native WebKit drag data and assignment; OS cursor/effect negotiation is manual."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT/'tests'))
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,label)=>{for(let i=0;i<150;i++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+label)};
const state=()=>window.deckforge.request('snapshot');
const initial=await until(async()=>{const s=await state();if(!s.simulate)throw Error('Simulation required');return s.actions?.length&&s},'initial state');
const id='com.elgato.streamdeck.system.website';
const action=initial.actions.find(a=>a.uuid===id);
const source=await until(()=>[...document.querySelectorAll('.library-action')].find(e=>e.querySelector('strong')?.textContent===action.name),'library action');
const keys=()=>document.querySelectorAll('.deck-key');
const event=(element,name,transfer,extra={})=>element.dispatchEvent(new DragEvent(name,{bubbles:true,cancelable:true,dataTransfer:transfer,...extra}));
const transfer=new DataTransfer();
event(source,'dragstart',transfer);
if(!transfer.types.includes('text/plain')||!transfer.getData('text/plain'))throw Error('Native drag has no standard text representation');
if(transfer.getData('application/x-deckforge-action')!==id)throw Error('Assignment payload/effect changed');
if(event(keys()[0],'dragover',transfer))throw Error('Compatible hover was not accepted as copy');
event(keys()[0],'drop',transfer);
const assigned=await until(async()=>(await state()).slots.find(s=>s.controller==='Keypad'&&s.index===0&&s.uuid===id),'real host assignment');
const plain=new DataTransfer();plain.setData('text/plain',id);
event(keys()[1],'drop',plain);
await pause(100);
if((await state()).slots.some(s=>s.controller==='Keypad'&&s.index===1))throw Error('Plain text was treated as a Deckforge action');
await until(()=>keys()[0].draggable,'rendered assigned source');
const move=new DataTransfer();event(keys()[0],'dragstart',move);
if(move.getData('application/x-deckforge-slot')!==assigned.context||!move.getData('text/plain'))throw Error('Slot drag formats/effect wrong');
if(event(keys()[1],'dragover',move))throw Error('Slot move hover was not accepted');
event(keys()[1],'drop',move);
const moved=await until(async()=>(await state()).slots.find(s=>s.uuid===id&&s.index===1),'slot move');
if((await state()).slots.some(s=>s.index===0))throw Error('Move left the source assigned');
await until(()=>keys()[1].draggable&&!keys()[0].draggable,'rendered moved source');
const copied=new DataTransfer();event(keys()[1],'dragstart',copied);
if(event(keys()[2],'dragover',copied,{ctrlKey:true}))throw Error('Ctrl-copy hover wrong');
event(keys()[2],'drop',copied,{ctrlKey:true});
await until(async()=>(await state()).slots.find(s=>s.index===2&&s.uuid===id&&s.context!==moved.context),'slot copy');
event(source,'dragend',transfer);
return{standard_text_representation:true,custom_assignment_payload_retained:true,hover_default_canceled:true,slot_move_and_ctrl_copy:true,plain_text_not_accepted:true,real_host_assignment:true,usb_access:false,cursor_painting_verified:false,synthetic_effect_allowed:transfer.effectAllowed,synthetic_drop_effect:transfer.dropEffect};
'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=PROJECT/'target/release/deckforge')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    binary=args.binary.resolve(strict=True); output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    report={'ok':False,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
            'boundary':'App-owned synthetic DragEvents in native WebKit with real host assignments. OS pointer/cursor painting requires human verification.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-native-drag-') as directory:
        root=Path(directory);script=root/'probe.js';script.write_text(SCRIPT);proof=root/'result.json'
        env=environment(root);env.update(LC_ALL='C.UTF-8',DECKFORGE_CAPTURE=str(output/'drag.png'),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path=output/'drag.log'
        with log_path.open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=35)==0,log_path.read_text()
                value=result(proof);assert value,log_path.read_text()
                report.update(ok=True,native=value['result'])
            finally:
                cleanup(process);(output/'drag.json').write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: native drag formats, event acceptance, real copy/move assignments and plain-text rejection. Physical cursor appearance not automated.')


if __name__=='__main__':main()
