#!/usr/bin/env python3
"""Actual Close and held Open Application against owned Qt windows in private KWin."""
import argparse,hashlib,json,os,shlex,shutil,signal,subprocess,sys,tempfile,time
from pathlib import Path
from unittest.mock import patch
from system_interop import Host
from sample_interop import tap

CLOSE='com.elgato.streamdeck.system.close';OPEN='com.elgato.streamdeck.system.openapp'
WINDOW=r'''
import os,sys,time
from pathlib import Path
root=Path(sys.argv[1]);root.mkdir(exist_ok=True)
if len(sys.argv)>2 and sys.argv[2]=='helper':
 (root/('ready-'+str(os.getpid()))).write_text('helper');time.sleep(60);raise SystemExit
from PySide6.QtWidgets import QApplication,QWidget,QDialog
from PySide6.QtCore import QTimer
Base=QDialog if len(sys.argv)>2 and sys.argv[2]=='dialog' else QWidget
class Window(Base):
 def closeEvent(self,event):
  with (root/'events').open('a') as f:f.write(str(os.getpid())+'\n')
  if (root/'allow-close').exists():event.accept()
  else:event.ignore()
app=QApplication([]);w=Window();w.setWindowTitle('Deckforge owned close fixture');w.resize(320,150);w.show()
QTimer.singleShot(250,lambda:(root/('ready-'+str(os.getpid()))).write_text('window'))
QTimer.singleShot(60000,app.quit);app.exec()
'''
def wait(fn,label,seconds=7):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        if fn():return
        time.sleep(.02)
    raise AssertionError(label)

