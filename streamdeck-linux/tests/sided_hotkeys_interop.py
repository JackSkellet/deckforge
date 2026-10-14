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
 processes=[];reports={}
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
   def chord(name,key,flags):
    before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':key,'VKeyCode':-1,'KeyModifiers':flags}]}))
    result=subprocess.run([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,capture_output=True,text=True,timeout=5)
    assert result.returncode==0,result.stdout+result.stderr
    time.sleep(.15);got=events()[before:];assert got,(layout,name,'no events');reports[name]=got;return got
   for side,flags,scan in [('left',0x10022,29),('right',0x10202,97),('both',0x10222,97)]:
    got=chord(side+'_ctrl_c',67,flags)
    assert any(e['type']=='press' and e['scan']==scan+8 for e in got),(layout,side,got)
    assert any(e['type']=='press' and e['key']==67 and e['modifiers']&0x04000000 for e in got),(layout,side,got)
   got=chord('right_shift_a',65,0x10101)
   assert any(e['type']=='press' and e['scan']==62 for e in got),got
   assert any(e['text']=='A' and e['type']=='press' for e in got),got
   got=chord('right_alt',50 if layout=='no' else 81,0x10404)
   expected='q' if layout=='us' else '@'
   assert any(e['text']==expected and e['type']=='press' for e in got),(layout,got)
   got=chord('plain_y',89,0)
   assert any(e['text']==('z' if layout=='de' else 'y') and e['type']=='press' for e in got),(layout,got)
   got=chord('plain_a_after_release',65,0)
   assert any(e['text']=='a' and e['modifiers']==0 and e['type']=='press' for e in got),(layout,got)
   (root/'report.json').write_text(json.dumps({'layout':layout,'cases':reports,'physical_key_mapping_observed':True},indent=2))
  finally:
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--test-binary',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('target/sided-hotkeys'));p.add_argument('--inside',type=Path);p.add_argument('--bridge',type=Path);p.add_argument('--layout');a=p.parse_args();binary=a.test_binary.resolve()
 if a.inside:inside(a.inside,binary,a.bridge,a.layout);return
 a.output.mkdir(parents=True,exist_ok=True);bridge=a.output.resolve()/'input-bridge'
 flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','Qt6DBus','libei-1.0'],text=True));subprocess.run(['g++','-fPIC','-std=c++17',str(Path(__file__).with_name('focused_clipboard_bridge.cpp')),'-o',str(bridge),*flags],check=True)
 reports=[]
 for layout in ['us','de','no']:
  with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
   root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'config').mkdir();(root/'config/kxkbrc').write_text('[Layout]\nUse=true\nLayoutList='+layout+'\nModel=pc105\nOptions=\n')
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
 report={'layouts':reports,'test_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'keyboard_source_sha256':hashlib.sha256(Path('src/system_keyboard.rs').read_bytes()).hexdigest(),'user_input_used':False,'uinput_used':False}
 (a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n');print('PASS: left/right/both Ctrl, right Shift, AltGr and released modifiers under US/German/Norwegian in owned private KWin. Physical layout mapping recorded.')
if __name__=='__main__':main()
