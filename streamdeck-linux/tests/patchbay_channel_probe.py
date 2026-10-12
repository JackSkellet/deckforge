#!/usr/bin/env python3
"""Verify PipeWire per-channel connections on a private server without hardware."""
import argparse
import array
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    report = {'ok': False, 'physical_audio_access': False}
    with tempfile.TemporaryDirectory(prefix='deckforge-port-probe-') as temporary:
        root = Path(temporary)
        with private_audio(root) as env:
            children = []
            def pulse(*argv):
                return subprocess.check_output(['pactl', *argv], env=env, text=True).strip()
            def play(name, left, right):
                child = subprocess.Popen(['pacat','--playback','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+name],env=env,stdin=subprocess.PIPE,stderr=subprocess.DEVNULL)
                children.append(child)
                def feed():
                    n=0
                    try:
                        while child.poll() is None:
                            block=array.array('f')
                            for _ in range(480):
                                block.extend((.1*math.sin(2*math.pi*left*n/48000),.1*math.sin(2*math.pi*right*n/48000)));n+=1
                            child.stdin.write(block.tobytes());child.stdin.flush()
                    except (BrokenPipeError,ValueError):pass
                threading.Thread(target=feed,daemon=True).start()
            def record(name):
                child=subprocess.Popen(['pacat','--record','--raw','--format=float32le','--rate=48000','--channels=2','--latency-msec=20','--device='+name+'.monitor'],env=env,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
                children.append(child);return child
            def connect(source,destination,source_channel,destination_channel):
                names={'FL':'front-left','FR':'front-right'}
                remap=f'remap_{source}_{destination}_{source_channel}_{destination_channel}'
                pulse('load-module','module-remap-source',f'master={source}.monitor source_name={remap} master_channel_map={names[source_channel]} channel_map={names[destination_channel]} channels=1 remix=no')
                return pulse('load-module','module-loopback',f'source={remap} sink={destination} channels=1 channel_map={names[destination_channel]} remix=false source_dont_move=true sink_dont_move=true latency_msec=30')

            def measure(recorders,seconds=1.0):
                selector=selectors.DefaultSelector();buffers={name:bytearray() for name in recorders}
                for name,child in recorders.items():
                    os.set_blocking(child.stdout.fileno(),False);selector.register(child.stdout,selectors.EVENT_READ,name)
                until=time.monotonic()+seconds
                while time.monotonic()<until:
                    for key,_ in selector.select(.03):
                        data=os.read(key.fileobj.fileno(),32768);assert data
                        buffers[key.data].extend(data)
                selector.close();result={}
                for name,data in buffers.items():
                    samples=array.array('f');samples.frombytes(data[:len(data)//8*8]);assert len(samples)>16000
                    result[name]={}
                    for ch,label in [(0,'left'),(1,'right')]:
                        values=list(samples)[ch::2][-12000:]
                        result[name][label]={}
                        for hz in (330,440,660,880):
                            re=sum(v*math.cos(2*math.pi*hz*n/48000) for n,v in enumerate(values))
                            im=sum(v*math.sin(2*math.pi*hz*n/48000) for n,v in enumerate(values))
                            result[name][label][str(hz)]=2*math.hypot(re,im)/len(values)
                return result
            try:
                for name in ['source_a','source_b','headphones','spare']:
                    pulse('load-module','module-null-sink',f'sink_name={name} channels=2 channel_map=front-left,front-right rate=48000')
                play('source_a',440,880);play('source_b',330,660)
                connect('source_a','headphones','FL','FR')
                recorders={name:record(name) for name in ['headphones','spare','source_a']}
                first=measure(recorders)
                assert first['headphones']['right']['440']>.08 and max(first['headphones']['left'].values())<.003 and first['headphones']['right']['880']<.003,first
                connect('source_b','headphones','FL','FR')
                connect('source_a','spare','FR','FL')
                second=measure(recorders)
                assert second['headphones']['right']['440']>.08 and second['headphones']['right']['330']>.08,second
                assert max(second['headphones']['left'].values())<.003,second
                assert second['spare']['left']['880']>.08 and max(second['spare']['right'].values())<.003,second
                report.update(ok=True,left_to_right=first,fan_in_and_split=second)
            except Exception as error:
                report['error']=str(error)
                report['inventory']={kind:[{key:item.get(key) for key in ['index','name','sink','source','channel_map','corked','mute']} for item in json.loads(pulse('--format=json','list',kind))] for kind in ['sinks','sink-inputs','source-outputs']}
                graph=json.loads(subprocess.check_output(['pw-dump'],env={**env,'PIPEWIRE_CONFIG_DIR':'/usr/share/pipewire'},text=True))
                report['graph']=[{'id':item['id'],'type':item['type'],'state':item.get('info',{}).get('state'),'ports':{key:item.get('info',{}).get('props',{}).get(key) for key in ['node.name','node.id','port.name','audio.channel','audio.position']}} for item in graph if item['type'].endswith((':Node',':Port',':Link'))]
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
                args.report.parent.mkdir(parents=True,exist_ok=True)
                args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: isolated L→R, two sources summed into one ear, simultaneous R→L on another output.')

if __name__=='__main__':main()
