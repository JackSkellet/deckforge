#!/usr/bin/env python3
"""Exercise arbitrary typed routing through the real host on private PipeWire."""
import argparse
import array
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import threading
import time
from private_audio import private_audio
from system_interop import Host


def run(binary,root,report):
    children=[];host=None;recorders={}
    def pulse(*args):return subprocess.check_output(['pactl',*args],text=True).strip()
    def play(name,hz,app=None):
        args=['pacat','--playback','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+name,'--client-name=Patchbay generator '+str(hz)]
        if app:args+=['--property=application.id='+app]
        child=subprocess.Popen(args,stdin=subprocess.PIPE,stderr=subprocess.DEVNULL);children.append(child)
        def feed():
            n=0
            try:
                while child.poll() is None:
                    block=array.array('f')
                    for _ in range(480):
                        value=.03*math.sin(2*math.pi*hz*n/48000);block.extend((value,value));n+=1
                    child.stdin.write(block.tobytes());child.stdin.flush()
            except (BrokenPipeError,ValueError):pass
        threading.Thread(target=feed,daemon=True).start()
    def capture(seconds=.9,measure=True):
        selector=selectors.DefaultSelector();buffers={name:bytearray() for name in recorders}
        for name,child in recorders.items():
            os.set_blocking(child.stdout.fileno(),False);selector.register(child.stdout,selectors.EVENT_READ,name)
        until=time.monotonic()+seconds
        while time.monotonic()<until:
            for key,_ in selector.select(.03):
                data=os.read(key.fileobj.fileno(),32768);assert data;buffers[key.data].extend(data)
        selector.close();result={}
        if not measure:return result
        for name,data in buffers.items():
            samples=array.array('f');samples.frombytes(data[:len(data)//8*8]);assert len(samples)>16000
            result[name]={}
            for ch,label in [(0,'left'),(1,'right')]:
                values=list(samples)[ch::2][-12000:];result[name][label]={}
                for hz in [330,440,550,660,770,880]:
                    re=sum(v*math.cos(2*math.pi*hz*n/48000) for n,v in enumerate(values));im=sum(v*math.sin(2*math.pi*hz*n/48000) for n,v in enumerate(values))
                    result[name][label][str(hz)]=2*math.hypot(re,im)/len(values)
        return result
    def apply(config):
        previous=host.state()['mixer']['config']
        host.post({'command':'mixer_config','settings':config,'expected':previous})
        def check(s):
            m=s['mixer'];assert not m['error'],m['error']
            return m['active'] and not m.get('pending',False) and m.get('applied_config',{}).get('connections')==config['connections']
        host.wait(check,'patchbay graph applied')
    def cable(id,source,target,src=None,dst=None):
        return {'id':id,'from':source,'to':target,'source_channel':src,'target_channel':dst,'volume':100,'muted':False}
    def mix(id):return {'kind':'mix','id':id}
    def channel(id):return {'kind':'channel','id':id}
    def source(name):return {'kind':'source','name':name+'.monitor'}
    def output(name):return {'kind':'output','name':name}
    try:
        for name in ['input_a','input_b','input_c','input_d','headphones','spare']:
            pulse('load-module','module-null-sink',f'sink_name={name} channels=2 channel_map=front-left,front-right rate=48000')
        pulse('load-module','module-null-sink','sink_name=input_mono channels=1 channel_map=mono rate=48000')
        play('input_mono',770)
        pulse('set-default-sink','spare');pulse('set-default-source','input_a.monitor')
        deadline=time.monotonic()+3
        while True:
            defaults=json.loads(pulse('--format=json','info'))
            if defaults['default_sink_name']=='spare' and defaults['default_source_name']=='input_a.monitor':break
            assert time.monotonic()<deadline,{'default_setup_did_not_settle':defaults}
            time.sleep(.04)
        report['defaults_before']={k:defaults[k] for k in ['default_sink_name','default_source_name']}
        for name,hz in [('input_a',440),('input_b',550),('input_c',660),('input_d',880)]:play(name,hz)
        state=root/'session';host=Host(binary,[],state,root/'host.log').ready()
        config=copy.deepcopy(host.state()['mixer']['config']);config.update(enabled=True,monitoring={'mix':None,'output':None},channels=[{'id':c,'name':c,'apps':[],'sources':[]} for c in ['one','two']],routes=[],connections=[
            cable('a',source('input_a'),mix('monitor')),cable('b',source('input_b'),mix('monitor')),cable('c',source('input_c'),mix('monitor')),
            cable('d',source('input_d'),channel('one')),cable('chain',channel('one'),channel('two')),cable('to_stream',channel('two'),mix('stream')),
            cable('left',mix('monitor'),output('headphones'),'front-left','front-left'),cable('right',mix('stream'),output('headphones'),'front-right','front-right')])
        apply(config)
        for name in ['headphones','spare']:
            child=subprocess.Popen(['pacat','--record','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+name+'.monitor','--client-name=Patchbay measurement '+name],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL);children.append(child);recorders[name]=child
        capture(.4,measure=False);baseline=capture()
        for hz in ['440','550','660']:assert .022<baseline['headphones']['left'][hz]<.04,baseline
        assert baseline['headphones']['left']['880']<.003 and baseline['headphones']['right']['880']>.022,baseline
        assert max(baseline['headphones']['right'][str(hz)] for hz in [440,550,660])<.003,baseline
        # Mixes can feed other mixes, and physical output monitors can be tapped.
        config['connections'].append(cable('mix_chain',mix('monitor'),mix('stream')))
        apply(config);capture(.3,measure=False);chain=capture()
        assert chain['headphones']['right']['440']>.022 and chain['headphones']['right']['880']>.022,chain
        config['connections']=[c for c in config['connections'] if c['id']!='mix_chain']
        config['mixes'].append({'id':'relay','name':'Relay','outputs':['spare'],'volume':100,'muted':False})
        config['connections'].append(cable('monitor_tap',output('headphones'),mix('relay')))
        apply(config);capture(.3,measure=False);monitor_tap=capture()
        assert monitor_tap['spare']['left']['440']>.022 and monitor_tap['spare']['right']['880']>.022,monitor_tap
        config['connections'][-1]['from']={'kind':'recording','mix':'monitor'}
        apply(config);capture(.3,measure=False);recording_tap=capture()
        assert recording_tap['spare']['right']['440']>.022 and recording_tap['spare']['right']['880']<.003,recording_tap
        config['connections']=[c for c in config['connections'] if c['id']!='monitor_tap']
        config['mixes']=[m for m in config['mixes'] if m['id']!='relay']
        config['connections'].append(cable('mono_ear',source('input_mono'),output('spare'),'mono','front-right'))
        apply(config);capture(.3,measure=False);mono=capture()
        assert mono['spare']['right']['770']>.022 and max(mono['spare']['left'].values())<.003,mono
        config['connections']=[c for c in config['connections'] if c['id']!='mono_ear']
        # A single application capture bus feeds both mixes independently.
        app={'kind':'app','selector':{'field':'application_id','value':'org.deckforge.fanout'}}
        config['connections'] += [cable('app_a',app,mix('monitor')),cable('app_b',app,mix('stream'))]
        apply(config);play('spare',330,'org.deckforge.fanout')
        capture(.5,measure=False);fanout=capture()
        assert .022<fanout['headphones']['left']['330']<.04 and .022<fanout['headphones']['right']['330']<.04,fanout
        assert fanout['spare']['left']['330']<.003,fanout
        # Node cycles can stall PipeWire even if channel directions look acyclic.
        invalid=copy.deepcopy(host.state()['mixer']['config']);previous=copy.deepcopy(invalid)
        invalid['connections'].append(cable('self_channel',mix('monitor'),mix('monitor'),'front-left','front-right'))
        host.post({'command':'mixer_config','settings':invalid,'expected':previous},expected=400)
        assert host.state()['mixer']['config']==previous
        # Shared fixed/listening destinations must not double the same mix feed.
        config['mixes'][0]['outputs']=['spare'];config['mixes'][1]['outputs']=['spare'];config['monitoring']={'output':'spare','mix':'monitor'}
        apply(config);capture(.4,measure=False);shared=capture()
        assert .022<shared['spare']['left']['440']<.04 and .022<shared['spare']['left']['880']<.04,shared
        assert .045<shared['spare']['left']['330']<.075,shared
        config['connections'][0]['volume']=50
        apply(config);capture(.3,measure=False);trimmed=capture()
        assert .011<trimmed['headphones']['left']['440']<.020 and trimmed['headphones']['left']['550']>.022,trimmed
        previous=copy.deepcopy(host.state()['mixer']['config']);invalid=copy.deepcopy(previous)
        invalid['connections'] += [cable('feedback',mix('stream'),channel('one'))]
        host.post({'command':'mixer_config','settings':invalid,'expected':previous},expected=400)
        assert host.state()['mixer']['config']==previous
        final_defaults=json.loads(pulse('--format=json','info'))
        report['defaults_after']={k:final_defaults[k] for k in ['default_sink_name','default_source_name']}
        assert all(final_defaults[k]==defaults[k] for k in ['default_sink_name','default_source_name']),report['defaults_after']
        report.update(ok=True,three_inputs_one_mix=baseline,mix_chain=chain,output_monitor_tap=monitor_tap,recording_tap=recording_tap,mono_to_one_ear=mono,app_fanout=fanout,shared_output=shared,connection_gain=trimmed,node_self_route_rejected=True,cycle_rejected=True,defaults_unchanged=True)
    except Exception as error:
        report['error']=str(error)
        if host:report['mixer']=host.state()['mixer']
        raise
    finally:
        for child in children:
            if child.poll() is None:child.terminate()
        for child in children:
            try:child.wait(timeout=3)
            except subprocess.TimeoutExpired:child.kill();child.wait()
            for pipe in (child.stdin,child.stdout):
                if pipe:
                    try:pipe.close()
                    except BrokenPipeError:pass
        if host:host.stop()


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--report',type=Path,required=True);args=parser.parse_args()
    binary=args.binary.resolve(strict=True);report={'ok':False,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'physical_audio_access':False}
    try:
        with tempfile.TemporaryDirectory(prefix='deckforge-patchbay-host-') as temporary:
            root=Path(temporary)
            with private_audio(root) as env:
                old=dict(os.environ);os.environ.update(env)
                try:run(binary,root,report)
                finally:os.environ.clear();os.environ.update(old)
    finally:args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: three inputs per mix, chained channels, per-ear outputs, app fan-out, shared listening/output and cycle rejection.')

if __name__=='__main__':main()
