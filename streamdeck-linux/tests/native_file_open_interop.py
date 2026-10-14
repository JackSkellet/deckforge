#!/usr/bin/env python3
"""Warm/cold desktop file dispatch, URI paths, clipboard paste and draft retention."""
import argparse
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import uuid
import zipfile
from pathlib import Path
from native_editor_interop import cleanup, environment, result, wait, command

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p),phase=__PHASE__;
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
if(phase==='cold'){
 const profile=s.library.profiles.find(p=>p.id===s.library.active_profile);
 if(profile?.name!=='Desktop profile'||s.slots[0]?.settings.text!=='from desktop')throw Error('Cold profile open failed');
 return {phase,profile_opened:true,usb_access:false};
}
const draftName='Unsaved before opening';
if(s.slots.some(v=>v.controller==='Keypad'&&v.index===1&&v.settings.text==='from desktop')){
 s=await until(async()=>{const v=await snapshot();return v.file_open?.error&&v},'bad file result');
 await until(()=>document.body.textContent.includes('Could not open file'),'visible file error');
 if(s.clipboard?.name!=='Desktop import')throw Error('Bad file replaced clipboard');
 if(!Object.values((await b.request('ui_state_get'))?.drafts||{}).some(d=>d.title===draftName&&d.dirty))throw Error('Bad file lost draft');
 return {phase,invalid_archive_visible:true,clipboard_and_draft_preserved:true,usb_access:false};
}
if(!Object.values((await b.request('ui_state_get'))?.drafts||{}).some(d=>d.title===draftName&&d.dirty)){
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 const input=await until(()=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent==='Title')?.querySelector('textarea,input'),'title');
 Object.getOwnPropertyDescriptor(input.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value').set.call(input,draftName);input.dispatchEvent(new Event('input',{bubbles:true}));
 await until(async()=>Object.values((await b.request('ui_state_get'))?.drafts||{}).some(d=>d.title===draftName&&d.dirty),'draft retained');
 void b.request('_test_result',{ok:true,result:{phase:'seed'}});await new Promise(()=>{});
}
s=await until(async()=>{const v=await snapshot();return v.clipboard?.name==='Desktop import'&&v},'external action clipboard');
if(s.slots.length!==1||s.library.profiles.length!==1)throw Error('File opening changed assignments or added a profile');
const original=s.slots[0].context;
if(!(await b.request('ui_state_get')).drafts[original]?.dirty)throw Error('File opening lost the unsaved draft');
await until(()=>document.body.textContent.includes('Action copied from file'),'file status visible');
const key=document.querySelectorAll('.deck-key')[1];key.click();await pause(60);key.dispatchEvent(new KeyboardEvent('keydown',{key:'v',ctrlKey:true,bubbles:true,cancelable:true}));
await until(async()=>{const p=(await snapshot()).slots.find(s=>s.index===1&&s.controller==='Keypad');return p?.settings.text==='from desktop'},'clipboard pasted');
if(!(await b.request('ui_state_get')).drafts[original]?.dirty)throw Error('Paste lost original draft');
void b.request('_test_result',{ok:true,result:{phase:'pasted',action_loaded_to_clipboard:true,draft_preserved:true,independent_paste:true,usb_access:false}});await new Promise(()=>{});
'''

def archive(path, action):
    profile,page,pins=(str(uuid.uuid4()) for _ in range(3));base=f'Profiles/{profile}.sdProfile/'
    model='Fake/Storage' if action else '20GBD9901'
    files={'package.json':{'FormatVersion':1,'DeviceModel':model,'RequiredPlugins':[]},
        base+'manifest.json':{'Name':'Shared action' if action else 'Desktop profile','Version':'3.0',
            'Device':{'UUID':'$storage' if action else 'fixture','Model':model},'Pages':{'Current':page,'Default':pins,'Pages':[page]}},
        base+f'Profiles/{page}/manifest.json':{'Controllers':[{'Type':'Keypad','Actions':{'0,0':{
            'UUID':'com.elgato.streamdeck.system.text','ActionID':str(uuid.uuid4()),'Name':'Desktop import',
            'Settings':{'text':'from desktop'},'States':[{'Title':'From file'}]}}}]},
        base+f'Profiles/{pins}/manifest.json':{'Controllers':[]}}
    with zipfile.ZipFile(path,'w') as z:
        for name,value in files.items():z.writestr(name,json.dumps(value))

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/file-open/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'native.json').unlink(missing_ok=True)
    binary=a.binary.resolve();reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-file-open-') as d:
        root=Path(d);action=root/'space % # action.streamDeckAction';profile=root/'space % profile.streamDeckProfile'
        archive(action,True);archive(profile,False)
        for phase in ['warm','cold']:
            state=(root/'data/deckforge') if phase=='warm' else root/phase
            script=root/'test.js';proof=root/'proof.json';proof.unlink(missing_ok=True)
            script.write_text(SCRIPT.replace('__PHASE__',json.dumps(phase)))
            env=environment(root);env['XDG_DATA_HOME']=str(root/'data');env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/(phase+'.png')))
            args=[str(binary),'desktop','--simulate','--model','plus','--state',str(state)]
            if phase=='cold':args+=['--open',profile.as_uri()]
            with (a.output/(phase+'.log')).open('w') as log:
                process=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    if phase=='warm':
                        wait(lambda:result(proof,'seed'),'seed draft');command(binary,state,'hide');wait(lambda:not command(binary,state,'status')['window_open'],'hidden editor')
                        pid=command(binary,state,'status')['pid'];assert pid==process.pid
                        spec=importlib.util.spec_from_file_location('installer',project/'scripts/install-desktop.py')
                        installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
                        entry=root/'deckforge-test.desktop';entry.write_text('[Desktop Entry]\nType=Application\nName=Deckforge test\nExec='+installer.desktop_quote(binary)+' %u\nTerminal=false\n')
                        dispatch=subprocess.run(['gio','launch',str(entry),action.as_uri()],env=env,capture_output=True,text=True,timeout=5)
                        assert dispatch.returncode==0,dispatch.stderr
                        assert command(binary,state,'status')['pid']==pid
                        wait(lambda:result(proof,'pasted'),'action pasted');reports.append(result(proof))
                        command(binary,state,'hide');wait(lambda:not command(binary,state,'status')['window_open'],'editor hidden after paste')
                        bad=root/'bad.streamDeckAction';bad.write_bytes(b'not a zip')
                        dispatch=subprocess.run(['gio','launch',str(entry),bad.as_uri()],env=env,capture_output=True,text=True,timeout=5)
                        assert dispatch.returncode==0,dispatch.stderr
                        assert command(binary,state,'status')['pid']==pid
                    code=process.wait(timeout=45);report=result(proof);assert code==0 and report and report['result']['phase']==phase,report;reports.append(report)
                finally:cleanup(process)
            if phase=='warm':
                saved=json.loads((state/'session.json').read_text())
                pasted=next(slot for p in saved['library']['profiles'] for page in p['pages'] for slot in page['slots'] if slot['settings'].get('text')=='from desktop')
                assert Path(pasted['asset_source']['source']).is_file(), 'Shutdown removed an archive still used by pasted data'
        invalid=subprocess.run([str(binary),'desktop','--state',str(root/'invalid'),'--open','https://example.com/file.streamDeckAction'],capture_output=True,text=True)
        assert invalid.returncode!=0 and not (root/'invalid').exists()
    (a.output/'native.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'runs':reports,'same_owner_pid':True,'gio_desktop_launch':True,'pasted_source_retained_after_shutdown':True,'remote_uri_rejected':True,'usb_access':False},indent=2)+'\n')
    print('PASS: warm file URI -> existing host clipboard with drafts preserved, paste, cold profile file open, remote URI rejection; no USB.')

if __name__=='__main__':main()
