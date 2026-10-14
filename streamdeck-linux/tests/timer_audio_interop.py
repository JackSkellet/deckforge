#!/usr/bin/env python3
"""Timer alarms through the real output worker into an owned virtual sink."""
import argparse,tempfile,uuid,json
from pathlib import Path
from soundboard_interop import pactl,wav,capture,active,configure
from system_interop import Host
from sample_interop import slot,require,tap
TIMER='com.elgato.streamdeck.system.timer';PLAY='com.elgato.streamdeck.soundboard.playaudio'
def press(h,index):
 for event in ['keyDown','keyUp']:h.post({'command':'input','index':index,'event':event})
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/debug/deckforge'));a=p.parse_args()
 original=pactl('get-default-sink');sink='deckforge_timer_'+uuid.uuid4().hex;module=pactl('load-module','module-null-sink','sink_name='+sink,'rate=48000','channels=2')
 try:
  if pactl('get-default-sink')!=original:pactl('set-default-sink',original)
  with tempfile.TemporaryDirectory(prefix='deckforge-timer-audio-') as d:
   root=Path(d);h=Host(a.binary.resolve(),[],root/'state',root/'host.log')
   try:
    h.ready();h.post({'command':'assign','controller':'Keypad','index':0,'uuid':TIMER});context=slot(h.state(),TIMER)['context']
    h.post({'command':'settings','context':context,'settings':{'duration':1,'actionIdx':0,'linuxOutput':sink}})
    pcm=capture(h,sink,root/'alarm.pcm');require(max(pcm)>.015,'Generated timer alarm produced no real PCM')
    custom=root/'custom.wav';wav(custom,10)
    h.post({'command':'assign','controller':'Keypad','index':1,'uuid':PLAY});other=slot(h.state(),PLAY)['context'];configure(h,other,custom,sink,mode=3)
    press(h,1);h.wait(lambda s:active(s)==1,'unrelated soundboard loop')
    h.post({'command':'settings','context':context,'settings':{'duration':1,'actionIdx':4,'lastUserFile':str(custom),'linuxOutput':sink}})
    tap(h);h.wait(lambda s:active(s)==2,'timer and unrelated loop both playing')
    tap(h);h.wait(lambda s:active(s)==1,'timer sound stopped without other voice')
    tap(h);require(active(h.state())==1,'Timer cancellation stopped unrelated sound')
   finally:h.stop()
 finally:
  pactl('unload-module',module)
  if pactl('get-default-sink')!=original:pactl('set-default-sink',original)
 print('PASS: Timer generated-alarm PCM, custom alarm, and scoped stop through the real Pulse/PipeWire worker. Speakers and microphone were not used.')
if __name__=='__main__':main()
