#!/usr/bin/env python3
"""Retain an attached legacy recording input while advertising the corrected one.

Both builds run only on a private hardware-free PipeWire/Pulse server. The old
recorder binds by numeric source ID, avoiding PipeWire's reserved name suffix.
"""
import argparse
import array
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time

from mixer_listening_interop import capture, inventory, pulse, stop
from private_audio import private_audio
from system_interop import Host


def run(args, root):
    state = root / 'session'
    state.mkdir()
    namespace = hashlib.sha256(os.fsencode(state.resolve())).hexdigest()[:24]
    prefix = 'deckforge.' + namespace + '.'
    old_name, new_name = prefix+'source.monitor', prefix+'source.monitor.input'
    report = {'ok':False, 'legacy_sha256':hashlib.sha256(args.legacy_binary.read_bytes()).hexdigest(),
              'new_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'boundary':'Private server, own generator and numeric legacy reader only. No user audio or recording app is moved.'}
    children, feeders = [], []
    host = None
    def recording(name, device):
        child = subprocess.Popen(['pacat','--record','--raw','--format=float32le','--rate=48000','--channels=2',
            '--latency-msec=20','--device='+str(device),'--client-name='+name],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
        children.append(child)
        return child
    def source_of(child):
        info = next(item for item in inventory('source-outputs') if
                    item.get('properties',{}).get('application.process.id')==str(child.pid))
        return info['source']
    try:
        host = Host(args.legacy_binary, [], state, root/'legacy.log').ready()
        initial = host.state()['mixer']['config']
        config = copy.deepcopy(initial)
        config.update(enabled=True,channels=[{'id':'tone','name':'Tone','apps':[],'sources':[]}],
            routes=[{'channel':'tone','mix':'monitor','volume':100,'muted':False}])
        host.post({'command':'mixer_config','settings':config,'expected':initial})
        host.wait(lambda s:s['mixer']['active'] and not s['mixer']['pending'], 'legacy graph')
        source = next(item for item in inventory('sources') if item['name']==old_name)
        original_index = source['index']
        generator = subprocess.Popen(['pacat','--playback','--raw','--format=float32le','--rate=48000','--channels=2',
            '--latency-msec=20','--device='+prefix+'channel.tone','--client-name=Deckforge migration tone'],stdin=subprocess.PIPE,stderr=subprocess.DEVNULL)
        children.append(generator)
        def feed():
            phase=0
            try:
                while generator.poll() is None:
                    samples=array.array('f')
                    for _ in range(480):
                        value=.2*math.sin(2*math.pi*440*phase/48000)
                        samples.extend((value,value));phase+=1
                    generator.stdin.write(samples.tobytes());generator.stdin.flush()
            except (BrokenPipeError,ValueError):pass
        thread=threading.Thread(target=feed,daemon=True);thread.start();feeders.append(thread)
        legacy=recording('Deckforge legacy numeric recorder', original_index)
        capture({'legacy':legacy},.4)
        original=capture({'legacy':legacy},.4)
        assert original['legacy']['440hz']>.17,original
        assert source_of(legacy)==original_index
        host.stop();host=None
        assert any(item['name']==old_name and item['index']==original_index for item in inventory('sources'))
        parked=capture({'legacy':legacy},.4)
        assert parked['legacy']['rms']<.002,parked
        host=Host(args.binary,[],state,root/'updated.log').ready()
        restored=host.wait(lambda s:s['mixer']['active'] and not s['mixer']['pending'] and
            s['mixer'].get('recording_sources',{}).get('monitor')==new_name,'corrected export alongside legacy')['mixer']
        assert not restored['error'],restored['error']
        assert source_of(legacy)==original_index,'Existing recorder was retargeted'
        current=recording('Deckforge corrected named recorder',new_name)
        capture({'legacy':legacy,'new':current},.4)
        both=capture({'legacy':legacy,'new':current},.5)
        assert both['legacy']['440hz']>.17 and both['new']['440hz']>.17,both
        new_index=next(item['index'] for item in inventory('sources') if item['name']==new_name)
        assert source_of(current)==new_index and new_index!=original_index
        stop(legacy)
        continuation=capture({'new':current},.9)
        host.wait(lambda s:all(source['name']!=old_name for source in s['mixer']['sources']), 'legacy export retired after reader releases it')
        assert continuation['new']['minimum_10ms_rms']>.08,continuation
        assert source_of(current)==new_index
        for child in children:stop(child)
        for thread in feeders:thread.join(timeout=1)
        host.stop();host=None
        assert not any(item['name'].startswith(prefix) for item in inventory('sources'))
        report.update(ok=True,legacy_numeric_binding=True,legacy_parked_silent=True,
            original_source_index_preserved=True,corrected_named_binding=True,both_inputs=both,
            legacy_removed_after_release=True,continued_capture=continuation,cleanup=True)
    except Exception as error:
        report['error']=str(error)
        report['mixer']=host.state()['mixer'] if host else None
        report['sources']=inventory('sources')
        report['captures']=inventory('source-outputs')
        raise
    finally:
        for child in children:stop(child)
        if host:host.stop()
        args.report.parent.mkdir(parents=True,exist_ok=True)
        args.report.write_text(json.dumps(report,indent=2)+'\n')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--legacy-binary',type=Path,required=True)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    args.binary=args.binary.resolve(strict=True);args.legacy_binary=args.legacy_binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-recording-migration-') as directory:
        root=Path(directory)
        with private_audio(root) as env:
            previous=dict(os.environ);os.environ.update(env)
            try:run(args,root)
            finally:os.environ.clear();os.environ.update(previous)
    print('PASS: legacy recorder retained without retargeting; corrected named input works; retired alias cleanup is continuous.')

if __name__=='__main__':main()
