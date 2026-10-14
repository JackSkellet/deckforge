#!/usr/bin/env python3
"""Public portal EIS: resolve against its keymap and release captured physical identities."""
import shutil
import argparse,ctypes,hashlib,json,os,shlex,signal,struct,subprocess,sys,tempfile,time
from pathlib import Path
from sided_hotkeys_interop import CLIENT
from focused_clipboard_interop import wait

def inside(root,bridge,layout,wayland_reader=None):
 from gi.repository import Gio,GLib
 assert root.parent==Path('/tmp') and root.name.startswith('deckforge-focused-') and os.environ['XDG_RUNTIME_DIR']==str(root/'runtime')
 processes=[];session=None;reports={}
 with (root/'kwin.log').open('w') as kl,(root/'client.log').open('w') as cl,(root/'bridge.log').open('w') as bl:
  try:
   kwin=subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-private-input'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=kl,stderr=kl);processes.append(kwin);wait(root/'runtime/deckforge-private-input',processes)
   bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
   def call(service,path,interface,method,signature,args):return bus.call_sync(service,path,interface,method,GLib.Variant(signature,args),None,Gio.DBusCallFlags.NONE,5000,None).unpack()
   for _ in range(100):
    try:
     assert call('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID','(s)',('org.kde.KWin',))[0]==kwin.pid;break
    except GLib.Error:time.sleep(.03)
   else:raise AssertionError('Private KWin not owned')
   call('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','StartServiceByName','(su)',('org.freedesktop.impl.portal.PermissionStore',0))
   pid=call('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID','(s)',('org.freedesktop.impl.portal.PermissionStore',))[0]
   assert ('XDG_DATA_HOME='+str(root/'data')).encode() in Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
   call('org.freedesktop.impl.portal.PermissionStore','/org/freedesktop/impl/portal/PermissionStore','org.freedesktop.impl.portal.PermissionStore','SetPermission','(sbssas)',('kde-authorized',True,'remote-desktop','',['yes']))
   service='org.freedesktop.portal.Desktop';path='/org/freedesktop/portal/desktop';interface='org.freedesktop.portal.RemoteDesktop';responses={}
   def response(connection,sender,path,interface,signal,parameters,data):responses[path]=parameters.unpack()
   subscription=bus.signal_subscribe(service,'org.freedesktop.portal.Request','Response',None,None,Gio.DBusSignalFlags.NONE,response,None)
   def request(method,signature,args):
    request_path=call(service,path,interface,method,signature,args)[0]
    deadline=time.monotonic()+8;context=GLib.MainContext.default()
    while request_path not in responses:
     assert time.monotonic()<deadline,method
     while context.pending():context.iteration(False)
     time.sleep(.005)
    code,result=responses.pop(request_path);assert code==0,(method,code);return result
   session=request('CreateSession','(a{sv})',({'session_handle_token':GLib.Variant('s','deckforge_probe')},))['session_handle']
   request('SelectDevices','(oa{sv})',(session,{'types':GLib.Variant('u',1),'persist_mode':GLib.Variant('u',0)}))
   assert request('Start','(osa{sv})',(session,'',{}))['devices']&1
   reply,fds=bus.call_with_unix_fd_list_sync(service,path,interface,'ConnectToEIS',GLib.Variant('(oa{sv})',(session,{})),None,Gio.DBusCallFlags.NONE,5000,None,None)
   fd=fds.get(reply.unpack()[0]);env={**os.environ,'DECKFORGE_PRIVATE_KWIN_PID':str(kwin.pid),'DECKFORGE_PRIVATE_EIS_FD':str(fd)}
   relay=subprocess.Popen([str(bridge),str(root)],env=env,pass_fds=(fd,),stdout=bl,stderr=bl);os.close(fd);processes.append(relay)
   script=root/'client.py';script.write_text(CLIENT);client=subprocess.Popen([sys.executable,str(script),str(root)],stdout=cl,stderr=cl);processes.append(client)
   wait(root/'bridge-ready',processes);wait(root/'client-ready',processes);wait(root/'keymap.xkb',processes)
   if wayland_reader:subprocess.run([str(wayland_reader),str(root)],check=True,timeout=3)
   lib=ctypes.CDLL('libxkbcommon.so.0');P=ctypes.c_void_p;U=ctypes.c_uint32
   for name,args,result in [('xkb_context_new',[ctypes.c_int],P),('xkb_keymap_new_from_string',[P,ctypes.c_char_p,ctypes.c_int,ctypes.c_int],P),('xkb_keymap_min_keycode',[P],U),('xkb_keymap_max_keycode',[P],U),('xkb_keymap_key_get_syms_by_level',[P,U,U,U,ctypes.POINTER(ctypes.POINTER(U))],ctypes.c_int),('xkb_keymap_unref',[P],None),('xkb_context_unref',[P],None)]:
    f=getattr(lib,name);f.argtypes=args;f.restype=result
   context=lib.xkb_context_new(0);keymap=lib.xkb_keymap_new_from_string(context,(root/('wayland-keymap.xkb' if wayland_reader else 'keymap.xkb')).read_bytes().rstrip(b'\0'),1,0);assert keymap
   def keyboard():
    if not wayland_reader and (root/'keyboard-state.json').exists():return {**json.loads((root/'keyboard-state.json').read_text()),'source':'EIS'}
    return {'group':call('org.kde.keyboard','/Layouts','org.kde.KeyboardLayouts','getLayout','()',())[0],'depressed':None,'source':'KDE D-Bus'}
   def resolve(symbol):
    group=keyboard()['group'];found=[]
    for key in range(lib.xkb_keymap_min_keycode(keymap),min(263,lib.xkb_keymap_max_keycode(keymap))+1):
     syms=ctypes.POINTER(U)();n=lib.xkb_keymap_key_get_syms_by_level(keymap,key,group,0,ctypes.byref(syms))
     if n==1 and syms[0]==symbol:found.append(key-8)
    assert found,(layout,group,symbol);return found[0]
   def events():
    try:return json.loads((root/'events.json').read_text())
    except (OSError,ValueError):return []
   def emit(key,down):
    with (root/'keys.bin').open('ab') as out:out.write(struct.pack('@llHHi',0,0,1,key,int(down))+struct.pack('@llHHi',0,0,0,0,0))
    time.sleep(.05)
   def switch(group):
    assert call('org.kde.keyboard','/Layouts','org.kde.KeyboardLayouts','setLayout','(u)',(group,))[0]
    deadline=time.monotonic()+3
    while keyboard()['group']!=group:
     assert time.monotonic()<deadline,keyboard();time.sleep(.01)
   def check(name,mods):
    before=len(events());key=resolve(ord('y'))
    for m in mods:emit(m,True)
    emit(key,True);emit(key,False)
    for m in reversed(mods):emit(m,False)
    got=events()[before:];expected=(0x04000000 if any(m in [29,97] for m in mods) else 0)|(0x02000000 if any(m in [42,54] for m in mods) else 0)
    assert any(e['type']=='press' and e['key']==89 and e['modifiers']==expected for e in got),got
    assert any(e['type']=='release' and e['scan']==key+8 for e in got),got
    reports[name]={'resolved_evdev':key,'events':got}
   reports['initial_keyboard_state']=keyboard()
   check('initial_y',[]);check('ctrl_y',[29]);check('right_ctrl_shift_y',[97,54])
   before=len(events());key=resolve(ord('y'));emit(key,True);switch(1);emit(key,False)
   got=events()[before:];assert any(e['type']=='release' and e['scan']==key+8 for e in got),got
   reports['release_after_layout_switch']={'resolved_evdev':key,'events_before_session_close':got}
   check('y_in_second_layout',[]);switch(0);check('y_after_switch_back',[])
   reports['keyboard_state']=keyboard();assert all(e['modifiers']==0 for e in reports['y_after_switch_back']['events'] if e['type']=='press'),reports['y_after_switch_back']
   lib.xkb_keymap_unref(keymap);lib.xkb_context_unref(context)
   (root/'report.json').write_text(json.dumps({'layout':layout,'public_eis':True,'keymap_source':'unfocused public Wayland' if wayland_reader else 'EIS','cases':reports},indent=2))
  finally:
   if session:
    try:call('org.freedesktop.portal.Desktop',session,'org.freedesktop.portal.Session','Close','()',())
    except Exception:pass
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,default=Path('target/eis-layouts'));p.add_argument('--inside',type=Path);p.add_argument('--bridge',type=Path);p.add_argument('--layout');p.add_argument('--wayland-reader',type=Path);a=p.parse_args()
 if a.inside:inside(a.inside,a.bridge,a.layout,a.wayland_reader);return
 a.output.mkdir(parents=True,exist_ok=True);bridge=a.output.resolve()/'input-bridge';flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','Qt6DBus','libei-1.0'],text=True));subprocess.run(['g++','-fPIC','-std=c++17',str(Path(__file__).with_name('focused_clipboard_bridge.cpp')),'-o',str(bridge),*flags],check=True)
 reports=[]
 for layout in ['us','de','no']:
  with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
   root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'config').mkdir();(root/'config/kxkbrc').write_text('[Layout]\nUse=true\nLayoutList='+layout+(',de' if layout!='de' else ',us')+'\nModel=pc105\nOptions=\n')
   env=os.environ.copy();env.pop('DISPLAY',None);env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WAYLAND_DISPLAY='deckforge-private-input',GDK_BACKEND='wayland',QT_QPA_PLATFORM='wayland',LIBGL_ALWAYS_SOFTWARE='1',XKB_DEFAULT_LAYOUT=layout)
   with (a.output/(layout+'-run.log')).open('w') as log:
    proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--bridge',str(bridge),'--layout',layout,*(['--wayland-reader',str(a.wayland_reader.resolve())] if a.wayland_reader else [])],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:code=proc.wait(timeout=40)
    finally:
     try:os.killpg(proc.pid,signal.SIGTERM)
     except ProcessLookupError:pass
     proc.wait(timeout=5)
     if shutil.which('fusermount3'):subprocess.run(['fusermount3','-u',str(root/'runtime/doc')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
   for name in ['kwin.log','client.log','bridge.log','keyboard-state.json']:
    if (root/name).exists():(a.output/(layout+'-'+name)).write_text((root/name).read_text())
   assert code==0,(a.output/(layout+'-run.log')).read_text()[-4000:]
   reports.append(json.loads((root/'report.json').read_text()))
 (a.output/'proof.json').write_text(json.dumps({'layouts':reports,'public_portal':True,'user_input_used':False,'prototype_only':True,'source_sha256':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path(__file__),Path(__file__).with_name('focused_clipboard_bridge.cpp'),*([Path(__file__).with_name('wayland_keymap_probe.c')] if a.wayland_reader else [])]}},indent=2)+'\n');print('PASS: public EIS keymap resolves logical Y, Ctrl/Shift modifiers and captured releases across live layout switches in private KWin.')
if __name__=='__main__':main()
