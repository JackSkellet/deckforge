#!/usr/bin/env python3
"""Real host: switch state/lifecycle and exact-path closing of an owned process."""
import argparse,json,shutil,subprocess,tempfile,time
from pathlib import Path
from system_interop import Host
from sample_interop import require,slot

PREFIX='com.elgato.streamdeck.system.'
def assign(host,name,index=0):
 host.post({'command':'assign','controller':'Keypad','index':index,'uuid':PREFIX+name})
 return slot(host.state(),PREFIX+name)
def press(host,index=0):
 host.post({'command':'input','event':'keyDown','index':index})
 host.post({'command':'input','event':'keyUp','index':index})
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/debug/deckforge'));args=p.parse_args()
 with tempfile.TemporaryDirectory(prefix='deckforge-system-next-') as d:
  root=Path(d);h=Host(args.binary.resolve(),[],root/'state',root/'host.log');child=None
  try:
   h.ready();s=assign(h,'hotkeyswitch');require(len(next(a for a in h.state()['actions'] if a['uuid']==PREFIX+'hotkeyswitch')['states'])==2,'Switch needs two editable states')
   press(h);h.wait(lambda s:slot(s,PREFIX+'hotkeyswitch')['state']==1,'switch enters second state')
   h.post({'command':'input','event':'keyUp','index':0});require(slot(h.state(),PREFIX+'hotkeyswitch')['state']==1,'stray release toggled switch')
   press(h);h.wait(lambda s:slot(s,PREFIX+'hotkeyswitch')['state']==0,'switch returns to first state')
   h.post({'command':'input','event':'keyDown','index':0})
   h.post({'command':'page_create','name':'Away'})
   h.post({'command':'input','event':'keyUp','index':0})
   h.post({'command':'page_step','direction':-1})
   require(slot(h.state(),PREFIX+'hotkeyswitch')['state']==0,'Cancelled hold changed switch state')
   text=assign(h,'text',1)
   settings={'pastedText':'Hei 🌍\næøå','isSendingEnter':True,'isTypingMode':False,'Future':{'keep':True}}
   h.post({'command':'settings','context':text['context'],'settings':settings})
   require(slot(h.state(),PREFIX+'text')['settings']==settings,'Unicode text settings lost')
   target=root/'owned application';shutil.copy2('/usr/bin/sleep',target);child=subprocess.Popen([str(target),'30'])
   close=assign(h,'close',2);h.post({'command':'settings','context':close['context'],'settings':{'path':str(target),'forceQuit':False,'linuxCloseMethod':'terminate'}})
   press(h,2);h.wait(lambda s:'SIGTERM requested' in s['system_status'],'exact executable close')
   child.wait(timeout=3);require(child.returncode==-15,'Close did not request SIGTERM')
   timer=assign(h,'timer',3)
   h.post({'command':'settings','context':timer['context'],'settings':{'duration':1,'actionIdx':6}})
   press(h,3);h.wait(lambda s:slot(s,PREFIX+'timer')['feedback'].get('timerRunning') is True,'timer starts')
   h.wait(lambda s:slot(s,PREFIX+'timer')['feedback'].get('timerTitle')=='Done','timer completes without sound')
   h.post({'command':'settings','context':timer['context'],'settings':{'duration':2,'actionIdx':6}})
   press(h,3);press(h,3)
   h.wait(lambda s:slot(s,PREFIX+'timer')['feedback'].get('timerRunning') is False,'timer cancellation')
   require(slot(h.state(),PREFIX+'timer')['feedback']['timerTitle']=='0:02','Timer did not reset')
   original_page=h.state()['library']['active_page']
   press(h,3);h.post({'command':'page_create','name':'Timer away'})
   h.wait(lambda s:s['system_status']=='Timer complete','timer continues off page')
   h.post({'command':'page_select','id':original_page})
   require(slot(h.state(),PREFIX+'timer')['feedback']['timerTitle']=='Done','Hidden timer completion lost')
  finally:
   if child and child.poll() is None:child.terminate();child.wait()
   h.stop()
 print('PASS: Hotkey Switch state and release ownership, Unicode settings, exact executable Close, Timer start/cancel/completion and hidden-page countdown through the real host. No user keys or clipboard sent.')
if __name__=='__main__':main()
