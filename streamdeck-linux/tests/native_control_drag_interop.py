#!/usr/bin/env python3
"""Actual GTK/WebKit pointer drags on an owned Xvfb display, including external files."""
import argparse,ctypes,hashlib,json,os,select,signal,socket,struct,subprocess,sys,tempfile,time
from pathlib import Path
from native_file_open_interop import archive
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const until=async(f,name)=>{for(let i=0;i<250;i++){if(document.body.textContent.includes('The dropped file path is not supported.'))throw Error('Native file hook intercepted internal control drag');const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const key=i=>document.querySelectorAll('.deck-key')[i],at=async i=>(await snapshot()).slots.find(s=>s.controller==='Keypad'&&s.index===i);
const center=e=>{const r=e.getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]};
const gesture=async(name,source,target,ctrl=false)=>{await pause(100);await b.request('_test_progress',{name,source:center(source),target:center(target),ctrl});};
if(!(await snapshot()).simulate)throw Error('Simulation required');
await cmd({command:'assign',controller:'Keypad',index:0,uuid:'com.elgato.streamdeck.system.text'});
const first=await at(0);await cmd({command:'settings',context:first.context,settings:{pastedText:'Owned drag test',isTypingMode:false,Future:{keep:42}}});
await cmd({command:'assign',controller:'Keypad',index:1,uuid:'com.elgato.streamdeck.system.website'});const second=await at(1);
await until(()=>key(0)?.querySelector('img')&&key(2),'canvas ready');
await gesture('move',key(0).querySelector('img'),key(2));await until(async()=>(await at(2))?.uuid===first.uuid&&!(await at(0)),'move to empty key');
await gesture('swap',key(2).querySelector('img'),key(1));await until(async()=>(await at(1))?.uuid===first.uuid&&(await at(2))?.uuid===second.uuid,'swap occupied keys');
const moved=await at(1);await gesture('copy',key(1).querySelector('img'),key(3),true);const copied=await until(async()=>{const s=await at(3);return s?.uuid===first.uuid&&s},'Ctrl-copy');
if(copied.context===moved.context||(await at(1))?.context!==moved.context||copied.settings.Future.keep!==42)throw Error('Copy changed identity or settings');
await gesture('external',key(0),key(6));await until(async()=>(await at(6))?.settings?.text==='from desktop','external file import');
if((await at(1)).settings.Future.keep!==42)throw Error('Moved settings lost');
return {move:true,swap:true,ctrl_copy:true,external_file_drag:true,settings_preserved:true,input:'XTest pointer through native GTK/WebKit drag negotiation',usb_access:false,user_display_used:false};
'''
SOURCE=r'''
import gi,sys
from pathlib import Path
gi.require_version('Gtk','3.0');from gi.repository import Gtk,Gdk
w=Gtk.Window();w.set_default_size(180,100);w.move(1220,30);b=Gtk.Button(label='Owned file');w.add(b)
b.drag_source_set(Gdk.ModifierType.BUTTON1_MASK,[Gtk.TargetEntry.new('text/uri-list',0,0)],Gdk.DragAction.COPY)
b.connect('drag-data-get',lambda widget,ctx,data,info,t:data.set_uris([Path(sys.argv[1]).as_uri()]))
w.connect('destroy',Gtk.main_quit);w.show_all();Gtk.main()
'''
def inside(root,binary,xvfb):
 processes=[]
 with (root/'server.log').open('w') as serverlog,(root/'host.log').open('w') as hostlog,(root/'source.log').open('w') as sourcelog:
  read,write=os.pipe();server=subprocess.Popen([str(xvfb),'-displayfd',str(write),'-screen','0','1500x1000x24','-nolisten','tcp','-noreset','-ac'],pass_fds=(write,),stdout=serverlog,stderr=serverlog);os.close(write);processes.append(server)
  try:
   assert select.select([read],[],[],5)[0];number=os.read(read,100).decode().strip();os.close(read);assert number.isdigit()
   with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as peer:
    peer.connect('/tmp/.X11-unix/X'+number);pid,uid,gid=struct.unpack('3i',peer.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12));assert pid==server.pid and uid==os.getuid()
   os.environ['DISPLAY']=':'+number
   subprocess.run(['dbus-update-activation-environment','DISPLAY'],check=True)
   tools=Path('target/x11-windows/tools').resolve()
   wm=subprocess.Popen([str(tools/'usr/bin/openbox'),'--sm-disable','--config-file',str(tools/'etc/xdg/openbox/rc.xml')],env={**os.environ,'LD_LIBRARY_PATH':str(tools/'usr/lib'),'XDG_DATA_DIRS':str(tools/'usr/share')+':/usr/share'},stdout=serverlog,stderr=serverlog);processes.append(wm);time.sleep(.25)
   x=ctypes.CDLL('libX11.so.6');xt=ctypes.CDLL('libXtst.so.6');P=ctypes.c_void_p;U=ctypes.c_ulong;I=ctypes.c_int;PP=ctypes.POINTER
   api=[('XOpenDisplay',[ctypes.c_char_p],P),('XDefaultRootWindow',[P],U),('XQueryTree',[P,U,PP(U),PP(U),PP(PP(U)),PP(ctypes.c_uint)],I),('XInternAtom',[P,ctypes.c_char_p,I],U),('XGetWindowProperty',[P,U,U,ctypes.c_long,ctypes.c_long,I,U,PP(U),PP(I),PP(U),PP(U),PP(PP(ctypes.c_ubyte))],I),('XGetGeometry',[P,U,PP(U),PP(I),PP(I),PP(ctypes.c_uint),PP(ctypes.c_uint),PP(ctypes.c_uint),PP(ctypes.c_uint)],I),('XTranslateCoordinates',[P,U,U,I,I,PP(I),PP(I),PP(U)],I),('XFree',[P],I),('XFlush',[P],I),('XKeysymToKeycode',[P,U],ctypes.c_uint),('XSetInputFocus',[P,U,I,U],I),('XCloseDisplay',[P],I)]
   for name,args,result in api:f=getattr(x,name);f.argtypes=args;f.restype=result
   for name,args in [('XTestFakeMotionEvent',[P,I,I,I,U]),('XTestFakeButtonEvent',[P,ctypes.c_uint,I,U]),('XTestFakeKeyEvent',[P,ctypes.c_uint,I,U])]:f=getattr(xt,name);f.argtypes=args;f.restype=I
   d=x.XOpenDisplay(os.environ['DISPLAY'].encode());assert d;rootwin=x.XDefaultRootWindow(d);pidatom=x.XInternAtom(d,b'_NET_WM_PID',0)
   def windows():
    typ=U();fmt=I();count=U();remain=U();data=PP(ctypes.c_ubyte)();x.XGetWindowProperty(d,rootwin,x.XInternAtom(d,b'_NET_CLIENT_LIST',0),0,128,0,0,ctypes.byref(typ),ctypes.byref(fmt),ctypes.byref(count),ctypes.byref(remain),ctypes.byref(data));clients=[ctypes.cast(data,PP(U))[i] for i in range(count.value)] if data and fmt.value==32 else []
    if data:x.XFree(data)
    if clients:return clients
    r=U();parent=U();items=PP(U)();n=ctypes.c_uint();x.XQueryTree(d,rootwin,ctypes.byref(r),ctypes.byref(parent),ctypes.byref(items),ctypes.byref(n));result=[items[i] for i in range(n.value)]
    if items:x.XFree(items)
    return result
   def owned(owner):
    for window in windows():
     typ=U();fmt=I();n=U();remain=U();data=PP(ctypes.c_ubyte)();x.XGetWindowProperty(d,window,pidatom,0,1,0,0,ctypes.byref(typ),ctypes.byref(fmt),ctypes.byref(n),ctypes.byref(remain),ctypes.byref(data));pid=ctypes.cast(data,PP(U))[0] if data and n.value and fmt.value==32 else 0
     if data:x.XFree(data)
     if pid==owner:
      r=U();xx=I();yy=I();w=ctypes.c_uint();h=ctypes.c_uint();border=ctypes.c_uint();depth=ctypes.c_uint();x.XGetGeometry(d,window,ctypes.byref(r),ctypes.byref(xx),ctypes.byref(yy),ctypes.byref(w),ctypes.byref(h),ctypes.byref(border),ctypes.byref(depth))
      if w.value>100 and h.value>70:
       child=U();x.XTranslateCoordinates(d,window,rootwin,0,0,ctypes.byref(xx),ctypes.byref(yy),ctypes.byref(child));return window,xx.value,yy.value,w.value,h.value
   action=root/'fixture.streamDeckAction';archive(action,True);sourcefile=root/'source.py';sourcefile.write_text(SOURCE);source=subprocess.Popen([sys.executable,str(sourcefile),str(action)],stdout=sourcelog,stderr=sourcelog);processes.append(source)
   script=root/'probe.js';script.write_text(SCRIPT);env={**os.environ,'DECKFORGE_WEBVIEW_TEST_SCRIPT':str(script),'DECKFORGE_WEBVIEW_TEST_RESULT':str(root/'proof.json')};app=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(root/'state'),'--port','0'],env=env,stdout=hostlog,stderr=hostlog);processes.append(app)
   completed=set();deadline=time.monotonic()+40
   while app.poll() is None:
    if (root/'proof.json').exists():
     app.terminate();app.wait(timeout=5);break
    assert time.monotonic()<deadline,'Native pointer fixture timed out'
    for line in (root/'host.log').read_text().splitlines():
     if not line.startswith('Native fixture progress: '):continue
     step=json.loads(line.split(': ',1)[1]);
     if 'trace' in step:continue
     name=step['name']
     if name in completed:continue
     window,ox,oy,ww,hh=owned(app.pid);assert ww>=1000 and hh>=700
     x.XSetInputFocus(d,window,1,0);origin=[ox+step['source'][0],oy+step['source'][1]];target=[ox+step['target'][0],oy+step['target'][1]]
     if name=='external':
      sw,sx,sy,swid,sh=owned(source.pid);origin=[sx+swid/2,sy+sh/2]
     ctrl=x.XKeysymToKeycode(d,0xffe3)
     xt.XTestFakeMotionEvent(d,-1,int(origin[0]),int(origin[1]),0);x.XFlush(d);time.sleep(.15)
     if step['ctrl']:xt.XTestFakeKeyEvent(d,ctrl,1,0)
     xt.XTestFakeButtonEvent(d,1,1,0);x.XFlush(d);time.sleep(.15)
     for i in range(1,26):
      xt.XTestFakeMotionEvent(d,-1,int(origin[0]+(target[0]-origin[0])*i/25),int(origin[1]+(target[1]-origin[1])*i/25),0);x.XFlush(d);time.sleep(.025)
     time.sleep(.6);xt.XTestFakeButtonEvent(d,1,0,0)
     if step['ctrl']:xt.XTestFakeKeyEvent(d,ctrl,0,0)
     x.XFlush(d);completed.add(name)
    time.sleep(.025)
   assert app.returncode==0,(root/'host.log').read_text()[-3000:]
   proof=json.loads((root/'proof.json').read_text());assert proof.get('ok'),proof
   x.XCloseDisplay(d)
  finally:
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/control-drag/native'));p.add_argument('--inside',type=Path);p.add_argument('--xvfb',type=Path,default=Path('target/x11-layouts/tools/usr/bin/Xvfb'));a=p.parse_args();binary=a.binary.resolve();xvfb=a.xvfb.resolve()
 if a.inside:inside(a.inside,binary,xvfb);return
 a.output.mkdir(parents=True,exist_ok=True);(a.output/'proof.json').unlink(missing_ok=True)
 with tempfile.TemporaryDirectory(prefix='deckforge-control-drag-') as d:
  root=Path(d);(root/'runtime').mkdir(mode=0o700);env=os.environ.copy()
  for key in ['DISPLAY','WAYLAND_DISPLAY','WAYLAND_SOCKET']:env.pop(key,None)
  env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XAUTHORITY=str(root/'auth'),QT_QPA_PLATFORM='xcb',GDK_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1',WEBKIT_DISABLE_COMPOSITING_MODE='1',WEBKIT_DISABLE_DMABUF_RENDERER='1',NO_AT_BRIDGE='1',GTK_USE_PORTAL='0')
  with (a.output/'run.log').open('w') as log:
   proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--binary',str(binary),'--xvfb',str(xvfb)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
   try:code=proc.wait(timeout=50)
   finally:
    try:os.killpg(proc.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    proc.wait(timeout=5)
  for name in ['host.log','source.log','server.log','native.png']:
   if (root/name).exists():(a.output/name).write_bytes((root/name).read_bytes())
  if (root/'proof.json').exists():(a.output/'result.json').write_bytes((root/'proof.json').read_bytes())
  assert code==0,(a.output/'run.log').read_text()[-5000:]
  report=json.loads((root/'proof.json').read_text());report['tested_binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest();(a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n')
 print('PASS: native GTK pointer move, swap, Ctrl-copy and external file import. Owned Xvfb only.')
if __name__=='__main__':main()
