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
  events.append({'type':'press','key':e.key(),'scan':e.nativeScanCode(),'text':e.text(),'modifiers':e.modifiers().value});save()
 def keyReleaseEvent(self,e):
  events.append({'type':'release','key':e.key(),'scan':e.nativeScanCode(),'text':e.text(),'modifiers':e.modifiers().value});save()
def save():(root/'events.json').write_text(json.dumps(events))
app=QApplication([]);w=Window();w.setWindowTitle('Deckforge owned modifier test');w.resize(320,160);w.show();w.activateWindow();w.setFocus()
QTimer.singleShot(300,lambda:(root/'client-ready').write_text('yes'))
QTimer.singleShot(45000,app.quit);app.exec()
'''
def inside(root,binary,bridge,layout):
 processes=[];reports={};timings=[]
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
   wait(root/'client-ready',processes);wait(root/'bridge-ready',processes)
   def events():
    try:return json.loads((root/'events.json').read_text())
    except (OSError,ValueError):return []
   def chord(name,key,flags,**extra):
    before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':key,'VKeyCode':-1,'KeyModifiers':flags,'linuxKeyMode':'logical',**extra}]}))
    result=subprocess.run([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,capture_output=True,text=True,timeout=5)
    assert result.returncode==0,result.stdout+result.stderr
    time.sleep(.15);got=events()[before:];assert got,(layout,name,'no events');reports[name]=got;timings.append(json.loads((root/'logical-resolution.json').read_text())['micros']);return got
   for side,flags,scan in [('left',0x10022,29),('right',0x10202,97),('both',0x10222,97)]:
    got=chord(side+'_ctrl_c',67,flags)
    assert any(e['type']=='press' and e['scan']==scan+8 for e in got),(layout,side,got)
    assert any(e['type']=='press' and e['key']==67 and e['modifiers']&0x04000000 for e in got),(layout,side,got)
   for name,flags,extra in [('legacy_ctrl',8192,{'KeyCtrl':True}),('modern_ctrl_stale_false',2,{'KeyCtrl':False}),('modern_right_stale_false',0x10202,{'KeyCtrl':False})]:
    got=chord(name,67,flags,**extra);assert any(e['type']=='press' and e['key']==67 and e['modifiers']&0x04000000 for e in got),(layout,name,got)
   got=chord('modern_zero_stale_true',67,0,KeyCtrl=True,KeyShift=True);assert any(e['type']=='press' and e['text']=='c' and e['modifiers']==0 for e in got),(layout,got)
   got=chord('legacy_modifier_only',0x01ffffff,8192,KeyCtrl=True,NativeCode=146);assert all(e['key']==0x01000021 for e in got),(layout,got)
   got=chord('right_shift_a',65,0x10101)
   assert any(e['type']=='press' and e['scan']==62 for e in got),got
   assert any(e['text']=='A' and e['type']=='press' for e in got),got
   if layout!='fr':
    got=chord('right_alt',50 if layout=='no' else 81,0x10404)
    expected='q' if layout=='us' else '@'
    assert any(e['text']==expected and e['type']=='press' for e in got),(layout,got)
   got=chord('plain_y',89,0)
   assert any(e['text']=='y' and e['type']=='press' for e in got),(layout,got)
   got=chord('plain_a_after_release',65,0)
   assert any(e['text']=='a' and e['modifiers']==0 and e['type']=='press' for e in got),(layout,got)
   for name,qt,text in [('slash',47,'/'),('backslash',92,'\\'),('digit_one',49,'1')]:
    got=chord(name,qt,0);assert any(e['type']=='press' and e['text']==text for e in got),(layout,name,got)
   before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':89,'linuxKeyMode':'logical'}],'waitForRelease':True}))
   held=subprocess.Popen([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,stdout=cl,stderr=cl);processes.append(held);wait(root/'logical-held',processes)
   time.sleep(.1);press=next(e for e in events()[before:] if e['type']=='press' and e['key']==89)
   subprocess.run(['busctl','--user','call','org.kde.keyboard','/Layouts','org.kde.KeyboardLayouts','setLayout','u','1'],check=True,stdout=subprocess.DEVNULL)
   (root/'logical-release').write_text('release');assert held.wait(timeout=4)==0;processes.remove(held);time.sleep(.15)
   got=events()[before:];assert any(e['type']=='release' and e['scan']==press['scan'] for e in got),(layout,got)
   reports['held_release_after_layout_switch']=got
   got=chord('y_after_switch',89,0);assert any(e['type']=='press' and e['text']=='y' for e in got),(layout,got)
   worker_before=len(events())
   worker=subprocess.Popen([str(binary),'logical_worker_on_private_wayland','--ignored','--nocapture'],env=env,stdout=cl,stderr=cl);processes.append(worker);wait(root/'worker-held',processes);time.sleep(.1)
   got=events()[worker_before:];assert sum(e['type']=='press' and e['scan']==37 for e in got)==1 and not any(e['type']=='release' and e['scan']==37 for e in got),got
   def switch(group):subprocess.run(['busctl','--user','call','org.kde.keyboard','/Layouts','org.kde.KeyboardLayouts','setLayout','u',str(group)],check=True,stdout=subprocess.DEVNULL)
   switch(0);(root/'worker-release').write_text('release');wait(root/'worker-released',processes)
   def dial_presses():return [e for e in events()[worker_before:] if e['type']=='press' and e['key']==89 and e['modifiers']==0]
   deadline=time.monotonic()+4
   while len(dial_presses())<1:
    assert time.monotonic()<deadline;time.sleep(.005)
   switch(1)
   while len(dial_presses())<2:
    assert time.monotonic()<deadline;time.sleep(.005)
   time.sleep(.35);got=events()[worker_before:];presses=dial_presses();assert len(presses)==2 and presses[0]['scan']!=presses[1]['scan'],got
   assert sum(e['type']=='release' and e['scan']==37 for e in got)==1,got
   for press in presses:assert any(e['type']=='release' and e['scan']==press['scan'] and e['modifiers']==0 for e in got),got
   (root/'worker-finish').write_text('done');assert worker.wait(timeout=3)==0;processes.remove(worker)
   reports['worker_shared_ctrl_and_live_dial_layout']=got
   before_bytes=(root/'keys.bin').stat().st_size
   failed=subprocess.run([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env={**env,'WAYLAND_DISPLAY':str(root/'missing-socket')},capture_output=True,text=True,timeout=4)
   assert failed.returncode!=0 and (root/'keys.bin').stat().st_size==before_bytes,'Unavailable layout emitted fallback keys'
   reports['unavailable_layout_no_input']=True
   reports['resolve_micros']=timings
   (root/'report.json').write_text(json.dumps({'layout':layout,'cases':reports,'logical_mapping_verified':True},indent=2))
  finally:
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--test-binary',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('target/logical-hotkeys'));p.add_argument('--inside',type=Path);p.add_argument('--bridge',type=Path);p.add_argument('--layout');a=p.parse_args();binary=a.test_binary.resolve()
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
   for name in ['kwin.log','client.log','bridge.log','events.json']:
    if (root/name).exists():(a.output/(layout+'-'+name)).write_text((root/name).read_text())
   assert code==0,(a.output/(layout+'-run.log')).read_text()[-4000:]
   reports.append(json.loads((root/'report.json').read_text()))
 report={'layouts':reports,'test_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'source_sha256':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path('src/system_keyboard.rs'),Path('src/keyboard_layout.rs'),Path('src/system_actions.rs'),Path('src/system_dial.rs')]},'user_input_used':False,'uinput_used':False}
 (a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n');print('PASS: production logical chord resolver and writer, modifiers, slash/backslash/digits, held release across layout changes in US/German/Norwegian/French. Private compositor only.')
if __name__=='__main__':main()