def inside(root,binary):
    from gi.repository import Gio,GLib
    assert str(root).startswith('/tmp/deckforge-close-') and os.environ['XDG_RUNTIME_DIR']==str(root/'runtime')
    children=[];host=None;error_host=None
    with (root/'kwin.log').open('w') as kl,(root/'apps.log').open('w') as al:
        kwin=subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-close'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=kl,stderr=kl)
        try:
            wait(lambda:(root/'runtime/deckforge-close').exists(),'private compositor socket')
            bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            def owned_bus():
                try:return bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('org.kde.KWin',)),None,Gio.DBusCallFlags.NONE,1000,None).unpack()[0]==kwin.pid
                except GLib.Error:return False
            wait(owned_bus,'private KWin service owner')
            executable=root/'owned-python';shutil.copy2('/usr/bin/python',executable)
            script=root/'window fixture.py';script.write_text(WINDOW)
            selected=root/'selected app';other=root/'other app';selected.mkdir();other.mkdir()
            def launch(directory,kind='window'):
                child=subprocess.Popen([str(executable),str(script),str(directory),kind],stdout=al,stderr=al);children.append(child)
                wait(lambda:(directory/('ready-'+str(child.pid))).exists(),'owned app ready');return child
            main=launch(selected);dialog=launch(selected,'dialog');helper=launch(selected,'helper');unrelated=launch(other)
            host=Host(binary,[],root/'state',root/'host.log').ready()
            args=shlex.join([str(script),str(selected)])
            def configure(settings,action=CLOSE,h=None):
                h=h or host;h.post({'command':'assign','controller':'Keypad','index':0,'uuid':action})
                slot=next(s for s in h.state()['slots'] if s['index']==0 and s['controller']=='Keypad')
                h.post({'command':'settings','context':slot['context'],'settings':settings})
            def events(directory):return (directory/'events').read_text().splitlines() if (directory/'events').exists() else []
            settings={'path':str(executable),'linuxMatchArgs':args,'forceQuit':False,'linuxCloseMethod':'windows','Future':{'keep':9}}
            configure(settings);tap(host)
            host.wait(lambda s:'Window close requested' in s['system_status'],'window close request')
            wait(lambda:str(main.pid) in events(selected) and str(dialog.pid) in events(selected),'normal and dialog close events')
            assert all(p.poll() is None for p in [main,dialog,helper,unrelated]) and not events(other),'Close bypassed veto or wrong argument scope'
            count=len(events(selected));configure({**settings,'linuxCloseMethod':'automatic'});tap(host)
            wait(lambda:len(events(selected))>=count+2,'automatic window close')
            assert all(p.poll() is None for p in [main,dialog,helper,unrelated]),'Automatic mode killed a GUI app/helper after veto'
            # Held Open Application shares the same guarded window path.
            count=len(events(selected));configure({'exec':str(executable),'args':args,'bring_to_front':False,'long_press':'quit'},OPEN)
            host.post({'command':'input','event':'keyDown','index':0});wait(lambda:len(events(selected))>=count+2,'held app close');host.post({'command':'input','event':'keyUp','index':0})
            assert all(p.poll() is None for p in [main,dialog,helper,unrelated])
            (selected/'allow-close').write_text('yes');configure(settings);tap(host)
            wait(lambda:main.poll() is not None and dialog.poll() is not None,'accepted close exits GUI apps')
            assert main.returncode==0 and dialog.returncode==0
            assert helper.poll() is None and unrelated.poll() is None
            # Window-only close does not terminate a remaining background helper.
            tap(host);host.wait(lambda s:'No matching application windows' in s['system_status'],'headless window-only result');assert helper.poll() is None
            configure({**settings,'linuxCloseMethod':'terminate'});tap(host);helper.wait(timeout=4);assert helper.returncode==-15
            # Force close bypasses the app's closeEvent only when explicitly selected.
            (selected/'allow-close').unlink();forced=launch(selected);forced_helper=launch(selected,'helper');before=len(events(selected))
            configure({**settings,'forceQuit':True});tap(host);forced.wait(timeout=4);forced_helper.wait(timeout=4)
            assert forced.returncode==-9 and forced_helper.returncode==-9 and len(events(selected))==before and unrelated.poll() is None
            # A failed compositor connection cannot silently escalate to SIGTERM.
            sleeper=root/'owned sleep';shutil.copy2('/usr/bin/sleep',sleeper);child=subprocess.Popen([str(sleeper),'30']);children.append(child)
            with patch.dict(os.environ,{'DBUS_SESSION_BUS_ADDRESS':'unix:path='+str(root/'missing-bus')}):
                error_host=Host(binary,[],root/'error-state',root/'error-host.log').ready()
            configure({'path':str(sleeper),'linuxCloseMethod':'automatic'},h=error_host);tap(error_host)
            error_host.wait(lambda s:'Desktop window close unavailable' in s['system_status'],'failed desktop close');assert child.poll() is None
            configure({'path':str(sleeper),'linuxCloseMethod':'terminate'},h=error_host);tap(error_host);child.wait(timeout=4);assert child.returncode==-15
            tap(error_host);error_host.wait(lambda s:'Nothing to close' in s['system_status'],'already exited no-op')
            assert unrelated.poll() is None and not events(other)
            legacy=subprocess.Popen([str(sleeper),'30']);children.append(legacy)
            configure({'path':str(sleeper)});tap(host);legacy.wait(timeout=4);assert legacy.returncode==-15
            subprocess.run([sys.executable,str(Path(__file__).with_name('open_application_interop.py')),'--binary',str(binary),'--kwin','--report',str(root/'activation.json')],check=True,timeout=20)
            print(json.dumps({'normal_and_dialog_close_events':True,'app_veto_preserved':True,'background_helper_preserved_with_gui':True,'literal_argument_scope':True,'held_open_application_close':True,'accepted_close_exits':True,'headless_window_only_no_signal':True,'explicit_sigterm':True,'explicit_force':True,'desktop_error_no_signal_fallback':True,'already_exited_noop':True,'unrelated_window_untouched':True,'user_windows_used':False,'usb_access':False}))
        finally:
            if error_host:error_host.stop()
            if host:host.stop()
            for child in children:
                if child.poll() is None:child.kill()
                child.wait(timeout=3)
            kwin.terminate();kwin.wait(timeout=5)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/system-close'));p.add_argument('--inside',type=Path);a=p.parse_args();binary=a.binary.resolve()
    if a.inside:inside(a.inside,binary);return
    a.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-close-') as d:
        root=Path(d);(root/'runtime').mkdir(mode=0o700);env=os.environ.copy();env.pop('DISPLAY',None)
        env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WAYLAND_DISPLAY='deckforge-close',QT_QPA_PLATFORM='wayland',GDK_BACKEND='wayland',LIBGL_ALWAYS_SOFTWARE='1')
        with (a.output/'run.log').open('w') as log:
            process=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--binary',str(binary)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:code=process.wait(timeout=55)
            finally:
                try:os.killpg(process.pid,signal.SIGTERM)
                except ProcessLookupError:pass
                process.wait(timeout=5)
        for name in ['host.log','error-host.log','kwin.log','apps.log','activation.json']:
            if (root/name).exists():(a.output/name).write_text((root/name).read_text())
        assert code==0,(a.output/'run.log').read_text()[-6000:]
        report=next(json.loads(line) for line in reversed((a.output/'run.log').read_text().splitlines()) if line.startswith('{'))
    report['tested_binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest();(a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: real Qt normal/dialog close and veto, matched helper protection, held app close, explicit signals, backend failure and unrelated-window isolation in private KWin.')

if __name__=='__main__':main()
