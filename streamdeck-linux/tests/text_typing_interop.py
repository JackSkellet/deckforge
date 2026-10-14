#!/usr/bin/env python3
"""Actual release host + public desktop portal typing on a disposable Wayland desktop."""
import argparse,hashlib,json,os,signal,subprocess,sys,tempfile,time
from pathlib import Path
from system_interop import Host
from sample_interop import slot,tap

TEXT='com.elgato.streamdeck.system.text'
def inside(root,binary):
    import gi
    from gi.repository import Gio,GLib
    assert str(root).startswith('/tmp/deckforge-focused-') and os.environ['XDG_RUNTIME_DIR']==str(root/'runtime') and 'DISPLAY' not in os.environ
    project=Path(__file__).resolve().parents[1];processes=[];host=None;sequence=0
    def client(operation='read',text='',select=False):
        nonlocal sequence
        sequence+=1;tmp=root/'client-command.tmp';tmp.write_text(json.dumps({'id':sequence,'operation':operation,'text':text,'select':select}));tmp.replace(root/'client-command.json')
        for _ in range(400):
            try:
                value=json.loads((root/'client-state.json').read_text())
                if value['id']==sequence:return value
            except (OSError,ValueError):pass
            time.sleep(.005)
        raise AssertionError('GTK client did not answer')
    def expect_text(expected,clipboard):
        for _ in range(200):
            state=client()
            if state['active'] and state['focused'] and state['text']==expected and state['clipboard']==clipboard:return state
            time.sleep(.01)
        raise AssertionError(state)
    with (root/'kwin.log').open('w') as kl,(root/'client.log').open('w') as gl:
        try:
            kwin=subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-private-input'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=kl,stderr=kl);processes.append(kwin)
            for _ in range(150):
                if (root/'runtime/deckforge-private-input').exists():break
                assert kwin.poll() is None;time.sleep(.02)
            bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            def call(service,path,interface,method,signature,args):return bus.call_sync(service,path,interface,method,GLib.Variant(signature,args),None,Gio.DBusCallFlags.NONE,5000,None).unpack()
            for _ in range(100):
                try:
                    pid=call('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID','(s)',('org.kde.KWin',))[0]
                    assert pid==kwin.pid;break
                except GLib.Error:time.sleep(.03)
            def permission(allowed):
                # Only the disposable session bus and data directory created above.
                call('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','StartServiceByName','(su)',('org.freedesktop.impl.portal.PermissionStore',0))
                store_pid=call('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID','(s)',('org.freedesktop.impl.portal.PermissionStore',))[0]
                environment=Path(f'/proc/{store_pid}/environ').read_bytes().split(b'\0')
                assert ('XDG_DATA_HOME='+str(root/'data')).encode() in environment,'Permission store is not using private fixture data'
                call('org.freedesktop.impl.portal.PermissionStore','/org/freedesktop/impl/portal/PermissionStore','org.freedesktop.impl.portal.PermissionStore','SetPermission','(sbssas)',('kde-authorized',True,'remote-desktop','',['yes'] if allowed else []))
            permission(True)
            gtk_client=subprocess.Popen(['/usr/bin/python',str(project/'tests/focused_clipboard_client.py'),str(root)],stdout=gl,stderr=gl);processes.append(gtk_client)
            sentinel='Preserve clipboard 🌍';client('clipboard',sentinel);expect_text('',sentinel)
            host=Host(binary,[],root/'state',root/'host.log').ready()
            def configure(text,index=0,**extra):
                if not any(s['controller']=='Keypad' and s['index']==index and s['uuid']==TEXT for s in host.state()['slots']):host.post({'command':'assign','controller':'Keypad','index':index,'uuid':TEXT})
                action=next(s for s in host.state()['slots'] if s['controller']=='Keypad' and s['index']==index)
                settings={'pastedText':text,'isTypingMode':True,'isSendingEnter':False,**extra}
                host.post({'command':'settings','context':action['context'],'settings':settings})
            def phase(value):host.wait(lambda s:s['typing_status']['phase']==value,'typing '+value)
            configure('Should not type');tap(host)
            host.wait(lambda s:'Enable desktop typing' in s['system_status'],'permission required before typing');expect_text('',sentinel)
            host.post({'command':'text_typing_setup'});phase('ready');expect_text('',sentinel)
            # Model the desktop ending this granted session externally.
            paths=subprocess.check_output(['busctl','--user','tree','--list','org.freedesktop.portal.Desktop'],text=True).splitlines()
            sessions=[p.strip() for p in paths if '/session/' in p and p.rsplit('/',1)[-1].startswith('deckforge_')]
            assert len(sessions)==1,sessions
            call('org.freedesktop.impl.portal.desktop.kde',sessions[0],'org.freedesktop.impl.portal.Session','Close','()',())
            phase('disabled');expect_text('',sentinel)
            host.post({'command':'text_typing_setup'});phase('ready')
            text='ASCII æøå Ω 中文 🌍 e\u0301\tCRLF\r\nCR\rEnd'
            expected='ASCII æøå Ω 中文 🌍 e\u0301\tCRLF\nCR\nEnd\n'
            configure(text,isSendingEnter=True,Hotkey={'QTKeyCode':'ignored in typing mode'})
            tap(host);expect_text(expected,sentinel);phase('ready')
            # Sequential keys cannot interleave, even though the portal runs asynchronously.
            client('seed','',False);configure('First Ω\n');configure('Second 🌍',index=1)
            tap(host);host.post({'command':'input','event':'keyDown','index':1});host.post({'command':'input','event':'keyUp','index':1})
            expect_text('First Ω\nSecond 🌍',sentinel);phase('ready')
            configure('bad\u0001control');host.post({'command':'input','event':'keyDown','index':0});host.post({'command':'input','event':'keyUp','index':0},expected=400)
            expect_text('First Ω\nSecond 🌍',sentinel)
            client('seed','',False);configure('Ωλ'*8192);tap(host)
            for _ in range(100):
                if client()['text']:break
                time.sleep(.01)
            host.post({'command':'text_typing_disable'});phase('disabled')
            time.sleep(.1);stopped=client()['text'];assert 0<len(stopped)<16384
            time.sleep(.1);assert client()['text']==stopped and client()['clipboard']==sentinel
            # Cancel a real permission dialog through the product command, then recover.
            permission(False);host.post({'command':'text_typing_setup'});phase('requesting')
            for _ in range(200):
                if not client()['active']:break
                time.sleep(.01)
            else:raise AssertionError('Expected private permission dialog to take focus')
            host.post({'command':'text_typing_disable'});phase('disabled');expect_text(stopped,sentinel)
            permission(True);host.post({'command':'text_typing_setup'});phase('ready')
            client('seed','',False);configure('After cancel');tap(host);expect_text('After cancel',sentinel)
            terminal=subprocess.Popen(['konsole','--separate','-e','/usr/bin/python',str(project/'tests/focused_clipboard_terminal.py'),str(root)],stdout=gl,stderr=gl);processes.append(terminal)
            for _ in range(100):
                if (root/'terminal-ready.json').exists():break
                time.sleep(.02)
            else:raise AssertionError('Private terminal did not start')
            time.sleep(.2)
            configure('Direct terminal 🌍 æøå',isSendingEnter=True);tap(host)
            configure('Second typed line Ω',isSendingEnter=True);tap(host)
            for _ in range(300):
                if (root/'terminal-result.json').exists():break
                time.sleep(.01)
            terminal_result=json.loads((root/'terminal-result.json').read_text())
            assert terminal_result=={'tty':True,'lines':['Direct terminal 🌍 æøå\n','Second typed line Ω\n']},terminal_result
            phase('ready');expect_text('After cancel',sentinel)
            # Shutdown releases the portal and stops in-flight text.
            client('seed','',False);configure('Ωλ'*8192);tap(host)
            for _ in range(100):
                if client()['text']:break
                time.sleep(.01)
            host.stop();host=None;time.sleep(.1);stopped=client()['text'];time.sleep(.1)
            after=client();assert len(stopped)<16384 and after['text']==stopped and after['clipboard']==sentinel, {'length':len(stopped),'after_length':len(after['text']),'clipboard_preserved':after['clipboard']==sentinel}
            host=Host(binary,[],root/'state',root/'restart.log').ready();phase('disabled')
            assert slot(host.state(),TEXT)['settings']['isTypingMode'] is True
            print(json.dumps({'public_portal':True,'unicode_and_combining':True,'newline_normalization':True,'tab_and_enter':True,'clipboard_preserved':True,'no_typing_before_permission':True,'setup_sends_no_text':True,'external_revocation':True,'keyboard_fifo':True,'invalid_text_preflight':True,'cancel_stops_typing':True,'permission_dialog_cancel_recovery':True,'konsole_direct_typing':True,'shutdown_stops_typing':True,'restart_requires_session_permission':True,'uinput_used':False,'usb_used':False}))
        finally:
            if host:host.stop()
            for process in reversed(processes):
                if process.poll() is None:process.terminate()
                process.wait(timeout=5)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/text-typing'));p.add_argument('--inside',type=Path);a=p.parse_args();binary=a.binary.resolve()
    if a.inside:inside(a.inside,binary);return
    a.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
        root=Path(d);(root/'runtime').mkdir(mode=0o700);env=os.environ.copy();env.pop('DISPLAY',None)
        env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WAYLAND_DISPLAY='deckforge-private-input',GDK_BACKEND='wayland',QT_QPA_PLATFORM='wayland',LIBGL_ALWAYS_SOFTWARE='1')
        with (a.output/'run.log').open('w') as log:
            process=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--binary',str(binary)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:code=process.wait(timeout=55)
            finally:
                try:os.killpg(process.pid,signal.SIGTERM)
                except ProcessLookupError:pass
                process.wait(timeout=5)
        for name in ['kwin.log','client.log','host.log','restart.log']:
            if (root/name).exists():(a.output/name).write_text((root/name).read_text())
        assert code==0,(a.output/'run.log').read_text()[-6000:]
        report=next(json.loads(line) for line in reversed((a.output/'run.log').read_text().splitlines()) if line.startswith('{'))
    report['tested_binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest();(a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: clipboard-free portal Unicode typing, FIFO, cancel/dialog recovery, shutdown and restart through the actual release host; isolated GTK client, no uinput or USB.')

if __name__=='__main__':main()
