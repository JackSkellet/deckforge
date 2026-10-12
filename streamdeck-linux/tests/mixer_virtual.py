#!/usr/bin/env python3
"""Generated PCM through owned virtual mixer nodes only; explicit opt-in required.
No microphone capture, physical output routing, default-device changes, or real app moves.
"""
import argparse, array, json, math, os, pathlib, selectors, subprocess, tempfile, threading, time, uuid

def pulse(*args):
    return subprocess.check_output(['pactl',*args],text=True)
def info():
    return json.loads(pulse('--format=json','info'))
def sink_inputs():
    return json.loads(pulse('--format=json','list','sink-inputs'))
def collect(records,seconds=.6):
    sel=selectors.DefaultSelector();buffers={k:bytearray() for k in records}
    for key,p in records.items():os.set_blocking(p.stdout.fileno(),False);sel.register(p.stdout,selectors.EVENT_READ,key)
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        for key,_ in sel.select(.02):
            chunk=os.read(key.fileobj.fileno(),65536)
            buffers[key.data].extend(chunk)
            if len(buffers[key.data])>1024*1024:del buffers[key.data][:-1024*1024]
    sel.close()
    result={}
    for key,data in buffers.items():
        usable=len(data)//8*8;samples=array.array('f');samples.frombytes(data[:usable]);mono=list(samples)[::2][-12000:]
        assert len(mono)>4000,(key,'no virtual PCM')
        rms=math.sqrt(sum(v*v for v in mono)/len(mono))
        def level(freq):
            x=sum(v*math.cos(2*math.pi*freq*n/48000) for n,v in enumerate(mono));y=sum(v*math.sin(2*math.pi*freq*n/48000) for n,v in enumerate(mono));return 2*math.hypot(x,y)/len(mono)
        result[key]={'rms':rms,'440hz':level(440),'880hz':level(880)}
    return result

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--probe-binary');ap.add_argument('--run-virtual',action='store_true');args=ap.parse_args()
    if not args.run_virtual:ap.error('--run-virtual explicitly permits only this test-owned audio graph')
    harness=None
    if not args.probe_binary:
        harness=tempfile.TemporaryDirectory(prefix='deckforge-mixer-probe-');root=pathlib.Path(harness.name);(root/'src').mkdir()
        (root/'Cargo.toml').write_text('[package]\nname="deckforge-mixer-probe"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nanyhow="1"\nserde={version="1",features=["derive"]}\nserde_json="1"\nlibpulse-binding="2.30"\nlibc="0.2"\nuuid={version="1",features=["v4"]}\n')
        module=pathlib.Path(__file__).resolve().parents[1]/'src/mixer.rs'
        (root/'src/main.rs').write_text('#[path='+json.dumps(str(module))+'] mod mixer;\n'+PROBE_MAIN)
        subprocess.run(['cargo','build','--offline','--quiet','--manifest-path',str(root/'Cargo.toml')],check=True)
        args.probe_binary=str(root/'target/debug/deckforge-mixer-probe')
    ns='mtest'+uuid.uuid4().hex[:10];before=info();children=[];writers=[];modules={};report={};fixture=None;fixtures=set()
    journal_state=tempfile.TemporaryDirectory(prefix='deckforge-mixer-state-')
    helper_env={**os.environ,'XDG_STATE_HOME':journal_state.name}
    def start_helper():return subprocess.Popen([args.probe_binary,ns],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=helper_env)
    helper=start_helper()
    def request(config=None):
        helper.stdin.write(json.dumps({'config':config} if config is not None else {})+'\n');helper.stdin.flush()
        ready=selectors.DefaultSelector();ready.register(helper.stdout,selectors.EVENT_READ)
        try:assert ready.select(15),'mixer helper timeout';line=helper.stdout.readline()
        finally:ready.close()
        assert line,helper.stderr.read();return json.loads(line)
    def restart_disabled():
        nonlocal helper
        helper.stdin.write('{"quit":true}\n');helper.stdin.flush();helper.wait(timeout=5)
        helper=start_helper();deadline=time.monotonic()+3
        while True:
            snapshot=request()
            if snapshot['connected']:break
            assert time.monotonic()<deadline,'disabled restart did not connect';time.sleep(.05)
        assert not snapshot['active'] and not snapshot['config']['enabled'] and snapshot['error'] is None
        return snapshot
    def configure(config):
        s=request(config);assert s['error'] is None,s['error'];assert s['connected'];return s
    def record(device):
        assert device.startswith('deckforge.'+ns+'.') or device in {name+'.monitor'for name in fixtures}
        p=subprocess.Popen(['pacat','--record','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+device,'--client-name=Deckforge matrix test capture'],stdout=subprocess.PIPE,stderr=subprocess.PIPE);children.append(p);return p
    def play(device,freq,app_id=None):
        assert device.startswith('deckforge.'+ns+'.') or device in fixtures
        cmd=['pacat','--playback','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+device,'--client-name=Deckforge matrix test generator']
        if app_id:cmd+=['--property=application.id='+app_id]
        p=subprocess.Popen(cmd,stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE);children.append(p)
        def feed():
            phase=0
            try:
                while p.poll() is None:
                    block=array.array('f')
                    for _ in range(480):
                        v=.2*math.sin(2*math.pi*freq*phase/48000);block.extend((v,v));phase+=1
                    p.stdin.write(block.tobytes());p.stdin.flush()
            except (BrokenPipeError,ValueError):pass
            finally:
                try:p.stdin.close()
                except (BrokenPipeError,ValueError):pass
        t=threading.Thread(target=feed,daemon=True);t.start();writers.append((p,t));return p
    def stop(p):
        if p.poll() is None:p.terminate()
        try:p.wait(timeout=3)
        except subprocess.TimeoutExpired:p.kill();p.wait(timeout=3)
    def make_sink(name):
        argument='sink_name='+json.dumps(name)+' rate=48000 channels=2'
        idx=int(pulse('load-module','module-null-sink',argument));modules[idx]=argument;fixtures.add(name);return idx
    def drop_sink(idx):
        actual=next((line.split('\t') for line in pulse('list','short','modules').splitlines() if line.split('\t')[0]==str(idx)),None)
        if actual and actual[1]=='module-null-sink' and actual[2]==modules[idx]:pulse('unload-module',str(idx))
        modules.pop(idx,None)
    def routed_to(app_id):
        streams=[x for x in sink_inputs()if x.get('properties',{}).get('application.id')==app_id];assert len(streams)==1
        sinks={x['index']:x['name']for x in json.loads(pulse('--format=json','list','sinks'))};return streams[0]['index'],sinks[streams[0]['sink']]
    c={'enabled':True,'channels':[{'id':'a','name':'Tone A','apps':[],'sources':[]},{'id':'b','name':'Tone B','apps':[],'sources':[]}],'mixes':[{'id':'one','name':'Mix one','outputs':[],'volume':100.,'muted':False},{'id':'two','name':'Mix two','outputs':[],'volume':100.,'muted':False}],'routes':[{'channel':'a','mix':'one','volume':50.,'muted':False},{'channel':'b','mix':'two','volume':100.,'muted':False}]}
    try:
        s=configure(c);assert s['active'];assert sum(d['owned'] for d in s['sinks'])==4
        records={'one':record(f'deckforge.{ns}.source.one'),'two':record(f'deckforge.{ns}.source.two')}
        a=play(f'deckforge.{ns}.channel.a',440);b=play(f'deckforge.{ns}.channel.b',880)
        collect(records,.35);initial=collect(records)
        assert .4<initial['one']['rms']/initial['two']['rms']<.6,initial
        assert initial['one']['880hz']<.008 and initial['two']['440hz']<.008,initial
        c['routes'][0]['muted']=True;configure(c);collect(records,.25);muted=collect(records)
        assert muted['one']['rms']<.003 and muted['two']['rms']>.10,muted
        c['routes'][0].update(muted=False,volume=25.);configure(c);collect(records,.25);quarter=collect(records)
        assert .18<quarter['one']['rms']/quarter['two']['rms']<.32,quarter
        c['mixes'][1]['muted']=True;configure(c);collect(records,.25);master=collect(records)
        assert master['two']['rms']<.003 and master['one']['rms']>.02,master
        for p in [a,b,*records.values()]:stop(p)
        for p,t in writers:t.join(timeout=2)
        source='deckforge_fixture_source_'+ns;make_sink(source)
        spaced='Deckforge fixture output '+ns;spaced_id=make_sink(spaced)
        c['channels'][0]['sources']=[source+'.monitor'];c['mixes'][0]['outputs']=[spaced];c['routes'][0]['volume']=50.
        configure(c)
        routed_records={'mix':record(f'deckforge.{ns}.source.one'),'quoted_output':record(spaced+'.monitor')}
        source_player=play(source,440);collect(routed_records,.35);source_output=collect(routed_records)
        assert .08<source_output['mix']['440hz']<.12,source_output
        assert .08<source_output['quoted_output']['440hz']<.12,source_output
        for p in [source_player,*routed_records.values()]:stop(p)
        c['channels'][0]['sources']=[];c['mixes'][0]['outputs']=[];configure(c)
        # This independent fixture sink is the known original destination of only
        # our generated app stream. Mixer Disable must restore it exactly.
        fixture='deckforge_fixture_'+ns
        make_sink(fixture)
        app_id='org.deckforge.fixture.'+ns;app=play(fixture,440,app_id);time.sleep(.15)
        c['channels'][0]['apps']=[{'field':'application_id','value':app_id}];configure(c)
        route=[x for x in sink_inputs()if x.get('properties',{}).get('application.id')==app_id];assert len(route)==1
        sinks=json.loads(pulse('--format=json','list','sinks'));sink_by_id={x['index']:x['name']for x in sinks}
        assert sink_by_id[route[0]['sink']]==f'deckforge.{ns}.channel.a'
        c['enabled']=False;configure(c)
        route=[x for x in sink_inputs()if x.get('properties',{}).get('application.id')==app_id]
        sinks=json.loads(pulse('--format=json','list','sinks'));sink_by_id={x['index']:x['name']for x in sinks}
        assert sink_by_id[route[0]['sink']]==fixture
        # A route changed outside the mixer remains owned by the user's choice.
        c['enabled']=True;configure(c);index,target=routed_to(app_id);assert target==f'deckforge.{ns}.channel.a'
        pulse('move-sink-input',str(index),spaced);time.sleep(.7);assert routed_to(app_id)[1]==spaced
        c['enabled']=False;configure(c);assert routed_to(app_id)[1]==spaced
        # Missing original endpoints park only this generated stream, never a default.
        c['enabled']=True;configure(c);assert routed_to(app_id)[1]==f'deckforge.{ns}.channel.a'
        drop_sink(spaced_id);c['enabled']=False;parked=configure(c)
        assert routed_to(app_id)[1]==f'deckforge.{ns}.channel.a';assert parked['warnings']
        restart_disabled();assert routed_to(app_id)[1]==f'deckforge.{ns}.channel.a'
        spaced_id=make_sink(spaced);time.sleep(.65)
        assert routed_to(app_id)[1]==f'deckforge.{ns}.channel.a','disabled restart must not restore or route automatically'
        c['enabled']=True;configure(c);c['enabled']=False;configure(c)
        deadline=time.monotonic()+3
        while routed_to(app_id)[1]!=spaced and time.monotonic()<deadline:time.sleep(.1)
        assert routed_to(app_id)[1]==spaced
        stop(app);time.sleep(.6)
        final=request();assert not any(d['name'].startswith('deckforge.'+ns+'.')for d in final['sinks'])
        # Keep an external reader attached across Disable: it must receive silence,
        # not be migrated to a system microphone when its virtual source vanishes.
        c['channels'][0]['apps']=[];c['enabled']=True;configure(c)
        reader=record(f'deckforge.{ns}.source.one');collect({'reader':reader},.3)
        c['enabled']=False;retained=configure(c);assert retained['warnings']
        assert any(d['name']==f'deckforge.{ns}.source.one' for d in retained['sources'])
        silence=collect({'reader':reader},.4);assert silence['reader']['rms']<.00001,silence
        source_name=f'deckforge.{ns}.source.one'
        source_index=next(x['index']for x in json.loads(pulse('--format=json','list','sources'))if x['name']==source_name)
        restart_disabled();assert collect({'reader':reader},.3)['reader']['rms']<.00001
        c['enabled']=True;configure(c)
        assert next(x['index']for x in json.loads(pulse('--format=json','list','sources'))if x['name']==source_name)==source_index,'recovery must preserve the recorder endpoint'
        resumed=play(f'deckforge.{ns}.channel.a',440);collect({'reader':reader},.3);recovered_pcm=collect({'reader':reader})
        assert .08<recovered_pcm['reader']['440hz']<.12,recovered_pcm
        stop(resumed);c['enabled']=False;configure(c)
        stop(reader);time.sleep(.65)
        final=request();assert not any(d['name'].startswith('deckforge.'+ns+'.')for d in final['sinks'])
        after=info();assert before['default_sink_name']==after['default_sink_name'];assert before['default_source_name']==after['default_source_name']
        report={'passed':True,'namespace':ns,'linear_gain_50':initial,'route_mute':muted,'linear_gain_25':quarter,'mix_mute':master,'source_and_quoted_output':source_output,'external_mix_reader_retained_silent':True,'restart_preserved_endpoint_and_pcm':recovered_pcm,'restart_preserved_app_restore':True,'disabled_restart_audio_read_only':True,'app_restored':True,'external_move_respected':True,'missing_original_parked_and_restored':True,'default_devices_unchanged':True,'scope':'Generated PCM and owned virtual endpoints only; no actual app/mic/speaker routes.'}
        print(json.dumps(report,indent=2))
    finally:
        for p in children:stop(p)
        for p,t in writers:t.join(timeout=2)
        if helper.poll() is None:
            try:helper.stdin.write('{"quit":true}\n');helper.stdin.flush();helper.wait(timeout=5)
            except (BrokenPipeError,subprocess.TimeoutExpired):helper.kill();helper.wait(timeout=3)
        for module in list(reversed(modules)):drop_sink(module)
        # Assert cleanup even after failure. Never unload by broad namespace search.
        remaining=[line for line in pulse('list','short','sinks').splitlines()if 'deckforge.'+ns+'.' in line]
        assert not remaining,('owned test graph leaked',remaining)
PROBE_MAIN=r'''
use mixer::{Config,Worker};
use std::{io::{self,BufRead},time::{Instant,Duration},thread};
fn main(){let ns=std::env::args().nth(1).expect("namespace");let worker=Worker::new(ns,Config::default()).unwrap();for line in io::stdin().lock().lines(){let value:serde_json::Value=serde_json::from_str(&line.unwrap()).unwrap();if value["quit"]==true {break;}if let Some(c)=value.get("config"){let config:Config=serde_json::from_value(c.clone()).unwrap();worker.submit(config.clone()).unwrap();let end=Instant::now()+Duration::from_secs(12);while worker.snapshot().config!=config && Instant::now()<end {thread::sleep(Duration::from_millis(20));}}let _=worker.drain();println!("{}",serde_json::to_string(&worker.snapshot()).unwrap());}}
'''
if __name__=='__main__':main()
