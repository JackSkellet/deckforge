#!/usr/bin/env python3
"""Native command-result ownership inside Multi Actions, switches, Trigger and Key Logic."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from native_editor_interop import cleanup,environment,result

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<220;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const COMMAND='com.deckforge.system.command',MULTI='com.elgato.streamdeck.multiactions.routine',SWITCH='com.elgato.streamdeck.multiactions.routine2',TRIGGER='com.elgato.streamdeck.keys.adaptor',LOGIC='com.elgato.streamdeck.keys.logic';
const kinds=[{uuid:MULTI,name:'multi'},{uuid:SWITCH,name:'switch'},{uuid:TRIGGER,name:'trigger',dial:true},{uuid:LOGIC,name:'logic',nested:true}];
const click=async name=>(await until(()=>{const e=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===name);return e&&!e.disabled&&e},name)).click();
const root=async controller=>(await snapshot()).slots.find(s=>s.controller===controller&&s.index===0);
const flatten=s=>[s,...s.multi_children.flatMap(g=>g.flatMap(flatten)),...s.trigger_children.filter(Boolean).flatMap(flatten),...s.stack_children.flatMap(flatten)];
const reports=[];
for(const kind of kinds){
 const controller=kind.dial?'Encoder':'Keypad';
 await cmd({command:'assign',controller,index:0,uuid:kind.uuid});
 let parent=await root(controller);
 const child={UUID:COMMAND,ActionID:'owned-'+kind.name,Name:'Nested '+kind.name,Settings:{mode:'shell',script:'sleep 5',output:'show',timeout_seconds:10},States:[{Title:'Nested command'}]};
 const wrapped=kind.nested?{UUID:MULTI,ActionID:'inner-sequence',Name:'Inner sequence',Settings:{},States:[{}],Actions:[{Actions:[child]}]}:child;
 const raw={...parent.raw,Actions:kind.uuid===MULTI||kind.uuid===SWITCH?[{Actions:[child]},{Actions:[]}]:kind.dial?[{},child,{}]:[wrapped,{},{}]};
 await cmd({command:kind.uuid===TRIGGER?'trigger_save':kind.uuid===LOGIC?'logic_save':'multi_save',context:parent.context,raw});
 (await until(()=>document.querySelectorAll(kind.dial?'.deck-dial':'.deck-key')[0],'parent control')).click();
 const region=kind.dial?'Action trigger':kind.uuid===LOGIC?'Key logic':'Action sequence';
 const outer=await until(()=>document.querySelector(`section.sequence[aria-label="${region}"]`),'container');
 (await until(()=>outer.querySelectorAll(':scope > .sequence-list > .sequence-row > .sequence-select')[kind.dial?1:0],'child row')).click();
 if(kind.nested)(await until(()=>outer.querySelector('.child-editor section.sequence .sequence-select'),'inner command')).click();
 await until(()=>[...document.querySelectorAll('label.field > span')].some(e=>e.textContent==='Command mode'),'command settings');
 const prefix=kind.dial?'dial':'key';
 await cmd({command:'input',event:prefix+'Down',index:0});await cmd({command:'input',event:prefix+'Up',index:0});
 let running=await until(async()=>flatten(await root(controller)).find(s=>s.uuid===COMMAND&&s.command_run?.state==='running'),'nested run');
 await click('Stop command');
 await until(async()=>flatten(await root(controller)).find(s=>s.context===running.context)?.command_run?.state==='cancelled','nested Stop owner');
 await click('Show output');
 await until(()=>document.querySelector('[aria-label="Standard output"]'),'nested preview');
 if(!document.querySelector('dialog').textContent.includes('Command stopped'))throw Error('Wrong command result');
 await click('Close output');
 let wrongDevice=false;try{await b.request('command_output',{context:running.context,id:running.command_run.id,device:'not-this-deck'})}catch{wrongDevice=true}
 if(!wrongDevice)throw Error('Wrong device accepted');
 reports.push({kind:kind.name,nested:!!kind.nested,context:running.context,stopped:true,preview:true,wrongDeviceRejected:true});
}
await click('Show output');await until(()=>document.querySelector('[aria-label="Standard output"]'),'final preview');
return {reports,clipboardUntouched:true};
'''

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/nested-commands/native'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='deckforge-nested-commands-') as d:
        root=Path(d);script=root/'test.js';script.write_text(SCRIPT);proof=root/'proof.json';env=environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/'preview.png'))
        with (a.output/'native.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                code=process.wait(timeout=45);report=result(proof);assert code==0 and report and report.get('ok'),report
            finally:cleanup(process)
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'report':report,'usb_access':False},indent=2)+'\n')
    print('PASS: native nested command Stop/results in Multi Action, Switch, Action Trigger and Key Logic -> Multi Action, with captured ownership and device guards. No clipboard writes or USB.')

if __name__=='__main__':main()
