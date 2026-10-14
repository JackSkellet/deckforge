#!/usr/bin/env python3
"""Actual host activation/Close with owned apps, Xvfb and a private Openbox WM."""
import argparse,ctypes,hashlib,json,os,select,shlex,shutil,signal,socket,struct,subprocess,sys,tempfile,time
from pathlib import Path
from system_interop import Host
from sample_interop import tap
from close_application_interop import WINDOW,wait

CLOSE='com.elgato.streamdeck.system.close';OPEN='com.elgato.streamdeck.system.openapp'
FIXTURE=WINDOW.replace('from PySide6.QtCore import QTimer','from PySide6.QtCore import QTimer,QEvent,Qt').replace(' def closeEvent(self,event):'," def event(self,event):\n  if event.type()==QEvent.Type.WindowActivate:\n   with (root/'active').open('a') as f:f.write(str(os.getpid())+'\\n')\n  return super().event(event)\n def closeEvent(self,event):").replace('w.show()',"w.setWindowFlags(Qt.Tool) if len(sys.argv)>2 and sys.argv[2]=='utility' else None\nw.showMinimized() if len(sys.argv)>2 and sys.argv[2]=='window' else w.show()\n(root/('window-'+str(os.getpid()))).write_text(str(int(w.winId())))")

def inside(root,binary,xvfb,openbox):
 children=[];host=None;wm=None
 with (root/'server.log').open('w') as sl,(root/'wm.log').open('w') as wl,(root/'apps.log').open('w') as al:
  read,write=os.pipe();server=subprocess.Popen([str(xvfb),'-displayfd',str(write),'-screen','0','1024x768x24','-nolisten','tcp','-noreset','-ac'],pass_fds=(write,),stdout=sl,stderr=sl);os.close(write)
  try:
   assert select.select([read],[],[],5)[0];number=os.read(read,100).decode().strip();os.close(read);assert number.isdigit()
   with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as probe:
    probe.connect('/tmp/.X11-unix/X'+number);pid,uid,gid=struct.unpack('3i',probe.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12));assert pid==server.pid and uid==os.getuid()
   os.environ['DISPLAY']=':'+number;subprocess.run(['dbus-update-activation-environment','DISPLAY','XAUTHORITY'],check=True)
   toolroot=openbox.parents[2];wm_env={**os.environ,'LD_LIBRARY_PATH':str(toolroot/'usr/lib'),'XDG_DATA_DIRS':str(toolroot/'usr/share')+':/usr/share','XDG_CONFIG_DIRS':str(toolroot/'etc/xdg')}
   wm=subprocess.Popen([str(openbox),'--sm-disable','--config-file',str(toolroot/'etc/xdg/openbox/rc.xml')],env=wm_env,stdout=wl,stderr=wl)
   x=ctypes.CDLL('libX11.so.6');P=ctypes.c_void_p;U=ctypes.c_ulong;I=ctypes.c_int;PP=ctypes.POINTER
   for name,args,result in [('XOpenDisplay',[ctypes.c_char_p],P),('XDefaultRootWindow',[P],U),('XInternAtom',[P,ctypes.c_char_p,I],U),('XGetWindowProperty',[P,U,U,ctypes.c_long,ctypes.c_long,I,U,PP(U),PP(I),PP(U),PP(U),PP(PP(ctypes.c_ubyte))],I),('XChangeProperty',[P,U,U,U,I,I,PP(ctypes.c_ubyte),I],I),('XDeleteProperty',[P,U,U],I),('XSendEvent',[P,U,I,ctypes.c_long,P],I),('XSync',[P,I],I),('XFree',[P],I),('XCloseDisplay',[P],I)]:
    f=getattr(x,name);f.argtypes=args;f.restype=result
   dpy=x.XOpenDisplay(os.environ['DISPLAY'].encode());assert dpy;rootwin=x.XDefaultRootWindow(dpy)
   def atom(name):return x.XInternAtom(dpy,name.encode(),0)
   def prop(window,name):
    typ=U();fmt=I();n=U();remaining=U();data=PP(ctypes.c_ubyte)();x.XGetWindowProperty(dpy,window,atom(name),0,4096,0,0,ctypes.byref(typ),ctypes.byref(fmt),ctypes.byref(n),ctypes.byref(remaining),ctypes.byref(data));values=[ctypes.cast(data,PP(U))[i] for i in range(n.value)] if data and fmt.value==32 else []
    if data:x.XFree(data)
    return values
   wait(lambda:bool(prop(rootwin,'_NET_SUPPORTING_WM_CHECK')),'private WM ready')
   class Data(ctypes.Union):_fields_=[('l',ctypes.c_long*5),('b',ctypes.c_char*20)]
   class Message(ctypes.Structure):_fields_=[('type',I),('serial',U),('send_event',I),('display',P),('window',U),('message_type',U),('format',I),('data',Data)]
   def request(window,name,values):
    storage=(ctypes.c_long*24)();e=ctypes.cast(storage,PP(Message)).contents;e.type=33;e.display=dpy;e.window=window;e.message_type=atom(name);e.format=32
    for i,v in enumerate(values):e.data.l[i]=v
    assert x.XSendEvent(dpy,rootwin,0,(1<<19)|(1<<20),ctypes.byref(storage));x.XSync(dpy,0)
   executable=root/'owned-python';shutil.copy2('/usr/bin/python',executable);script=root/'window.py';script.write_text(FIXTURE);selected=root/'selected';other=root/'other';selected.mkdir();other.mkdir()
   def launch(directory,kind='window'):
    child=subprocess.Popen([str(executable),str(script),str(directory),kind],stdout=al,stderr=al);children.append(child);wait(lambda:(directory/('ready-'+str(child.pid))).exists(),'owned app ready');return child
   def wid(child,directory=selected):return int((directory/('window-'+str(child.pid))).read_text())
   def events(directory):return (directory/'events').read_text().splitlines() if (directory/'events').exists() else []
   main=launch(selected);dialog=launch(selected,'dialog');helper=launch(selected,'helper');unrelated=launch(other)
   mainwin=wid(main);otherwin=wid(unrelated,other)
   # Deliberately forge the editable PID label: actual server ownership must win.
   fake=(U*1)(main.pid);x.XChangeProperty(dpy,otherwin,atom('_NET_WM_PID'),6,32,0,ctypes.cast(fake,PP(ctypes.c_ubyte)),1);x.XSync(dpy,0)
   host=Host(binary,[],root/'state',root/'host.log').ready();args=shlex.join([str(script),str(selected)])
   def configure(settings,action=CLOSE):
    host.post({'command':'assign','controller':'Keypad','index':0,'uuid':action});slot=next(s for s in host.state()['slots'] if s['index']==0 and s['controller']=='Keypad');host.post({'command':'settings','context':slot['context'],'settings':settings})
   app={'exec':str(executable),'args':shlex.join([str(script),str(selected),'window']),'bring_to_front':True,'long_press':'quit'}
   configure(app,OPEN);tap(host);host.wait(lambda s:'brought to front' in s['system_status'],'X11 activation');wait(lambda:prop(rootwin,'_NET_ACTIVE_WINDOW')==[mainwin],'minimized window activated')
   assert len(list(selected.glob('ready-*')))==3,'Activation launched duplicate'
   request(mainwin,'_NET_WM_DESKTOP',[1,2]);wait(lambda:prop(mainwin,'_NET_WM_DESKTOP')==[1],'window moved to desktop1');request(rootwin,'_NET_CURRENT_DESKTOP',[0,0]);wait(lambda:prop(rootwin,'_NET_CURRENT_DESKTOP')==[0],'desktop0 ready')
   tap(host)
   try:wait(lambda:prop(rootwin,'_NET_CURRENT_DESKTOP')==[1] and prop(rootwin,'_NET_ACTIVE_WINDOW')==[mainwin],'cross-desktop activation')
   except AssertionError:raise AssertionError({'current':prop(rootwin,'_NET_CURRENT_DESKTOP'),'active':prop(rootwin,'_NET_ACTIVE_WINDOW'),'main':mainwin,'main_desktop':prop(mainwin,'_NET_WM_DESKTOP'),'clients':prop(rootwin,'_NET_CLIENT_LIST'),'status':host.state()['system_status']})
   settings={'path':str(executable),'linuxMatchArgs':args,'linuxCloseMethod':'windows'};configure(settings);tap(host);host.wait(lambda s:'Window close requested' in s['system_status'],'X11 close request');wait(lambda:str(main.pid) in events(selected) and str(dialog.pid) in events(selected),'normal/dialog close events')
   assert all(p.poll() is None for p in [main,dialog,helper,unrelated]) and not events(other),'Close bypassed veto or trusted spoofed PID'
   count=len(events(selected));configure({**app,'args':args},OPEN);host.post({'command':'input','event':'keyDown','index':0});wait(lambda:len(events(selected))>=count+2,'held Open close');host.post({'command':'input','event':'keyUp','index':0})
   (selected/'allow-close').write_text('yes');configure(settings);tap(host);wait(lambda:main.poll()==0 and dialog.poll()==0,'accepted normal close');assert helper.poll() is None
   inventory=prop(rootwin,'_NET_CLIENT_LIST');x.XDeleteProperty(dpy,rootwin,atom('_NET_CLIENT_LIST'));x.XSync(dpy,0)
   configure({**settings,'linuxCloseMethod':'automatic'});tap(host);host.wait(lambda s:'Desktop window close unavailable' in s['system_status'],'missing client inventory error');assert helper.poll() is None
   restored=(U*len(inventory))(*inventory);x.XChangeProperty(dpy,rootwin,atom('_NET_CLIENT_LIST'),33,32,0,ctypes.cast(restored,PP(ctypes.c_ubyte)),len(inventory));x.XSync(dpy,0)
   unsupported=launch(selected,'dialog');x.XDeleteProperty(dpy,wid(unsupported),atom('WM_PROTOCOLS'));x.XSync(dpy,0)
   configure({**settings,'linuxCloseMethod':'automatic'});tap(host);host.wait(lambda s:'do not support normal closing' in s['system_status'],'unsupported graceful close');assert unsupported.poll() is None and helper.poll() is None
   configure({**settings,'forceQuit':True});tap(host);unsupported.wait(timeout=3);helper.wait(timeout=3);assert unsupported.returncode==-9 and helper.returncode==-9 and unrelated.poll() is None
   utility=launch(selected,'utility');configure({**settings,'linuxCloseMethod':'automatic'});tap(host);host.wait(lambda s:'do not support normal closing' in s['system_status'],'utility must not become a headless signal');assert utility.poll() is None
   configure({**settings,'forceQuit':True});tap(host);utility.wait(timeout=3);assert utility.returncode==-9
   # No WM must be an error, never an empty inventory followed by SIGTERM.
   sleeper=root/'owned-sleep';shutil.copy2('/usr/bin/sleep',sleeper);background=subprocess.Popen([str(sleeper),'30']);children.append(background)
   wm.terminate();wm.wait(timeout=5);configure({'path':str(sleeper),'linuxCloseMethod':'automatic'});tap(host);host.wait(lambda s:'Desktop window close unavailable' in s['system_status'],'missing WM error');assert background.poll() is None
   configure({'path':str(sleeper),'linuxCloseMethod':'terminate'});tap(host);background.wait(timeout=3);assert background.returncode==-15
   assert unrelated.poll() is None and not events(other);x.XCloseDisplay(dpy)
   (root/'proof.json').write_text(json.dumps({'missing_authority_file_supported':True,'activation_restores_minimized':True,'activation_changes_desktop':True,'no_duplicate_launch':True,'normal_and_dialog_close':True,'veto_preserved':True,'held_open_close':True,'accepted_exit':True,'spoofed_pid_ignored':True,'unsupported_protocol_no_signal':True,'utility_window_no_signal':True,'missing_wm_no_signal':True,'missing_inventory_no_signal':True,'explicit_force_and_sigterm':True,'user_windows_used':False,'usb_access':False},indent=2))
  finally:
   if host:host.stop()
   for child in children:
    if child.poll() is None:child.kill()
    child.wait(timeout=3)
   if wm and wm.poll() is None:wm.terminate();wm.wait(timeout=5)
   server.terminate();server.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/x11-windows'));p.add_argument('--inside',type=Path);p.add_argument('--xvfb',type=Path,default=Path('target/x11-layouts/tools/usr/bin/Xvfb'));p.add_argument('--openbox',type=Path,default=Path('target/x11-windows/tools/usr/bin/openbox'));a=p.parse_args();binary=a.binary.resolve();xvfb=a.xvfb.resolve();openbox=a.openbox.resolve()
 if a.inside:inside(a.inside,binary,xvfb,openbox);return
 a.output.mkdir(parents=True,exist_ok=True)
 with tempfile.TemporaryDirectory(prefix='deckforge-x11-windows-') as d:
  root=Path(d);(root/'runtime').mkdir(mode=0o700);env=os.environ.copy()
  for name in ['DISPLAY','WAYLAND_DISPLAY','WAYLAND_SOCKET']:env.pop(name,None)
  env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_SESSION_TYPE='x11',XDG_CURRENT_DESKTOP='GNOME',XAUTHORITY=str(root/'auth'),QT_QPA_PLATFORM='xcb',GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1')
  with (a.output/'run.log').open('w') as log:
   proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--binary',str(binary),'--xvfb',str(xvfb),'--openbox',str(openbox)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
   try:code=proc.wait(timeout=55)
   finally:
    try:os.killpg(proc.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    proc.wait(timeout=5)
  for name in ['server.log','wm.log','apps.log','host.log']:
   if (root/name).exists():(a.output/name).write_text((root/name).read_text())
  assert code==0,(a.output/'run.log').read_text()[-5000:];report=json.loads((root/'proof.json').read_text())
 report['tested_binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest();(a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n');print('PASS: actual X11 activation/Close, minimized/workspace behavior, spoofed PID isolation, veto and no-signal error handling in owned Xvfb/Openbox.')
if __name__=='__main__':main()
