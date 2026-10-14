#!/usr/bin/env python3
"""Native settings and real KWin focus routing, pause/manual override and recovery."""
import argparse
import hashlib
import json
import os
import shlex
import subprocess
import tempfile
import time
import uuid
from pathlib import Path
from native_editor_interop import cleanup, environment, result, command
from focus_monitor_interop import SOURCE, dbus, wait

SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<400;i++){const v=await f();if(v)return v;await pause(35)}throw Error('Timeout: '+label)};
const b=window.deckforge,cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot'),fixture=__FIXTURE__,phase=__PHASE__;
const field=name=>[...document.querySelectorAll('label.field')].find(l=>l.querySelector('span')?.textContent===name)?.querySelector('input,select');
const set=(element,value)=>{Object.getOwnPropertyDescriptor(element.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype,'value').set.call(element,value);element.dispatchEvent(new Event('input',{bubbles:true}));element.dispatchEvent(new Event('change',{bubbles:true}));};
let s=await snapshot();if(!s.simulate)throw Error('Private simulation required');
if(phase==='setup'){
 const base=s.library.active_profile;
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 await cmd({command:'settings',context:(await snapshot()).slots[0].context,settings:{text:'fallback'}});
 await cmd({command:'profile_create',name:'Focused fixture'});s=await snapshot();const target=s.library.active_profile;
 await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
 await cmd({command:'settings',context:(await snapshot()).slots[0].context,settings:{text:'focused'}});
 await cmd({command:'profile_select',id:base});
 const open=async()=>{const details=document.querySelector('.sidebar-bottom details');if(!details.open)details.querySelector('summary').click();(await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent==='Smart Profiles…'&&!b.disabled),'Smart entry')).click();await until(()=>document.querySelector('dialog[open]'),'Smart dialog')};
 await open();
 [...document.querySelectorAll('label.check')].find(l=>l.textContent.includes('Switch profiles automatically'))?.querySelector('input').click();
 // Check components may use a plain label rather than .check.
 const enabled=()=>[...document.querySelectorAll('label')].find(l=>l.textContent.trim()==='Switch profiles automatically')?.querySelector('input');
 if(!enabled()?.checked)enabled().click();
 await until(async()=>(await b.request('ui_state_get')).smartEdit?.settings.enabled,'enabled draft');
 set(await until(()=>field('Default profile'),'default choice'),base);
 await until(async()=>(await b.request('ui_state_get')).smartEdit?.settings.fallback===base,'default draft');
 set(field('Profile to configure'),target);await pause(50);
 document.querySelector('[data-smart-advanced] summary').click();
 set(field('Match by'),'executable');await pause(50);
 set(field('Application identity'),fixture);await until(async()=>(await b.request('ui_state_get')).smartEdit?.settings.bindings[target]?.value===fixture,'identity draft');
 set(field('Application label'),'Owned fixture');await pause(80);
 const save=[...document.querySelectorAll('button')].find(b=>b.textContent==='Save Smart Profiles');save.scrollIntoView({block:'center'});save.click();
 s=await until(async()=>{const s=await snapshot();return s.smart_profiles.config.enabled&&s.smart_profiles.config.bindings[target]?.value===fixture&&s},'saved Smart rules');
 if(!s.smart_profiles.paused)throw Error('Visible editor did not pause switching');
 await until(()=>!document.querySelector('dialog[open]'),'dialog closed');
 await open();set(await until(()=>field('Profile to configure'),'profile choice'),target);await pause(80);
 const footer=[...document.querySelectorAll('button')].find(b=>b.textContent==='Save Smart Profiles');footer.scrollIntoView({block:'nearest'});await pause(80);
 const rect=document.querySelector('dialog[open]').getBoundingClientRect(),buttonRect=footer.getBoundingClientRect();if(rect.top<0||rect.bottom>innerHeight+1||buttonRect.bottom>rect.bottom||buttonRect.top<rect.top)throw Error('Smart dialog or Save button outside window');
 return {phase,base,target,config:s.smart_profiles.config,native_form:true};
}
const config=s.smart_profiles.config,base=config.fallback,target=Object.keys(config.bindings).find(id=>config.bindings[id].value===fixture);
await until(async()=>{const s=await snapshot();return s.smart_profiles.observing&&!s.smart_profiles.error},'focus observer ready');
void b.request('_test_result',{ok:true,result:{phase:'watching',base,target}});
await until(async()=>(await snapshot()).smart_profiles.focus?.executable===fixture,'real fixture focus');
await pause(650);s=await snapshot();
if(!s.smart_profiles.paused||s.library.active_profile!==base)throw Error('Visible editor allowed automatic routing');
void b.request('_test_result',{ok:true,result:{phase:'focused_visible'}});
await until(async()=>{const s=await snapshot();return !s.smart_profiles.paused&&s.library.active_profile===target&&s.slots[0].settings.text==='focused'},'minimized focus routing');
await cmd({command:'profile_select',id:base});await pause(650);s=await snapshot();
if(s.library.active_profile!==base)throw Error('Same focused app overrode manual profile selection');
void b.request('_test_result',{ok:true,result:{phase:'manual_override',base,target}});
await new Promise(()=>{});
'''

def state_json(path):
    try:return json.loads((path/'session.json').read_text())
    except (OSError,json.JSONDecodeError):return None

def active(path,profile,text):
    s=state_json(path)
    return s and s['library']['active_profile']==profile and s['slots'][0]['settings'].get('text')==text

def window_action(root,pid,mode):
    name='deckforge-smart-test-'+str(uuid.uuid4());script=root/'window.js'
    action={'activate':'w.minimized=false;workspace.activeWindow=w;','minimize':'w.minimized=true;','restore':'w.minimized=false;workspace.activeWindow=w;'}[mode]
    script.write_text(f'for(const w of workspace.windowList()){{if(w.pid==={pid}&&w.normalWindow){{{action}break;}}}}')
    identifier=int(dbus('/Scripting','org.kde.kwin.Scripting.loadScript',script,name));assert identifier>=0
    try:dbus(f'/Scripting/Script{identifier}','org.kde.kwin.Script.run')
    finally:dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',name)

def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/smart-profiles/native')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);(a.output/'summary.json').unlink(missing_ok=True);binary=a.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='deckforge-smart-native-') as d:
        root=Path(d);state=root/'state';fixture=root/'focus-fixture';source=root/'fixture.c';source.write_text(SOURCE.replace('g_timeout_add(2000,','g_timeout_add(20000,'))
        flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','gtk+-3.0'],text=True));subprocess.run(['cc',str(source),'-o',str(fixture),*flags],check=True,capture_output=True)
        script=root/'test.js';proof=root/'proof.json';env=environment(root)
        script.write_text(SCRIPT.replace('__FIXTURE__',json.dumps(str(fixture))).replace('__PHASE__',json.dumps('setup')))
        setup_env={**env,'DECKFORGE_WEBVIEW_TEST_SCRIPT':str(script),'DECKFORGE_WEBVIEW_TEST_RESULT':str(proof),'DECKFORGE_CAPTURE':str(a.output.resolve()/'settings.png')}
        with (a.output/'setup.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(state)],env=setup_env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:assert process.wait(timeout=40)==0;setup=result(proof)['result']
            finally:cleanup(process)
        (a.output/'setup.json').write_text(json.dumps(setup,indent=2)+'\n')
        base,target=setup['base'],setup['target'];name='deckforge-focus-'+hashlib.sha256(os.fsencode(state.resolve())).hexdigest()
        assert dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',name)=='false'
        assert not (state/'.smart-profile-focus.js').exists()
        proof.unlink();script.write_text(SCRIPT.replace('__FIXTURE__',json.dumps(str(fixture))).replace('__PHASE__',json.dumps('live')))
        live_env={**env,'DECKFORGE_WEBVIEW_TEST_SCRIPT':str(script),'DECKFORGE_WEBVIEW_TEST_RESULT':str(proof)}
        with (a.output/'live.log').open('w') as log,(a.output/'fixture.log').open('w') as fixture_log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(state)],env=live_env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);window=None
            try:
                wait(lambda:result(proof,'watching'),'native observer ready',10)
                window=subprocess.Popen([str(fixture)],stdout=fixture_log,stderr=fixture_log);time.sleep(.2);assert window.poll() is None;window_action(root,window.pid,'activate')
                wait(lambda:result(proof,'focused_visible'),'visible editor paused',10)
                window_action(root,process.pid,'minimize')
                wait(lambda:result(proof,'manual_override'),'minimized routing and manual override',10)
                window_action(root,process.pid,'restore');time.sleep(.3)
                command(binary,state,'hide');wait(lambda:not command(binary,state,'status')['window_open'],'hidden editor')
                window_action(root,window.pid,'activate');wait(lambda:active(state,target,'focused'),'hidden focus route',10)
                window.terminate();window.wait(timeout=4);window=None
                try:wait(lambda:active(state,base,'fallback'),'default fallback',10)
                except Exception:
                    (a.output/'failure-state.json').write_text(json.dumps(state_json(state),indent=2))
                    observed=subprocess.run([str(binary),'focus-probe','--seconds','1'],capture_output=True,text=True,timeout=5)
                    (a.output/'failure-focus.json').write_text(observed.stdout)
                    raise
                command(binary,state,'quit');assert process.wait(timeout=10)==0
            finally:
                if window is not None:cleanup(window)
                cleanup(process)
        assert dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',name)=='false'
        # Restart background mode, then deliberately kill this private simulated
        # owner to verify recovery of only its deterministic KWin script name.
        with (a.output/'restart.log').open('w') as log,(a.output/'fixture-restart.log').open('w') as fixture_log:
            window=subprocess.Popen([str(fixture)],stdout=fixture_log,stderr=fixture_log);time.sleep(.2);window_action(root,window.pid,'activate')
            process=subprocess.Popen([str(binary),'desktop','--simulate','--background','--state',str(state)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            recovered=None
            try:
                wait(lambda:active(state,target,'focused'),'restart settings route',10)
                wait(lambda:dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',name)=='true','owned observer script')
                process.kill();process.wait(timeout=4)
                assert dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',name)=='true'
                recovered=subprocess.Popen([str(binary),'desktop','--simulate','--background','--state',str(state)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                wait(lambda:command(binary,state,'status',required=False),'recovered native endpoint',10)
                time.sleep(.6);window.terminate();window.wait(timeout=4)
                wait(lambda:active(state,base,'fallback'),'recovered observer receives new focus',10)
                command(binary,state,'quit');assert recovered.wait(timeout=10)==0
            finally:
                cleanup(window);cleanup(process)
                if recovered is not None:cleanup(recovered)
                # An assertion must not leave this owned test script loaded.
                if dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',name)=='true':dbus('/Scripting','org.kde.kwin.Scripting.unloadScript',name)
        assert dbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',name)=='false'
        assert not (state/'.smart-profile-focus.js').exists()
    report={'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'native_settings':True,'real_kde_focus':True,'visible_pause':True,
        'minimized_routing':True,'manual_override':True,'hidden_routing':True,'default_fallback':True,'restart':True,
        'crash_recovery':True,'scripts_and_runtime_file_cleaned':True,'owned_windows_only':True,'usb_access':False}
    (a.output/'summary.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))

if __name__=='__main__':main()
