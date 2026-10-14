#!/usr/bin/env python3
"""Actual production layout resolver and writer against a disposable Xvfb/Qt client."""
import socket,struct,threading
import argparse,ctypes,hashlib,json,os,select,shlex,signal,subprocess,sys,tempfile,time
from pathlib import Path
from logical_hotkeys_interop import CLIENT
from focused_clipboard_interop import wait

def inside(root,binary,bridge,xvfb):
 processes=[];reports={};server=None
 with (root/'server.log').open('w') as sl,(root/'client.log').open('w') as cl,(root/'bridge.log').open('w') as bl:
  try:
   read,write=os.pipe();server=subprocess.Popen([str(xvfb),'-displayfd',str(write),'-screen','0','800x600x24','-nolisten','tcp','-noreset','-ac'],pass_fds=(write,),stdout=sl,stderr=sl);processes.append(server);os.close(write)
   assert select.select([read],[],[],5)[0],'Xvfb did not announce a display';number=os.read(read,100).decode().strip();os.close(read);assert number.isdigit()
   with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as probe:
    probe.connect('/tmp/.X11-unix/X'+number);pid,uid,gid=struct.unpack('3i',probe.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12));assert pid==server.pid and uid==os.getuid()
   display=':'+number;os.environ['DISPLAY']=display;env={**os.environ,'DECKFORGE_PRIVATE_X11_PID':str(server.pid),'DECKFORGE_PRIVATE_CLIPBOARD_TEST':str(root)}
   subprocess.run(['dbus-update-activation-environment','DISPLAY','XAUTHORITY'],env=env,check=True)
   subprocess.run(['setxkbmap','-layout','us,de,no,fr','-option',''],env=env,check=True)
   code=CLIENT.replace("w.show();w.activateWindow();w.setFocus()", "w.show();w.activateWindow();w.setFocus();(root/'x11-window').write_text(str(int(w.winId())))")
   script=root/'client.py';script.write_text(code)
   client=subprocess.Popen([sys.executable,str(script),str(root)],env=env,stdout=cl,stderr=cl);processes.append(client)
   relay=subprocess.Popen([str(bridge),str(root)],env=env,stdout=bl,stderr=bl);processes.append(relay);wait(root/'client-ready',processes);wait(root/'bridge-ready',processes)
   x=ctypes.CDLL('libX11.so.6');P=ctypes.c_void_p
   for name,args,result in [('XOpenDisplay',[ctypes.c_char_p],P),('XSetInputFocus',[P,ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong],ctypes.c_int),('XSync',[P,ctypes.c_int],ctypes.c_int),('XkbLockGroup',[P,ctypes.c_uint,ctypes.c_uint],ctypes.c_int),('XCloseDisplay',[P],ctypes.c_int)]:
    f=getattr(x,name);f.argtypes=args;f.restype=result
   dpy=x.XOpenDisplay(display.encode());assert dpy;x.XSetInputFocus(dpy,int((root/'x11-window').read_text()),1,0);x.XSync(dpy,0);time.sleep(.1)
   def group(n):assert x.XkbLockGroup(dpy,0x100,n);x.XSync(dpy,0);time.sleep(.03)
   def events():
    try:return json.loads((root/'events.json').read_text())
    except (OSError,ValueError):return []
   def chord(name,key,flags=0,mode='logical'):
    before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':key,'KeyModifiers':flags,'linuxKeyMode':mode}]}))
    result=subprocess.run([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,capture_output=True,text=True,timeout=5);assert result.returncode==0,result.stdout+result.stderr
    time.sleep(.1);got=events()[before:];assert got,name;reports[name]={'events':got,'resolution':json.loads((root/'logical-resolution.json').read_text())};return got
   for n,layout in enumerate(['us','de','no','fr']):
    group(n)
    for name,key,text in [('y',89,'y'),('slash',47,'/'),('backslash',92,'\\'),('one',49,'1')]:
     got=chord(layout+'_'+name,key);assert any(e['type']=='press' and e['text']==text for e in got),(layout,name,got)
    got=chord(layout+'_right_ctrl_y',89,0x10202);assert any(e['type']=='press' and e['key']==89 and e['modifiers']&0x04000000 for e in got),(layout,got)
   group(1);got=chord('physical_y_in_german',89,mode='physical');assert any(e['type']=='press' and e['text']=='z' for e in got),got
   group(0);before=len(events());(root/'hotkey-request.json').write_text(json.dumps({'Hotkeys':[{'QTKeyCode':89,'linuxKeyMode':'logical'}],'waitForRelease':True}))
   held=subprocess.Popen([str(binary),'sided_hotkeys_on_private_wayland','--ignored','--nocapture'],env=env,stdout=cl,stderr=cl);processes.append(held);wait(root/'logical-held',processes);time.sleep(.1)
   press=next(e for e in events()[before:] if e['type']=='press');group(1);(root/'logical-release').write_text('release');assert held.wait(timeout=4)==0;processes.remove(held);time.sleep(.15)
   got=events()[before:];assert any(e['type']=='release' and e['scan']==press['scan'] for e in got),got;reports['release_after_group_change']=got
   subprocess.run(['setxkbmap','-layout','us','-variant','dvorak','-option',''],env=env,check=True);group(0)
   got=chord('replacement_dvorak_ctrl_v',86,2);assert any(e['type']=='press' and e['key']==86 and e['scan']==60 and e['modifiers']&0x04000000 for e in got),got
   for mode in ['cancel','timeout']:
    for name in ['proxy-accepted','lookup-failed','proxy-unblocked','lookup-proof.json']:(root/name).unlink(missing_ok=True)
    listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen();listener.settimeout(.1);port=listener.getsockname()[1];assert port>6000
    stop=threading.Event();unblock=threading.Event();handlers=[]
    def forward(client,first):
     with client:
      if first:
       (root/'proxy-accepted').write_text('ready')
       if not unblock.wait(5):return
      with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as upstream:
       upstream.connect('/tmp/.X11-unix/X'+number)
       while not stop.is_set():
        ready=select.select([client,upstream],[],[],.1)[0]
        for source in ready:
         data=source.recv(65536)
         if not data:return
         (upstream if source is client else client).sendall(data)
    def accept():
     first=True
     while not stop.is_set():
      try:client,_=listener.accept()
      except socket.timeout:continue
      thread=threading.Thread(target=forward,args=(client,first),daemon=True);handlers.append(thread);thread.start();first=False
    acceptor=threading.Thread(target=accept,daemon=True);acceptor.start()
    probe=subprocess.Popen([str(binary),'layout_lookup_failure_on_private_x11','--ignored','--nocapture'],env={**env,'DISPLAY':f'127.0.0.1:{port-6000}','DECKFORGE_LAYOUT_PROBE_MODE':mode},stdout=cl,stderr=cl);processes.append(probe)
    try:
     wait(root/'lookup-failed',processes);unblock.set();(root/'proxy-unblocked').write_text('ready');assert probe.wait(timeout=5)==0;processes.remove(probe)
     reports['lookup_'+mode]=json.loads((root/'lookup-proof.json').read_text())
    finally:
     unblock.set();stop.set();acceptor.join(timeout=1);listener.close()
     for thread in handlers:thread.join(timeout=1)
   client.terminate();client.wait(timeout=3);processes.remove(client);(root/'x11-window').unlink()
   gtk_source=Path(__file__).with_name('focused_clipboard_client.py').read_text().replace("gi.require_version('Gtk','3.0')","gi.require_version('Gtk','3.0');gi.require_version('GdkX11','3.0');from gi.repository import GdkX11").replace('window.show_all();view.grab_focus();',"window.show_all();view.grab_focus();(root/'x11-window').write_text(str(GdkX11.X11Window.get_xid(window.get_window())));")
   gtk_script=root/'gtk-client.py';gtk_script.write_text(gtk_source)
   gtk=subprocess.Popen([sys.executable,str(gtk_script),str(root)],env=env,stdout=cl,stderr=cl);processes.append(gtk);wait(root/'x11-window',processes);x.XSetInputFocus(dpy,int((root/'x11-window').read_text()),1,0);x.XSync(dpy,0);time.sleep(.2)
   clip_env={**env,'DECKFORGE_TEST_LOGICAL_KEYS':'1'}
   checked=subprocess.run([str(binary),'focused_clipboard_on_private_wayland','--ignored','--nocapture'],env=clip_env,capture_output=True,text=True,timeout=12);assert checked.returncode==0,checked.stdout+checked.stderr
   reports['dvorak_clipboard']=json.loads((root/'rust-proof.json').read_text());gtk.terminate();gtk.wait(timeout=3);processes.remove(gtk)
   terminal=subprocess.Popen(['konsole','--separate','-e',sys.executable,str(Path(__file__).with_name('focused_clipboard_terminal.py')),str(root)],env=env,stdout=cl,stderr=cl);processes.append(terminal);wait(root/'terminal-ready.json',processes)
   U=ctypes.c_ulong;UI=ctypes.c_uint;PP=ctypes.POINTER
   for name,args,result in [('XDefaultRootWindow',[P],U),('XQueryTree',[P,U,PP(U),PP(U),PP(PP(U)),PP(UI)],ctypes.c_int),('XInternAtom',[P,ctypes.c_char_p,ctypes.c_int],U),('XGetWindowProperty',[P,U,U,ctypes.c_long,ctypes.c_long,ctypes.c_int,U,PP(U),PP(ctypes.c_int),PP(U),PP(U),PP(PP(ctypes.c_ubyte))],ctypes.c_int),('XFree',[P],ctypes.c_int)]:
    f=getattr(x,name);f.argtypes=args;f.restype=result
   atom=x.XInternAtom(dpy,b'_NET_WM_PID',0)
   def terminal_window():
    windows=[x.XDefaultRootWindow(dpy)];seen=0
    while windows and seen<4096:
     win=windows.pop();seen+=1;typ=U();fmt=ctypes.c_int();count=U();remaining=U();data=PP(ctypes.c_ubyte)()
     x.XGetWindowProperty(dpy,win,atom,0,1,0,6,ctypes.byref(typ),ctypes.byref(fmt),ctypes.byref(count),ctypes.byref(remaining),ctypes.byref(data))
     pid=ctypes.cast(data,PP(U))[0] if data and count.value==1 and fmt.value==32 else None
     if data:x.XFree(data)
     if pid==terminal.pid:return win
     rootwin=U();parent=U();children=PP(U)();n=UI()
     if x.XQueryTree(dpy,win,ctypes.byref(rootwin),ctypes.byref(parent),ctypes.byref(children),ctypes.byref(n)):
      windows.extend(children[i] for i in range(n.value))
      if children:x.XFree(children)
   target=None;deadline=time.monotonic()+3
   while target is None:
    target=terminal_window();assert time.monotonic()<deadline,'Owned terminal window not found';time.sleep(.03)
   x.XSetInputFocus(dpy,target,1,0);x.XSync(dpy,0);time.sleep(.15)
   checked=subprocess.run([str(binary),'terminal_clipboard_on_private_wayland','--ignored','--nocapture'],env=clip_env,capture_output=True,text=True,timeout=12);assert checked.returncode==0,checked.stdout+checked.stderr
   reports['dvorak_terminal']=json.loads((root/'terminal-proof.json').read_text())
   x.XCloseDisplay(dpy)
   (root/'report.json').write_text(json.dumps({'cases':reports,'server_pid_verified':True,'server':'Xvfb','user_display_used':False,'uinput_used':False,'kde_required':False},indent=2))
  finally:
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--test-binary',type=Path,required=True);p.add_argument('--xvfb',type=Path,default=Path('target/x11-layouts/tools/usr/bin/Xvfb'));p.add_argument('--output',type=Path,default=Path('target/x11-layouts'));p.add_argument('--inside',type=Path);p.add_argument('--bridge',type=Path);a=p.parse_args();binary=a.test_binary.resolve();xvfb=a.xvfb.resolve()
 if a.inside:inside(a.inside,binary,a.bridge,xvfb);return
 a.output.mkdir(parents=True,exist_ok=True);bridge=a.output.resolve()/'input-bridge';flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','Qt6Core','x11','xtst'],text=True));subprocess.run(['g++','-fPIC','-std=c++17',str(Path(__file__).with_name('x11_keyboard_bridge.cpp')),'-o',str(bridge),*flags],check=True)
 with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
  root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'empty-auth').touch();env=os.environ.copy()
  for name in ['DISPLAY','WAYLAND_DISPLAY','WAYLAND_SOCKET']:env.pop(name,None)
  env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_SESSION_TYPE='x11',XDG_CURRENT_DESKTOP='GNOME',XAUTHORITY=str(root/'empty-auth'),QT_QPA_PLATFORM='xcb',GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1')
  with (a.output/'run.log').open('w') as log:
   proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--test-binary',str(binary),'--bridge',str(bridge),'--xvfb',str(xvfb)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
   try:code=proc.wait(timeout=60)
   finally:
    try:os.killpg(proc.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    proc.wait(timeout=5)
  for name in ['server.log','client.log','bridge.log','events.json']:
   if (root/name).exists():(a.output/name).write_text((root/name).read_text())
  assert code==0,(a.output/'run.log').read_text()[-4000:]
  report=json.loads((root/'report.json').read_text())
 report.update(test_binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),source_sha256={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path('src/system_keyboard.rs'),Path('src/keyboard_layout.rs'),Path('src/system_actions.rs')]});(a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n');print('PASS: production X11 layout mapping in four groups, physical opt-out, held release, and live Dvorak map replacement. Owned Xvfb only.')
if __name__=='__main__':main()
