#!/usr/bin/env python3
"""Actual decoded keyboard packets in owned Qt clients under US, German and Norwegian KWin layouts."""
import argparse,hashlib,json,os,shlex,signal,subprocess,sys,tempfile,time
from pathlib import Path
from focused_clipboard_interop import wait
CLIENT=r'''
import json,sys
from pathlib import Path
from PySide6.QtWidgets import QApplication,QWidget
from PySide6.QtCore import QTimer
root=Path(sys.argv[1]);events=[]
class Window(QWidget):
 def keyPressEvent(self,e):
  events.append({'type':'press','key':e.key(),'scan':e.nativeScanCode(),'text':e.text(),'modifiers':e.modifiers().value,'repeat':e.isAutoRepeat()});save()
 def keyReleaseEvent(self,e):
  events.append({'type':'release','key':e.key(),'scan':e.nativeScanCode(),'text':e.text(),'modifiers':e.modifiers().value,'repeat':e.isAutoRepeat()});save()
def save():(root/'events.json').write_text(json.dumps(events))
app=QApplication([]);w=Window();w.setWindowTitle('Deckforge owned modifier test');w.resize(320,160);w.show();w.activateWindow();w.setFocus()
QTimer.singleShot(300,lambda:(root/'client-ready').write_text('yes'))
QTimer.singleShot(45000,app.quit);app.exec()
'''
def inside(root,binary,bridge,layout):
 import struct
 processes=[];reports={};held=set()
 with (root/'kwin.log').open('w') as kl,(root/'client.log').open('w') as cl,(root/'bridge.log').open('w') as bl:
  try:
   kwin=subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-private-input'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=kl,stderr=kl);processes.append(kwin);wait(root/'runtime/deckforge-private-input',processes)
   env={**os.environ,'DECKFORGE_PRIVATE_KWIN_PID':str(kwin.pid),'DECKFORGE_PRIVATE_CLIPBOARD_TEST':str(root)}
   for _ in range(100):
    if subprocess.run(['busctl','--user','status','org.kde.KWin'],capture_output=True).returncode==0:break
    time.sleep(.03)
   script=root/'client.py';script.write_text(CLIENT)
   client=subprocess.Popen([sys.executable,str(script),str(root)],env=env,stdout=cl,stderr=cl);processes.append(client)
   relay=subprocess.Popen([str(bridge),str(root)],env=env,stdout=bl,stderr=bl);processes.append(relay)
   other=root/'second-keyboard';other.mkdir()
   external=subprocess.Popen([str(bridge),str(other)],env=env,stdout=bl,stderr=bl);processes.append(external)
   wait(root/'client-ready',processes);wait(root/'bridge-ready',processes);wait(other/'bridge-ready',processes)
   def events():
    try:return json.loads((root/'events.json').read_text())
    except (OSError,ValueError):return []
   def foreign(key,down):
    if down:held.add(key)
    else:held.discard(key)
    with (other/'keys.bin').open('ab') as out:out.write(struct.pack('@llHHi',0,0,1,key,int(down))+struct.pack('@llHHi',0,0,0,0,0))
    time.sleep(.05)
   def toggle(key):foreign(key,True);foreign(key,False)
   def chord(name,qt,flags=0,**extra):
    before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':qt,'VKeyCode':-1,'KeyModifiers':flags,'linuxKeyMode':'logical',**extra}]}))
    result=subprocess.run([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,capture_output=True,text=True,timeout=5)
    assert result.returncode==0,result.stdout+result.stderr
    time.sleep(.1);got=events()[before:];assert got,(layout,name,'no events');reports[name]=got;(root/'partial.json').write_text(json.dumps(reports));return got
   def text(got,value):assert any(e['type']=='press' and e['text']==value for e in got),(layout,value,got)
   text(chord('baseline_a',65),'a')
   toggle(58);text(chord('caps_a',65),'A');text(chord('caps_shift_a',65,1),'a');text(chord('caps_digit_one',49),'1');text(chord('caps_still_on',65),'A')
   toggle(58);text(chord('caps_off',65),'a')
   kp=chord('keypad_before_numlock',55,0x4000,NativeCode=103)
   toggle(69);kp2=chord('keypad_after_numlock',55,0x4000,NativeCode=103)
   key=lambda got:next((e['key'],e['text']) for e in got if e['type']=='press' and e['scan']==79)
   assert key(kp)!=key(kp2) and any(key(g)==(55,'7') for g in [kp,kp2]),(layout,kp,kp2)
   toggle(69);assert key(chord('numlock_restored',55,0x4000,NativeCode=103))==key(kp)
   for name,code,flags,mask in [('left_ctrl',29,2,0x04000000),('right_ctrl',97,0x10202,0x04000000),('left_shift',42,1,0x02000000),('right_shift',54,0x10101,0x02000000)]:
    foreign(code,True)
    for phase,own_flags in [('inherited',0),('same_modifier',flags),('after_own_release',0)]:
     got=chord(name+'_'+phase,89,own_flags)
     assert any(e['type']=='press' and e['key']==89 and e['modifiers']&mask for e in got),(layout,name,phase,got)
    foreign(code,False)
    got=chord(name+'_released',89);assert any(e['type']=='press' and e['key']==89 and e['modifiers']==0 for e in got),(layout,name,got)
   if layout in ['de','no']:
    foreign(100,True)
    for name,flags in [('external_altgr',0),('shared_altgr',0x10404),('external_altgr_after_own_release',0)]:text(chord(name,81,flags),'@')
    foreign(100,False);text(chord('altgr_released',81),'q')
   for name,qt,flags,duration in [('held_y_repeat',89,0,1.3),('held_modifier_no_repeat',0x01ffffff,2,.8)]:
    for file in ['logical-held','logical-release']:(root/file).unlink(missing_ok=True)
    before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':qt,'VKeyCode':-1,'KeyModifiers':flags,'linuxKeyMode':'logical'}],'waitForRelease':True}))
    process=subprocess.Popen([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,stdout=cl,stderr=cl);processes.append(process);wait(root/'logical-held',processes);time.sleep(duration)
    got=events()[before:]
    if qt==89:assert any(e['type']=='press' and e['key']==89 and e['repeat'] for e in got),(layout,name,got)
    else:assert not any(e['repeat'] for e in got) and sum(e['type']=='press' for e in got)==1,(layout,name,got)
    (root/'logical-release').write_text('release');assert process.wait(timeout=3)==0;processes.remove(process);time.sleep(.15)
    got=events()[before:];assert sum(e['type']=='release' and not e['repeat'] for e in got)==1,(layout,name,got)
    count=len(events());time.sleep(.15);assert len(events())==count,(layout,name,'repeat continued after release')
    reports[name]=got
   (root/'report.json').write_text(json.dumps({'layout':layout,'cases':reports,'two_owned_virtual_keyboards':True,'physical_hardware_tested':False},indent=2))
  finally:
   if 'foreign' in locals():
    for key in list(held):foreign(key,False)
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)

def main():
 p=argparse.ArgumentParser();p.add_argument('--test-binary',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('target/keyboard-state'));p.add_argument('--inside',type=Path);p.add_argument('--bridge',type=Path);p.add_argument('--layout');a=p.parse_args();binary=a.test_binary.resolve()
 if a.inside:inside(a.inside,binary,a.bridge,a.layout);return
 a.output.mkdir(parents=True,exist_ok=True);bridge=a.output.resolve()/'input-bridge'
 flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','Qt6DBus','libei-1.0'],text=True));subprocess.run(['g++','-fPIC','-std=c++17',str(Path(__file__).with_name('focused_clipboard_bridge.cpp')),'-o',str(bridge),*flags],check=True)
 reports=[]
 for layout in ['us','de','no','fr']:
  with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
   root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'config').mkdir();(root/'config/kxkbrc').write_text('[Layout]\nUse=true\nLayoutList='+layout+(',de' if layout!='de' else ',us')+'\nModel=pc105\nOptions=\n')
   env=os.environ.copy();env.pop('DISPLAY',None);env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WAYLAND_DISPLAY='deckforge-private-input',GDK_BACKEND='wayland',QT_QPA_PLATFORM='wayland',LIBGL_ALWAYS_SOFTWARE='1',XKB_DEFAULT_LAYOUT=layout)
   with (a.output/(layout+'-run.log')).open('w') as log:
    proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--test-binary',str(binary),'--bridge',str(bridge),'--layout',layout],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:code=proc.wait(timeout=40)
    finally:
     try:os.killpg(proc.pid,signal.SIGTERM)
     except ProcessLookupError:pass
     proc.wait(timeout=5)
   for name in ['kwin.log','client.log','bridge.log','events.json','partial.json']:
    if (root/name).exists():(a.output/(layout+'-'+name)).write_text((root/name).read_text())
   assert code==0,(a.output/(layout+'-run.log')).read_text()[-4000:]
   reports.append(json.loads((root/'report.json').read_text()))
 report={'layouts':reports,'test_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'source_sha256':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path('src/system_keyboard.rs'),Path('src/keyboard_layout.rs'),Path('src/system_actions.rs'),Path('src/system_dial.rs')]},'user_input_used':False,'uinput_used':False}
 (a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n');print('PASS: Caps/Num Lock, second-keyboard Ctrl/Shift/AltGr ownership, held-letter repeat and modifier-only nonrepeat in four layouts. No physical user input.')
if __name__=='__main__':main()
