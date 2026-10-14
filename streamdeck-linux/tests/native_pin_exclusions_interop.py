#!/usr/bin/env python3
"""Exercise pin exclusions through the native editor with private simulated state."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

SCRIPT = r'''
const bridge=window.deckforge, command=payload=>bridge.request('command',payload), snapshot=()=>bridge.request('snapshot');
const until=async(fn,label)=>{for(let i=0;i<150;i++){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,40));}throw Error('Timed out: '+label);};
const initial=await until(async()=>{const s=await snapshot();if(!s.simulate)throw Error('USB simulation required');return s.library?.active_page&&s;},'initial page');
const first=initial.library.active_page;
await command({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.page.next'});
const pin=await until(async()=>(await snapshot()).slots.find(s=>s.index===0&&s.controller==='Keypad'),'assigned action');
await command({command:'slot_pin',context:pin.context,pinned:true});
await command({command:'page_create',name:'Middle'});const middle=(await snapshot()).library.active_page;
await command({command:'page_create',name:'Last'});const last=(await snapshot()).library.active_page;
await command({command:'page_select',id:middle});
await command({command:'slot_pin_exclusions',context:pin.context,expected:[],excluded_pages:[first,last]});
for(const [page,visible] of [[first,false],[middle,true],[last,false]]){
 await command({command:'page_select',id:page});
 if((await snapshot()).slots.some(s=>s.context===pin.context)!==visible)throw Error('Incorrect pin visibility');
}
const state=await snapshot(), profile=state.library.profiles.find(p=>p.id===state.library.active_profile);
await command({command:'page_select',id:profile.pin_page});
await until(()=>document.querySelector('.deck-key'),'key');document.querySelector('.deck-key').click();
const summary=await until(()=>[...document.querySelectorAll('summary')].find(s=>s.textContent.includes('Exclude pages')),'exclusion editor');summary.click();
const checkbox=await until(()=>[...document.querySelectorAll('label')].find(l=>l.textContent.includes('Page 2: Middle'))?.querySelector('input[type=checkbox]'),'middle page checkbox');
if(checkbox.checked)throw Error('Middle was unexpectedly excluded');checkbox.click();
await until(async()=>(await snapshot()).slots.find(s=>s.context===pin.context)?.pin_excluded_pages.length===3,'saved exclusions');
await until(()=>checkbox.checked,'checkbox updated');checkbox.click();
await until(async()=>(await snapshot()).slots.find(s=>s.context===pin.context)?.pin_excluded_pages.length===2,'middle page restored');
return {first_last_excluded:true,middle_preserved:true,native_checkbox_saved:true,pinned_editor_accessible:true,usb_access:false};
'''

def main():
    root=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=root/'target/release/deckforge')
    parser.add_argument('--output',type=Path,default=root/'target/pin-exclusions-test')
    args=parser.parse_args();output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-pin-test-') as folder:
        temporary=Path(folder);script=temporary/'test.js';script.write_text(SCRIPT)
        env=dict(os.environ,XDG_CONFIG_HOME=str(temporary/'config'),XDG_STATE_HOME=str(temporary/'private-state'),XDG_DATA_HOME=str(temporary/'data'),DECKFORGE_CAPTURE=str(output/'native.png'),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(output/'result.json'))
        with (output/'host.log').open('w') as log:
            result=subprocess.run([str(args.binary.resolve()),'desktop','--simulate','--state',str(temporary/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=30)
        proof=json.loads((output/'result.json').read_text());assert proof.get('ok'),proof
        assert result.returncode==0,result.returncode
    print('PASS: native page exclusions, editor checkbox and pinned-page recovery; no USB')
if __name__=='__main__':main()
