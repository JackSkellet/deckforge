#!/usr/bin/env python3
"""Real Rust soundboard output captured from an owned virtual Pulse/PipeWire sink.
No speaker, microphone, USB, default-volume or third-party stream is touched.
"""
import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import time
import uuid
import wave
from system_interop import Host
from sample_interop import require, tap, slot
PLAY='com.elgato.streamdeck.soundboard.playaudio'
STOP='com.elgato.streamdeck.soundboard.stopaudioplay'

def pactl(*args):
    return subprocess.check_output(['pactl',*args],text=True,timeout=8).strip()

def wav(path,seconds=1.5):
    samples=[int(8192*math.sin(2*math.pi*440*i/48000)) for i in range(int(48000*seconds))]
    with wave.open(str(path),'wb') as output:
        output.setnchannels(1);output.setsampwidth(2);output.setframerate(48000)
        output.writeframes(struct.pack('<'+'h'*len(samples),*samples))

def configure(host,context,path,sink,mode=1,volume=50,fade=0):
    host.post({'command':'settings','context':context,'settings':{'path':str(path),'linuxOutput':sink,'outputType':'','actionType':mode,'volume':volume,'fadeType':fade,'fadeLen':1}})

def active(s):return sum(s.get('audio_active',{}).values())

def capture(host,sink,path):
    with path.open('wb') as output:
        process=subprocess.Popen(['parec','--device='+sink+'.monitor','--format=float32le','--rate=48000','--channels=2','--latency-msec=10'],stdout=output,stderr=subprocess.PIPE)
        try:
            time.sleep(.15);tap(host)
            host.wait(lambda s:active(s)>0,'audio voice started')
            host.wait(lambda s:active(s)==0,'natural audio completion')
            time.sleep(.15)
        finally:
            process.terminate();process.wait(timeout=5)
    samples=array.array('f');data=path.read_bytes();samples.frombytes(data[:len(data)//4*4])
    require(len(samples)>4800,'Monitor returned no PCM frames')
    require(all(math.isfinite(x) for x in samples),'Nonfinite audio samples')
    windows=[]
    for start in range(0,len(samples)-960,960):
        windows.append(math.sqrt(sum(x*x for x in samples[start:start+960])/960))
    return windows

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    parser.add_argument('--evidence',type=Path)
    args=parser.parse_args()
    for tool in ['pactl','parec']:require(shutil.which(tool),tool+' is required for actual audio verification')
    original=pactl('get-default-sink')
    sink='deckforge_test_'+uuid.uuid4().hex
    module=pactl('load-module','module-null-sink','sink_name='+sink,'rate=48000','channels=2','sink_properties=device.description=Deckforge_isolated_test')
    try:
        # Keep the prior selection if a desktop's automatic-device policy changed it.
        if pactl('get-default-sink')!=original:pactl('set-default-sink',original)
        with tempfile.TemporaryDirectory(prefix='deckforge-audio-') as directory:
            root=Path(directory);tone=root/'tone.wav';wav(tone)
            host=Host(args.binary.resolve(),[],root/'state',root/'host.log')
            try:
                host.ready();host.post({'command':'soundboard_refresh'})
                host.wait(lambda s:any(o[0]==sink for o in s['audio_outputs']),'real output enumeration')
                host.post({'command':'assign','controller':'Keypad','index':0,'uuid':PLAY})
                context=slot(host.state(),PLAY)['context']
                configure(host,context,tone,sink,volume=100)
                full=capture(host,sink,root/'full.pcm')
                full_peak=max(full);require(.14<full_peak<.20,f'Unexpected decoded PCM level: {full_peak}')
                configure(host,context,tone,sink,volume=50)
                half=capture(host,sink,root/'half.pcm')
                require(.47<max(half)/full_peak<.53,'Volume did not halve decoded amplitude')
                # Fade-in and natural fade-out are measured in the actual monitor.
                long=root/'long.wav';wav(long,3)
                configure(host,context,long,sink,volume=100,fade=3)
                faded=capture(host,sink,root/'faded.pcm')
                audible=[x for x in faded if x>full_peak*.01]
                require(len(audible)>150,'Faded clip truncated')
                require(sum(audible[:20])/20 < full_peak*.25,'Fade-in onset was abrupt')
                require(sum(audible[-20:])/20 < full_peak*.25,'Natural fade-out end was abrupt')
                configure(host,context,long,sink,mode=0)
                tap(host);tap(host)
                host.wait(lambda s:active(s)==2,'overlapping voices')
                host.post({'command':'soundboard_stop'});host.wait(lambda s:active(s)==0,'stop all')
                configure(host,context,long,sink,mode=2)
                tap(host);host.wait(lambda s:active(s)==1,'restart source began');tap(host)
                require(active(host.state())==1,'Restart added overlap')
                # Invalid replacement must not kill the original voice.
                configure(host,context,root/'missing.wav',sink,mode=2)
                tap(host)
                host.wait(lambda s:'failed' in s['system_status'].lower(),'missing file error')
                require(active(host.state())==1,'Failed replacement destroyed playing voice')
                host.post({'command':'soundboard_stop'});host.wait(lambda s:active(s)==0,'clear prior voice')
                short=root/'short.wav';wav(short,.15)
                configure(host,context,short,sink,mode=3)
                tap(host);host.wait(lambda s:active(s)==1,'loop began');time.sleep(.6)
                require(active(host.state())==1,'Loop ended after one clip')
                tap(host);host.wait(lambda s:active(s)==0,'loop toggle stop')
                configure(host,context,long,sink,mode=1)
                tap(host);host.wait(lambda s:active(s)==1,'play stop began');tap(host)
                host.wait(lambda s:active(s)==0,'play stop toggled')
                configure(host,context,long,'deckforge_missing_'+uuid.uuid4().hex)
                tap(host);host.wait(lambda s:'failed' in s['system_status'].lower(),'missing output error')
                require(active(host.state())==0,'Unavailable output fell back to another device')
                configure(host,context,short,sink,mode=3)
                tap(host);host.wait(lambda s:active(s)==1,'loop before builtin stop')
                host.post({'command':'assign','controller':'Keypad','index':1,'uuid':STOP})
                for event in ['keyDown','keyUp']:host.post({'command':'input','index':1,'event':event})
                host.wait(lambda s:active(s)==0,'builtin Stop Audio action')
                # Original wrapped Windows package, unchanged MP3 and foreign path.
                original_profile=Path(__file__).resolve().parents[2]/'evidence/elgato-7.6/raw/flsKaSPXYxUUvmKtC3TdqgYiqQcKKY'
                require(hashlib.sha256(original_profile.read_bytes()).hexdigest()=='b9fa67b7956999fabf6bc2308504cfa23bf99e744a5233a63dcfd17c0698a7b1','Official audio fixture changed')
                host.post({'command':'import_profile','path':str(original_profile)})
                snapshot=host.state()
                saved=json.loads((root/'state/session.json').read_text())
                imported=next(p for p in saved['library']['profiles'] if p['id']==snapshot['library']['active_profile'])
                page=next(p for p in imported['pages'] if any(s['uuid']==PLAY for s in p['slots']))
                host.post({'command':'page_select','id':page['id']})
                sound=slot(host.state(),PLAY)
                original_path=sound['settings']['path']
                require(not Path(original_path).exists(),'Fixture must exercise its embedded resource')
                settings=dict(sound['settings'],linuxOutput=sink)
                host.post({'command':'settings','context':sound['context'],'settings':settings})
                with (root/'original-mp3.pcm').open('wb') as output:
                    recording=subprocess.Popen(['parec','--device='+sink+'.monitor','--format=float32le','--rate=48000','--channels=2','--latency-msec=10'],stdout=output,stderr=subprocess.PIPE)
                    try:
                        time.sleep(.2)
                        for event in ['keyDown','keyUp']:host.post({'command':'input','index':sound['index'],'event':event})
                        host.wait(lambda s:active(s)>0,'embedded original MP3 playback')
                        time.sleep(1)
                        host.post({'command':'soundboard_stop'});host.wait(lambda s:active(s)==0,'stop imported audio')
                        time.sleep(.1)
                    finally:
                        recording.terminate();recording.wait(timeout=5)
                data=(root/'original-mp3.pcm').read_bytes();mp3=array.array('f');mp3.frombytes(data[:len(data)//4*4])
                require(mp3 and max(abs(x) for x in mp3)>.001,f'Imported original MP3 produced no captured audio: frames={len(mp3)//2}, peak={max((abs(x) for x in mp3),default=0)}')
                require(slot(host.state(),PLAY)['settings']['path']==original_path,'Runtime overwrote original source path')
                if args.evidence:
                    args.evidence.mkdir(parents=True,exist_ok=True)
                    for name in ['full.pcm','half.pcm','faded.pcm','original-mp3.pcm','host.log']:shutil.copy2(root/name,args.evidence/name)
                    (args.evidence/'capture-metrics.json').write_text(json.dumps({'full_rms_peak':full_peak,'half_rms_peak':max(half),'fade_first_200ms_mean':sum(audible[:20])/20,'fade_last_200ms_mean':sum(audible[-20:])/20,'sink':sink,'original_default':original},indent=2)+'\n')
            finally:
                host.stop();require(host.process.returncode==0,'Audio host failed to shut down cleanly')
        require(pactl('get-default-sink')==original,'Default audio device changed')
        print('PASS: real PipeWire/Pulse monitor PCM, decoding/resampling, volume, fades, overlap/restart/toggle/loop/stop, exact sink routing, error preservation and unchanged embedded official MP3 playback. No USB or physical audio output used.')
    finally:
        pactl('unload-module',module)

if __name__=='__main__':main()
