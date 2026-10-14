#!/usr/bin/env python3
"""Focused GTK clipboard acceptance through actual worker packets and private KWin EIS."""
import argparse,hashlib,json,os,shlex,signal,subprocess,sys,tempfile,time
from pathlib import Path

def wait(path,processes):
    for _ in range(150):
        if path.exists():return
        for process in processes:assert process.poll() is None,'Owned fixture exited early'
        time.sleep(.03)
    raise AssertionError(f'Timed out: {path.name}')

def inside(root,test_binary,bridge):
    project=Path(__file__).resolve().parents[1];processes=[]
    with (root/'kwin.log').open('w') as kl,(root/'bridge.log').open('w') as bl,(root/'client.log').open('w') as gl:
        try:
            kwin=subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-private-input'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=kl,stderr=kl);processes.append(kwin)
            wait(root/'runtime/deckforge-private-input',processes)
            env={**os.environ,'DECKFORGE_PRIVATE_KWIN_PID':str(kwin.pid),'DECKFORGE_PRIVATE_CLIPBOARD_TEST':str(root)}
            # Wait for the private compositor to own its private session-bus name.
            for _ in range(100):
                probe=subprocess.run(['busctl','--user','status','org.kde.KWin'],capture_output=True,text=True)
                if probe.returncode==0:break
                time.sleep(.03)
            client=subprocess.Popen(['/usr/bin/python',str(project/'tests/focused_clipboard_client.py'),str(root)],env=env,stdout=gl,stderr=gl);processes.append(client)
            relay=subprocess.Popen([str(bridge),str(root)],env=env,stdout=bl,stderr=bl);processes.append(relay)
            wait(root/'bridge-ready',processes)
            result=subprocess.run([str(test_binary),'focused_clipboard_on_private_wayland','--ignored','--nocapture'],env=env,capture_output=True,text=True,timeout=35)
            (root/'worker.log').write_text(result.stdout+'\n'+result.stderr)
            assert result.returncode==0,result.stdout+'\n'+result.stderr
            report=json.loads((root/'rust-proof.json').read_text())
            client.terminate();client.wait(timeout=5)
            terminal=subprocess.Popen(['konsole','--separate','-e','/usr/bin/python',str(project/'tests/focused_clipboard_terminal.py'),str(root)],env=env,stdout=gl,stderr=gl);processes.append(terminal)
            wait(root/'terminal-ready.json',[kwin,relay,terminal]);time.sleep(.3)
            result=subprocess.run([str(test_binary),'terminal_clipboard_on_private_wayland','--ignored','--nocapture'],env=env,capture_output=True,text=True,timeout=15)
            (root/'terminal-worker.log').write_text(result.stdout+'\n'+result.stderr)
            assert result.returncode==0,result.stdout+'\n'+result.stderr
            report.update(json.loads((root/'terminal-proof.json').read_text()));report['bridge']=json.loads((root/'bridge.json').read_text())
            assert report['bridge']['key_events']>0
            print(json.dumps(report))
        finally:
            for process in reversed(processes):
                if process.poll() is None:process.terminate()
                process.wait(timeout=5)

def main():
    project=Path(__file__).resolve().parents[1]
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--test-binary',type=Path,required=True);parser.add_argument('--output',type=Path,default=project/'target/focused-clipboard/acceptance');parser.add_argument('--inside',type=Path);parser.add_argument('--bridge',type=Path);parser.add_argument('--logical',action='store_true');parser.add_argument('--layout',default='us');parser.add_argument('--variant',default='');a=parser.parse_args()
    if a.inside:inside(a.inside,a.test_binary,a.bridge);return
    a.output.mkdir(parents=True,exist_ok=True);bridge=a.output.resolve()/'input-bridge';test_binary=a.test_binary.resolve()
    flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','Qt6DBus','libei-1.0'],text=True))
    subprocess.run(['g++','-fPIC','-std=c++17',str(project/'tests/focused_clipboard_bridge.cpp'),'-o',str(bridge),*flags],check=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-focused-') as d:
        root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'config').mkdir();(root/'config/kxkbrc').write_text('[Layout]\nUse=true\nLayoutList='+a.layout+'\nVariantList='+a.variant+'\nModel=pc105\nOptions=\n');env=os.environ.copy();env.pop('DISPLAY',None)
        env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WAYLAND_DISPLAY='deckforge-private-input',GDK_BACKEND='wayland',QT_QPA_PLATFORM='wayland',LIBGL_ALWAYS_SOFTWARE='1',DECKFORGE_TEST_LOGICAL_KEYS='1' if a.logical else '0')
        process=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--test-binary',str(test_binary),'--bridge',str(bridge)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
        try:
            stdout,stderr=process.communicate(timeout=45)
            (a.output/'run.log').write_text(stdout+'\n'+stderr)
            for name in ['kwin.log','bridge.log','client.log','worker.log','terminal-worker.log']:
                if (root/name).exists():(a.output/name).write_text((root/name).read_text())
            assert process.returncode==0,stdout+'\n'+stderr
            report=json.loads(stdout.strip().splitlines()[-1]);report['logical_mapping']=a.logical;report['layout']=a.layout;report['variant']=a.variant;report['test_binary_sha256']=hashlib.sha256(test_binary.read_bytes()).hexdigest()
            report['source_sha256']={name:hashlib.sha256((project/name).read_bytes()).hexdigest() for name in ['src/system_actions.rs','src/system_keyboard.rs','src/system_clipboard.rs','src/keyboard_layout.rs']}
            (a.output/'proof.json').write_text(json.dumps(report,indent=2)+'\n')
        finally:
            try:os.killpg(process.pid,signal.SIGTERM)
            except ProcessLookupError:pass
            process.wait(timeout=5)
    print('PASS: actual worker Copy/Cut/Paste and Unicode Text+Enter in GTK, terminal paste and custom Text shortcut in Konsole, through a private EIS relay. No real uinput or user desktop input.')

if __name__=='__main__':main()
