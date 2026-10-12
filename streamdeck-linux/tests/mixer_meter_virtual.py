#!/usr/bin/env python3
"""Measured meters/output gains with generated PCM and owned virtual nodes only."""
import argparse, array, copy, json, math, os, pathlib, selectors, subprocess, tempfile, threading, time, uuid
from mixer_virtual import PROBE_MAIN, collect, info, pulse


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--run-virtual',action='store_true');ap.add_argument('--report',type=pathlib.Path)
    args=ap.parse_args()
    if not args.run_virtual:ap.error('--run-virtual is required; this creates only owned virtual audio nodes')
    namespace='meter'+uuid.uuid4().hex[:10]
    before=info();children=[];feeders=[];fixture_modules={};helper=None
    with tempfile.TemporaryDirectory(prefix='deckforge-meter-probe-') as temporary:
        root=pathlib.Path(temporary);(root/'src').mkdir()
        (root/'Cargo.toml').write_text('[package]\nname="deckforge-meter-probe"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nanyhow="1"\nserde={version="1",features=["derive"]}\nserde_json="1"\nlibpulse-binding="2.30"\nsha2="0.10"\nlibc="0.2"\nuuid={version="1",features=["v4"]}\n')
        module=pathlib.Path(__file__).resolve().parents[1]/'src/mixer.rs'
        (root/'src/main.rs').write_text('#[path='+json.dumps(str(module))+'] mod mixer;\n'+PROBE_MAIN.replace('if let Some(c)=value.get("config")', 'if let Some(c)=value.get("fade"){let config:Config=serde_json::from_value(c.clone()).unwrap();worker.submit_controls(config,std::collections::BTreeMap::from([(mixer::control::Target::Route{channel:"tone".into(),mix:"mix".into()},Duration::from_secs(3))])).unwrap();}if let Some(c)=value.get("config")'))
        build_target=module.parents[1]/'target/mixer-meter-probe'
        subprocess.run(['cargo','build','--offline','--quiet','--manifest-path',str(root/'Cargo.toml')],check=True,env={**os.environ,'CARGO_TARGET_DIR':str(build_target)})
        env={**os.environ,'XDG_STATE_HOME':str(root/'private-state')}
        helper=subprocess.Popen([str(build_target/'debug/deckforge-meter-probe'),namespace],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=env)
        def request(config=None,fade=None):
            helper.stdin.write(json.dumps({'fade':fade} if fade is not None else {'config':config} if config is not None else {})+'\n');helper.stdin.flush()
            sel=selectors.DefaultSelector();sel.register(helper.stdout,selectors.EVENT_READ)
            try:assert sel.select(15),'Mixer helper did not respond';line=helper.stdout.readline()
            finally:sel.close()
            assert line,helper.stderr.read();return json.loads(line)
        def until(fn,label,timeout=5):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                value=fn()
                if value:return value
                time.sleep(.04)
            raise AssertionError(label)
        def configure(config):
            state=request(config);assert state['error'] is None,state;return state
        def modules():
            rows=subprocess.check_output(['pactl','list','short','modules'],text=True,stderr=subprocess.PIPE).splitlines()
            return [{'index':int(p[0]),'name':p[1],'argument':p[2]} for line in rows if len(p:=line.split('\t'))>=3 and p[0].isdigit()]
        def own_modules():return [m for m in modules() if 'deckforge.'+namespace+'.' in m.get('argument','')]
        def graph_gone():
            return not own_modules() and all(not d['name'].startswith('deckforge.'+namespace+'.') for kind in ['sinks','sources'] for d in json.loads(pulse('--format=json','list',kind)))
        def fixturesink(suffix):
            name='df_meter_'+namespace+'_'+suffix;argument='sink_name='+name+' rate=48000 channels=2'
            ident=int(pulse('load-module','module-null-sink',argument));fixture_modules[ident]=argument;return name
        def record(source):
            assert source.startswith('deckforge.'+namespace+'.') or source.startswith('df_meter_'+namespace+'_')
            proc=subprocess.Popen(['pacat','--record','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+source,'--client-name=Deckforge owned meter test recorder'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            children.append(proc);return proc
        def play(sink):
            assert sink=='deckforge.'+namespace+'.channel.tone'
            proc=subprocess.Popen(['pacat','--playback','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+sink,'--client-name=Deckforge owned meter test generator'],stdin=subprocess.PIPE,stderr=subprocess.PIPE)
            children.append(proc)
            def feed():
                phase=0
                try:
                    while proc.poll() is None:
                        samples=array.array('f')
                        for _ in range(480):
                            value=.2*math.sin(2*math.pi*440*phase/48000);samples.extend((value,-value));phase+=1
                        proc.stdin.write(samples.tobytes());proc.stdin.flush()
                except (BrokenPipeError,ValueError):pass
                finally:
                    try:proc.stdin.close()
                    except (BrokenPipeError,ValueError):pass
            thread=threading.Thread(target=feed,daemon=True);thread.start();feeders.append(thread);return proc
        def stop(proc):
            if proc.poll() is None:proc.terminate()
            try:proc.wait(timeout=3)
            except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=3)
        def meter_sources():
            return [i for i in json.loads(pulse('--format=json','list','source-outputs')) if i.get('properties',{}).get('deckforge.meter')=='true' and i.get('properties',{}).get('deckforge.meter.source','').startswith('deckforge.'+namespace+'.')]
        try:
            one,two=fixturesink('one'),fixturesink('two')
            config={'enabled':True,'fade_ms':0,'channels':[{'id':'tone','name':'Generated tone','apps':[],'sources':[]}],
                'mixes':[{'id':'mix','name':'Owned mix','outputs':[one,two],'volume':100,'muted':False,'output_controls':{one:{'volume':25,'muted':False},two:{'volume':75,'muted':False}}}],
                'routes':[{'channel':'tone','mix':'mix','volume':50,'muted':False}]}
            configure(config)
            # No external streams yet: meters alone must never park the graph.
            disabled=copy.deepcopy(config);disabled['enabled']=False;configure(disabled)
            try:until(graph_gone,'Meters retained nodes after immediate disable')
            except AssertionError:
                snapshot=request();print('EARLY_DISABLE_SNAPSHOT',json.dumps({k:snapshot[k] for k in ['config','active','warnings','error','levels']}))
                own_sinks={s['index'] for s in json.loads(pulse('--format=json','list','sinks')) if s['name'].startswith('deckforge.'+namespace+'.')}
                own_sources={s['index'] for s in json.loads(pulse('--format=json','list','sources')) if s['name'].startswith('deckforge.'+namespace+'.')}
                print('EARLY_DISABLE_INPUTS',json.dumps([s for s in json.loads(pulse('--format=json','list','sink-inputs')) if s['sink'] in own_sinks]))
                print('EARLY_DISABLE_CAPTURES',json.dumps([s for s in json.loads(pulse('--format=json','list','source-outputs')) if s['source'] in own_sources]))
                raise
            configure(config)
            generator=play('deckforge.'+namespace+'.channel.tone')
            records={'mix':record('deckforge.'+namespace+'.source.mix'),'one':record(one+'.monitor'),'two':record(two+'.monitor')}
            collect(records,.4)
            measured=until(lambda:(s if (s:=request())['levels']['channels'].get('tone',{}).get('available') and s['levels']['mixes'].get('mix',{}).get('available') and .17<s['levels']['channels']['tone']['peak']<.23 and .08<s['levels']['mixes']['mix']['peak']<.12 else None),'Peak meters did not measure expected stereo amplitudes')
            pcm=collect(records,.6)
            assert .20<pcm['one']['440hz']/pcm['mix']['440hz']<.30,pcm
            assert .68<pcm['two']['440hz']/pcm['mix']['440hz']<.82,pcm
            # Submit a three-second route fade without waiting for completion;
            # the captured PCM must move through intermediate amplitudes.
            config['routes'][0]['volume']=100
            request(fade=config)
            fade_early=collect(records,.5)
            collect(records,.6);fade_middle=collect(records,.4)
            collect(records,1.7);fade_final=collect(records,.4)
            amplitudes=[part['mix']['440hz'] for part in [fade_early,fade_middle,fade_final]]
            assert .09<amplitudes[0]<.135 and amplitudes[0]+.015<amplitudes[1]<.175 and .185<amplitudes[2]<.215,amplitudes
            config['routes'][0]['volume']=50;configure(config);collect(records,.3)
            ids={m['index']:m['argument'] for m in own_modules()}
            meter_ids={m['index'] for m in meter_sources()};assert len(meter_ids)==2,meter_ids
            config['channels'][0]['name']='Renamed channel';config['mixes'][0]['name']='Renamed mix';configure(config)
            assert {m['index']:m['argument'] for m in own_modules()}==ids,'Rename recreated an endpoint/module'
            assert {m['index'] for m in meter_sources()}==meter_ids,'Rename restarted own meter streams'
            assert all(p.poll() is None for p in records.values()) and generator.poll() is None,'Rename interrupted external fixture streams'
            config['mixes'][0]['output_controls'][one]['muted']=True;configure(config);collect(records,.25);muted=collect(records,.6)
            assert muted['one']['rms']<.002 and muted['two']['rms']>.025,muted
            config['mixes'][0]['muted']=True;configure(config);collect(records,.25)
            silent=until(lambda:(s if (s:=request())['levels']['mixes'].get('mix',{}).get('available') and s['levels']['mixes']['mix']['peak']<.002 else None),'Muted mix meter did not reach measured silence')
            for child in children:stop(child)
            for thread in feeders:thread.join(timeout=1)
            disabled=copy.deepcopy(config);disabled['enabled']=False;state=configure(disabled)
            until(graph_gone,'Meter readers prevented full disabled graph cleanup')
            assert not state['levels']['channels'] and not state['levels']['mixes'],state['levels']
            helper.stdin.write('{"quit":true}\n');helper.stdin.flush();helper.wait(timeout=5)
            after=info();assert after['default_sink_name']==before['default_sink_name'] and after['default_source_name']==before['default_source_name'],'Default device changed'
            report={'measured_route_fade_amplitudes':amplitudes,'peak_levels':measured['levels'],'pcm_output_gains':pcm,'muted_output_pcm':muted,'muted_mix_levels':silent['levels'],'rename_preserved_module_and_meter_ids':True,'disabled_meter_cleanup_removed_all_owned_nodes':True,'default_devices_unchanged':True,'boundary':'Generated anti-phase stereo PCM into test-owned virtual channel. Captures only owned virtual mix/source/fixture outputs; no microphone/default recording, real-app routing or physical output changes.'}
            if args.report:args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
            print('PASS: measured stereo peaks, independent output gains/mute, rename identity, disabled cleanup and unchanged defaults.')
        finally:
            for child in children:stop(child)
            if helper.poll() is None:
                try:helper.stdin.write('{"quit":true}\n');helper.stdin.flush();helper.wait(timeout=5)
                except (BrokenPipeError,subprocess.TimeoutExpired):helper.terminate();helper.wait(timeout=3)
            # Positively identify this unique fixture namespace and exact module
            # arguments before each unload; never unload another user's nodes.
            for item in reversed(own_modules()):
                actual=next((m for m in modules() if m['index']==item['index']),None)
                if actual and actual['name']==item['name'] and actual.get('argument')==item.get('argument'):pulse('unload-module',str(item['index']))
            for ident,argument in reversed(list(fixture_modules.items())):
                actual=next((m for m in modules() if m['index']==ident),None)
                if actual and actual['name']=='module-null-sink' and actual.get('argument')==argument:pulse('unload-module',str(ident))

if __name__=='__main__':main()
