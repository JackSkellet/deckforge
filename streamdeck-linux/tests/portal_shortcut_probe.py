#!/usr/bin/env python3
"""Investigate logical shortcut delivery through the public portal in private KWin."""
import shutil
import argparse,hashlib,json,os,signal,subprocess,sys,tempfile,time
from pathlib import Path
from sided_hotkeys_interop import CLIENT
from focused_clipboard_interop import wait

def inside(root,binary,layout):
 from gi.repository import Gio,GLib
 assert root.parent==Path('/tmp') and root.name.startswith('deckforge-focused-') and os.environ['XDG_RUNTIME_DIR']==str(root/'runtime')
 processes=[];reports={}
 with (root/'kwin.log').open('w') as kl,(root/'client.log').open('w') as cl:
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
   script=root/'client.py';script.write_text(CLIENT)
   client=subprocess.Popen([sys.executable,str(script),str(root)],stdout=cl,stderr=cl);processes.append(client);wait(root/'client-ready',processes)
   def events():
    try:return json.loads((root/'events.json').read_text())
    except (OSError,ValueError):return []
   def chord(name,mods,key,override=None):
    before=len(events());request=[('keycode',m,1) for m in mods]+[('keysym',key,1),('keysym',key,0)]+[('keycode',m,0) for m in reversed(mods)]
    if override is not None:request=override
    (root/'portal-request.json').write_text(json.dumps(request))
    result=subprocess.run([str(binary),'portal_shortcut_probe_on_private_wayland','--ignored','--nocapture'],env={**os.environ,'DECKFORGE_PRIVATE_CLIPBOARD_TEST':str(root)},capture_output=True,text=True,timeout=10)
    assert result.returncode==0,result.stdout+result.stderr
    time.sleep(.15);got=events()[before:];assert got,(layout,name,'no events');reports[name]={'after_session_close':got,'before_session_close':json.loads((root/'portal-before-close.json').read_text())[before:]}
   for name,mods,key in [('plain_y',[],ord('y')),('ctrl_y',[29],ord('y')),('shift_y',[42],ord('y')),('ctrl_shift_z',[29,42],ord('z')),('right_alt_q',[100],ord('q')),('right_ctrl_y',[97],ord('y')),('plain_a_after_release',[],ord('a'))]:chord(name,mods,key)
   chord('release_after_layout_switch',[],ord('y'),[('keysym',ord('y'),1),('layout',1,0),('keysym',ord('y'),0),('layout',0,0)])
   (root/'report.json').write_text(json.dumps({'layout':layout,'cases':reports},indent=2))
  finally:
   for proc in reversed(processes):
    if proc.poll() is None:proc.terminate()
    proc.wait(timeout=5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--test-binary',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('target/portal-shortcuts'));p.add_argument('--inside',type=Path);p.add_argument('--layout');a=p.parse_args();binary=a.test_binary.resolve()
 if a.inside:inside(a.inside,binary,a.layout);return
 a.output.mkdir(parents=True,exist_ok=True);reports=[]
 for layout in ['us','de','no']:
  with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
   root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'config').mkdir();(root/'config/kxkbrc').write_text('[Layout]\nUse=true\nLayoutList='+layout+(',de' if layout!='de' else ',us')+'\nModel=pc105\nOptions=\n')
   env=os.environ.copy();env.pop('DISPLAY',None);env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WAYLAND_DISPLAY='deckforge-private-input',GDK_BACKEND='wayland',QT_QPA_PLATFORM='wayland',LIBGL_ALWAYS_SOFTWARE='1',XKB_DEFAULT_LAYOUT=layout)
   with (a.output/(layout+'-run.log')).open('w') as log:
    proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--test-binary',str(binary),'--layout',layout],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:code=proc.wait(timeout=50)
    finally:
     try:os.killpg(proc.pid,signal.SIGTERM)
     except ProcessLookupError:pass
     proc.wait(timeout=5)
     if shutil.which('fusermount3'):subprocess.run(['fusermount3','-u',str(root/'runtime/doc')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
   for name in ['kwin.log','client.log','events.json']:
    if (root/name).exists():(a.output/(layout+'-'+name)).write_text((root/name).read_text())
   assert code==0,(a.output/(layout+'-run.log')).read_text()[-4000:]
   reports.append(json.loads((root/'report.json').read_text()))
 ordinary=True;release_requires_close=[]
 for layout in reports:
  cases=layout['cases']
  for name,mask in [('plain_y',0),('ctrl_y',0x04000000),('shift_y',0x02000000),('ctrl_shift_z',0x06000000),('right_ctrl_y',0x04000000)]:
   ordinary &= any(e['type']=='press' and e['key']==(90 if name=='ctrl_shift_z' else 89) and e['modifiers']&mask==mask for e in cases[name]['before_session_close'])
  case=cases['release_after_layout_switch']
  release_requires_close.append(not any(e['type']=='release' for e in case['before_session_close']) and any(e['type']=='release' for e in case['after_session_close']))
 assert ordinary,'Ordinary logical shortcuts failed; inspect recorded events'
 report={'layouts':reports,'test_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'typing_source_sha256':hashlib.sha256(Path('src/system_typing.rs').read_bytes()).hexdigest(),'user_input_used':False,'public_portal':True,'assessment':{'ordinary_logical_shortcuts':ordinary,'release_after_layout_switch_requires_session_close':all(release_requires_close),'suitable_for_persistent_held_shortcuts':not any(release_requires_close)}}
 (a.output/'probe.json').write_text(json.dumps(report,indent=2)+'\n')
 for r in reports:
  print(r['layout'],{name:[(e['key'],e['scan'],e['text'],hex(e['modifiers'])) for e in events if e['type']=='press'] for name,case in r['cases'].items() for events in [case['before_session_close']]})
if __name__=='__main__':main()
