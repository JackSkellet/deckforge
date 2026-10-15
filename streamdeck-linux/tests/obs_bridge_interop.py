#!/usr/bin/env python3
"""Run a private OBS under Xvfb and a network namespace. Never contact live OBS."""
import argparse, hashlib, json, os, select, shutil, signal, subprocess, sys, tempfile, time
from pathlib import Path

PROBE = r'''
const fs=require('node:fs'),vm=require('node:vm');
globalThis.crypto=require('node:crypto').webcrypto; globalThis.window=globalThis; globalThis.event=undefined; // Node's browser-compatible WebSocket events are required by the vendor client.
const source=process.argv[3];
const files=['config.js','lib/TypedArray.toHex.js','lib/JSON.js','lib/jsonrpc.js','lib/uuid.js','lib/dbfs.js','lib/common.js',
'obs_frontend_recording.js','obs_frontend_replaybuffer.js','obs_frontend_scene.js','obs_frontend_scenecollection.js','obs_frontend_profile.js','obs_frontend_streaming.js','obs_frontend_virtualcam.js','obs_frontend_studiomode.js','obs_frontend_transition.js','obs_frontend.js','obs_scene.js','obs_source.js','obs.js'];
for(const file of files)vm.runInThisContext(fs.readFileSync(source+'/'+file,'utf8'),{filename:file});
vm.runInThisContext(`
const obs=new OBS({log:()=>{},_applicationInfo:{plugin:{version:'3.0.3.31'}}});
const deadline=setTimeout(()=>{console.error('OBS client handshake timed out');process.exit(1)},20000);
obs.addEventListener('open',()=>{
 obs.call('obs.frontend.scene.list', scenes=>{
  if(!Array.isArray(scenes)||scenes.length!==2)throw Error('Expected two isolated scenes');
  obs.call('obs.frontend.scene', before=>{
   const target=scenes.find(s=>s!==before);
   obs.call('obs.frontend.scene', changed=>{
    if(changed!==target)throw Error('Scene switch failed');
    obs.call('obs.frontend.scene', restored=>{
     if(restored!==before)throw Error('Scene restore failed');
     console.log('DECKFORGE_OBS_PROOF '+JSON.stringify({connected:true,obs_version:obs.obs_version,scenes,before,scene_switch:true,scene_restore:true,official_client:true}));
     clearTimeout(deadline);obs.disconnect();process.exit(0);
    },{scene:before});
   },{scene:target});
  });
 });
});
obs.connect();
`);
'''
EDITOR = r'''const b=window.deckforge,pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,label)=>{for(let i=0;i<250;i++){const v=await f();if(v)return v;await pause(80)}throw Error('Timed out: '+label)};
const plugin=await until(async()=>{const s=await b.request('snapshot');return s.plugins.find(p=>p.id==='com.elgato.obsstudio'&&p.connected)},'OBS plugin');
[...document.querySelectorAll('button')].find(b=>b.textContent.startsWith('Plugins')).click();
const configure=await until(()=>document.querySelector('[aria-label="Configure OBS Studio"]'),'Configure');configure.click();
await until(()=>document.body.textContent.includes('OBS 32.2.1 connected'),'bridge status');
await pause(400);await b.request('_test_progress',{stage:'menu'});await pause(900);
const scene=plugin.setup_actions.find(a=>a.uuid==='com.elgato.obsstudio.scene');if(!scene)throw Error('Scene settings missing');
const select=document.querySelector('[aria-label="OBS Studio settings page"]');
Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(select,scene.uuid);select.dispatchEvent(new Event('change',{bubbles:true}));await pause(100);
[...document.querySelectorAll('button')].find(b=>b.textContent==='Open plugin settings').click();
await until(async()=>{const s=await b.request('snapshot');return s.inspectors.some(i=>i.connected)},'native inspector registration');
await until(async()=>{const s=await b.request('snapshot');return s.logs.some(l=>l.includes('Connected with OBS Studio.'))},'official OBS plugin bridge connection');
await pause(2200);await b.request('_test_progress',{stage:'inspector'});await pause(900);
return {plugin_connected:true,bridge_status:true,setup_from_menu:true,native_inspector_registered:true,official_backend_connected:true};
'''
def inside(project, bundle, vendor, output, reference=None):
    subprocess.run(['ip','link','set','lo','up'],check=True)
    processes=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-obs-test-') as d:
        root=Path(d); config=root/'config'; plugin=config/'obs-studio/plugins/StreamDeckPlugin'
        (plugin/'bin/64bit').mkdir(parents=True)
        shutil.copy2(bundle/'StreamDeckPlugin.so',plugin/'bin/64bit/StreamDeckPlugin.so')
        shutil.copytree(bundle/'data',plugin/'data')
        (config/'obs-studio/basic/profiles/Test').mkdir(parents=True)
        (config/'obs-studio/global.ini').write_text('[General]\nFirstRun=false\n[Basic]\nProfile=Test\nProfileDir=Test\nSceneCollection=Test\nSceneCollectionFile=Test\n[BasicWindow]\nShowWhatsNew=false\n')
        (config/'obs-studio/basic/profiles/Test/basic.ini').write_text('[General]\nName=Test\n[Audio]\nDesktopDevice1=disabled\nDesktopDevice2=disabled\nAuxDevice1=disabled\nAuxDevice2=disabled\nAuxDevice3=disabled\n[Video]\nBaseCX=640\nBaseCY=360\nOutputCX=640\nOutputCY=360\nFPSType=0\nFPSCommon=10\n')
        scenes=config/'obs-studio/basic/scenes';scenes.mkdir(parents=True)
        (scenes/'Untitled.json').write_text(json.dumps({'name':'Untitled','current_scene':'Deckforge A','current_program_scene':'Deckforge A','scene_order':[{'name':n} for n in ['Deckforge A','Deckforge B']], 'sources':[{'name':n,'id':'scene','versioned_id':'scene','settings':{'items':[]}} for n in ['Deckforge A','Deckforge B']]}))
        (config/'obs-studio/user.ini').write_text('[Basic]\nProfile=Test\nProfileDir=Test\nSceneCollection=Untitled\nSceneCollectionFile=Untitled\n')
        runtime=root/'run';runtime.mkdir(mode=0o700)
        env={**os.environ,'HOME':str(root),'XDG_CONFIG_HOME':str(config),'XDG_DATA_HOME':str(root/'data'),'XDG_CACHE_HOME':str(root/'cache'),'XDG_RUNTIME_DIR':str(runtime),'QT_QPA_PLATFORM':'xcb','LIBGL_ALWAYS_SOFTWARE':'1','PULSE_SERVER':'unix:'+str(root/'no-pulse'),'PIPEWIRE_REMOTE':'no-pipewire'}
        env.pop('WAYLAND_DISPLAY',None);env.pop('DBUS_SESSION_BUS_ADDRESS',None)
        with (output/'obs.log').open('w') as log:
            try:
                read,write=os.pipe()
                xvfb=subprocess.Popen([str(project/'target/x11-layouts/tools/usr/bin/Xvfb'),'-displayfd',str(write),'-screen','0','1280x900x24','-nolisten','tcp','-noreset'],pass_fds=(write,),stdout=log,stderr=log);processes.append(xvfb);os.close(write)
                assert select.select([read],[],[],5)[0];env['DISPLAY']=':'+os.read(read,100).decode().strip();os.close(read)
                obs=subprocess.Popen(['obs','--multi','--disable-updater','--disable-shutdown-check','--disable-missing-files-check'],env=env,stdout=log,stderr=log,start_new_session=True);processes.append(obs)
                for _ in range(300):
                    text=(output/'obs.log').read_text()
                    if '==== Startup complete' in text:break
                    if obs.poll() is not None:raise RuntimeError('Private OBS exited; inspect obs.log')
                    time.sleep(.1)
                else:raise RuntimeError('Private OBS startup timeout; inspect obs.log')
                probe=root/'probe.cjs';probe.write_text(PROBE)
                result=subprocess.run(['node','--experimental-websocket',str(probe),str(project/'frontend/node_modules/ws'),str(vendor/'source')],capture_output=True,text=True,timeout=25,env=env)
                (output/'client.log').write_text(result.stdout+result.stderr)
                result.check_returncode()
                line=next(l for l in result.stdout.splitlines() if l.startswith('DECKFORGE_OBS_PROOF '))
                status=json.loads(subprocess.check_output([str(project/'target/release/deckforge'),'obs-bridge'],env=env,text=True))
                assert status['connection']['obs_version'] and status['installed'],status
                proof=json.loads(line.split(' ',1)[1]);proof['deckforge_status']=status['connection'];proof.update(network_namespace=True,user_obs_touched=False,bridge_sha256=hashlib.sha256((bundle/'StreamDeckPlugin.so').read_bytes()).hexdigest())
                if reference:
                    state=root/'deckforge-state';state.mkdir()
                    (state/'session.json').write_text(json.dumps({'slots':[], 'reference_app':str(reference), 'plugins':[str(vendor)]}))
                    script=root/'editor.js';script.write_text(EDITOR)
                    editor_proof=root/'editor-proof.json'
                    editor_env={**env,'GDK_BACKEND':'x11','WEBKIT_DISABLE_COMPOSITING_MODE':'1','DECKFORGE_WEBVIEW_TEST_SCRIPT':str(script),'DECKFORGE_WEBVIEW_TEST_RESULT':str(editor_proof)}
                    with (output/'editor.log').open('w') as editor_log:
                        editor=subprocess.Popen([str(project/'target/release/deckforge'),'desktop','--simulate','--state',str(state)],env=editor_env,stdout=editor_log,stderr=editor_log,start_new_session=True)
                        processes.append(editor); captured=set()
                        deadline=time.monotonic()+40
                        while time.monotonic()<deadline:
                            lines=(output/'editor.log').read_text()
                            for stage in ['menu','inspector']:
                                if stage not in captured and '"stage":"'+stage+'"' in lines:
                                    subprocess.run(['import','-window','root',str(output/(stage+'.png'))],env=env,check=True,timeout=5);captured.add(stage)
                            if editor_proof.exists(): break
                            if editor.poll() is not None: raise RuntimeError('Editor exited during OBS setup')
                            time.sleep(.1)
                        assert editor_proof.exists(), 'OBS editor timeout'
                        editor_result=json.loads(editor_proof.read_text());assert editor_result['ok'],editor_result
                        proof['native_official_plugin_setup']=editor_result['result']
                (output/'proof.json').write_text(json.dumps(proof,indent=2)+'\n')
            finally:
                for process in reversed(processes):
                    if process.poll() is None:
                        process.terminate()
                        try:process.wait(timeout=5)
                        except subprocess.TimeoutExpired:process.kill();process.wait()
    print('PASS: private OBS loads Linux bridge; official client connects, switches and restores private scenes. No live OBS access.')

def main():
    p=argparse.ArgumentParser(description=__doc__);project=Path(__file__).resolve().parents[1]
    p.add_argument('--bundle',type=Path,default=project/'target/release/obs-bridge')
    p.add_argument('--vendor',type=Path,default=Path.home()/'.local/share/deckforge/plugins/com.elgato.obsstudio.sdPlugin')
    p.add_argument('--reference-app',type=Path,help='Also exercise the installed protected plugin and its real native inspector')
    p.add_argument('--output',type=Path,default=project/'target/obs-interop');p.add_argument('--inside',action='store_true')
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    if not a.inside:
        for name in ['proof.json','menu.png','inspector.png']:(a.output/name).unlink(missing_ok=True)
    if not a.inside:
        return subprocess.run(['unshare','--user','--map-root-user','--net',sys.executable,str(Path(__file__).resolve()),'--inside','--bundle',str(a.bundle.resolve()),'--vendor',str(a.vendor.resolve()),'--output',str(a.output.resolve()),*(['--reference-app',str(a.reference_app.resolve())] if a.reference_app else [])],check=True)
    inside(project,a.bundle.resolve(),a.vendor.resolve(),a.output.resolve(),a.reference_app)
if __name__=='__main__':main()
