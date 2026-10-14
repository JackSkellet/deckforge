#!/usr/bin/env python3
"""Warm/cold desktop package opening and native drop dispatch preserve editor drafts."""
import argparse
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import zipfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result, wait, command
from sample_interop import SAMPLE_COMMIT, COUNTER_LOCK_SHA256

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<220;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),kind=__KIND__,mode=__MODE__;
const title='Draft before package installation';
const field=()=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent==='Title')?.querySelector('textarea,input');
const state=await b.request('ui_state_get');let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
const retained=Object.values(state?.drafts||{}).some(d=>d.title===title&&d.dirty);
if(mode!=='cold'&&!retained){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 const input=await until(()=>field(),'title');Object.getOwnPropertyDescriptor(input.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(input,title);input.dispatchEvent(new Event('input',{bubbles:true}));
 await until(async()=>Object.values((await b.request('ui_state_get')).drafts||{}).some(d=>d.title===title&&d.dirty),'retained draft');
 if(mode==='warm'){void b.request('_test_result',{ok:true,result:{phase:'seed'}});await new Promise(()=>{});}
 const rect=document.querySelector('.deck-key').getBoundingClientRect();const drop={path:__FILE__,kind,x:rect.x+rect.width/2,y:rect.y+rect.height/2};
 window.dispatchEvent(new CustomEvent('deckforge-file-drag',{detail:{...drop,phase:'enter'}}));await pause(50);
 if(document.querySelector('.file-drop-target'))throw Error('Package hover incorrectly selected a control');
 window.dispatchEvent(new CustomEvent('deckforge-file-drag',{detail:{...drop,phase:'drop'}}));
}
const expected=kind==='plugin'?'Plugin installed.':'Icon pack installed.';
s=await until(async()=>{const s=await snapshot();return s.system_status.includes(expected)&&s},'package installed');
let pluginPid=null;
if(kind==='plugin'){
 const plugin=await until(async()=>(await snapshot()).plugins.find(p=>p.id==='com.elgato.counter'&&p.connected),'Counter registered');pluginPid=plugin.pid;if(!pluginPid)throw Error('Owned plugin process missing');
}else{
 const catalog=await b.request('icon_packs',{operation:'list'});if(catalog.packs.length!==1||catalog.packs[0].icons.length!==134)throw Error('Icon pack not installed');
}
const profiles=JSON.stringify(s.library),contexts=s.slots.map(v=>v.context).join(),revision=s.history.revision;
for(const path of [__FILE__,__BAD__]){
 let rejected=false;try{await b.request('open_file',{path,device:s.device})}catch{rejected=true}if(!rejected)throw Error('Duplicate or malformed package was accepted');
}
s=await snapshot();if(JSON.stringify(s.library)!==profiles||s.slots.map(v=>v.context).join()!==contexts||s.history.revision!==revision)throw Error('Rejected package changed editor graph/history');
if(pluginPid&&s.plugins.find(p=>p.id==='com.elgato.counter')?.pid!==pluginPid)throw Error('Duplicate open restarted plugin');
if(mode!=='cold'){
 if(field()?.value!==title||!Object.values((await b.request('ui_state_get')).drafts||{}).some(d=>d.title===title&&d.dirty))throw Error('Package opening lost draft');
 if(s.slots.find(v=>v.index===0&&v.controller==='Keypad')?.uuid!=='com.elgato.streamdeck.system.text')throw Error('Package drop replaced control');
}
return {phase:'done',mode,kind,installed:true,draft_preserved:mode!=='cold',duplicates_and_bad_archives_rejected:true,plugin_pid:pluginPid,usb_access:false};
'''

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/package-open/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True)
    binary=a.binary.resolve();samples=project.parent/'evidence/official-samples';counter=samples/'counter/com.elgato.counter.sdPlugin'
    assert hashlib.sha256((samples/'counter/package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256
    if (samples/'.git').exists():assert subprocess.check_output(['git','-C',str(samples),'rev-parse','HEAD'],text=True).strip()==SAMPLE_COMMIT
    spec=importlib.util.spec_from_file_location('installer',project/'scripts/install-desktop.py');installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
    reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-package-open-') as d:
        root=Path(d);plugin=root/'plugin % # owned.streamDeckPlugin';pack=root/'icons % # owned.streamDeckIconPack';bad=root/'bad.streamDeckPlugin';bad.write_bytes(b'not a zip')
        with zipfile.ZipFile(plugin,'w',compression=zipfile.ZIP_DEFLATED) as z:
            for file in sorted(counter.rglob('*')):
                if file.is_file():z.write(file,counter.name+'/'+str(file.relative_to(counter)))
        prefix='com.elgato.keycreator.sdPlugin/static/com.elgato.logoiconscolor.sdIconPack/'
        with zipfile.ZipFile(project.parent/'evidence/elgato-7.6/raw/flsy_lZh6qrewVZPcqeNQmr2lhuQjI') as source,zipfile.ZipFile(pack,'w',compression=zipfile.ZIP_DEFLATED) as z:
            for name in source.namelist():
                if name.startswith(prefix) and not name.endswith('/'):z.writestr('com.elgato.logoiconscolor.sdIconPack/'+name[len(prefix):],source.read(name))
        for kind,path in [('plugin',plugin),('icon_pack',pack)]:
            for mode in ['warm','cold','drop']:
                case=root/(kind+'-'+mode);case.mkdir();state=case/'data/deckforge';proof=case/'proof.json';script=case/'test.js'
                script.write_text(SCRIPT.replace('__KIND__',json.dumps(kind)).replace('__MODE__',json.dumps(mode)).replace('__FILE__',json.dumps(str(path))).replace('__BAD__',json.dumps(str(bad))))
                env=environment(case);env['XDG_DATA_HOME']=str(case/'data');env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(kind+'-'+mode+'.png')))
                args=[str(binary),'desktop','--simulate','--state',str(state),'--port','0']
                if mode=='cold':args+=['--open',path.as_uri()]
                with (a.output/(kind+'-'+mode+'.log')).open('w') as log:
                    process=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                    try:
                        if mode=='warm':
                            wait(lambda:result(proof,'seed'),'seed draft');command(binary,state,'hide');wait(lambda:not command(binary,state,'status')['window_open'],'hidden editor')
                            pid=command(binary,state,'status')['pid'];assert pid==process.pid
                            entry=case/'deckforge-test.desktop';entry.write_text('[Desktop Entry]\nType=Application\nName=Deckforge package fixture\nExec='+installer.desktop_quote(binary)+' %u\nTerminal=false\n')
                            launch=subprocess.run(['gio','launch',str(entry),path.as_uri()],env=env,capture_output=True,text=True,timeout=5);assert launch.returncode==0,launch.stderr
                            assert command(binary,state,'status')['pid']==pid
                        code=process.wait(timeout=40);report=result(proof);assert code==0 and report and report['result']['phase']=='done',report;reports.append(report)
                    finally:cleanup(process)
                if kind=='plugin':
                    installed=state/'plugins'/counter.name;assert (installed/'bin/plugin.js').read_bytes()==(counter/'bin/plugin.js').read_bytes()
                else:assert (state/'icon-packs/com.elgato.logoiconscolor.sdIconPack/icons/LogoAdobe-Color.svg').is_file()
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sample_commit':SAMPLE_COMMIT,'runs':reports,'usb_access':False},indent=2)+'\n')
    print('PASS: plugin/icon-pack warm GIO reuse, cold opening and synthetic native drops; drafts/history preserved on rejection; no USB.')

if __name__=='__main__':main()
