#!/usr/bin/env python3
"""Real command, Konsole and clipboard execution in a private virtual Wayland desktop."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
from system_interop import Host
from sample_interop import slot, tap

CLIPBOARD = 'com.deckforge.system.clipboard'
COMMAND = 'com.deckforge.system.command'

READ_CLIPBOARD = '''
import gi,json
gi.require_version('Gtk','3.0')
from gi.repository import Gtk,Gdk,GLib
assert Gtk.init_check()[0]
window=Gtk.Window(); window.add(Gtk.Entry()); window.show_all()
def read():
    print(json.dumps(Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).wait_for_text()),flush=True)
    Gtk.main_quit()
    return False
GLib.timeout_add(300,read)
Gtk.main()
'''

def wait_file(path):
    for _ in range(160):
        if path.exists(): return json.loads(path.read_text())
        time.sleep(.05)
    raise AssertionError(f'Missing command output: {path}')

def inside(binary, root):
    with (root/'kwin.log').open('w') as log:
        compositor = subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-test'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=log,stderr=log)
        host = None
        try:
            for _ in range(100):
                if (Path(os.environ['XDG_RUNTIME_DIR'])/'deckforge-test').exists(): break
                assert compositor.poll() is None, (root/'kwin.log').read_text()
                time.sleep(.05)
            time.sleep(.3)
            host = Host(binary, [], root/'state', root/'host.log').ready()
            def assign(uuid, settings):
                host.post({'command':'assign','controller':'Keypad','index':0,'uuid':uuid})
                action = slot(host.state(),uuid)
                host.post({'command':'settings','context':action['context'],'settings':settings})

            text = '  Hei 🌍\næøå\tclipboard  '
            assign(CLIPBOARD, {'operation':'copy_text','text':text})
            tap(host)
            host.wait(lambda s:'Text copied to desktop clipboard' in s['system_status'],'copy without keyboard')
            result = subprocess.run(['/usr/bin/python','-c',READ_CLIPBOARD],capture_output=True,text=True,timeout=8,check=True)
            assert json.loads(result.stdout) == text, result
            # An empty saved string must replace previous contents too.
            assign(CLIPBOARD, {'operation':'copy_text','text':''})
            tap(host)
            host.wait(lambda s:'Text copied to desktop clipboard' in s['system_status'],'empty clipboard')
            result = subprocess.run(['/usr/bin/python','-c',READ_CLIPBOARD],capture_output=True,text=True,timeout=8,check=True)
            assert json.loads(result.stdout) == '', result

            working = root/'working folder'; working.mkdir()
            marker = working/'program.json'
            script = root/'program with spaces.py'
            script.write_text('import json,os,sys\nfrom pathlib import Path\nPath(sys.argv[1]).write_text(json.dumps({"cwd":os.getcwd(),"args":sys.argv[2:],"tty":sys.stdin.isatty()}))\n')
            literal = '$(touch unwanted); $HOME \\ "quoted" \'single\'\nnext line'
            settings = {'program':'/usr/bin/python','args':shlex.join([str(script),str(marker),'two words',literal]),'directory':str(working)}
            assign(COMMAND,settings)
            assert not marker.exists(), 'Saving executed a command'
            host.post({'command':'input','event':'keyDown','index':0})
            assert not marker.exists(), 'Command ran before release'
            host.post({'command':'input','event':'keyUp','index':0})
            report = wait_file(marker)
            assert report == {'cwd':str(working),'args':['two words',literal],'tty':False}, report
            assert not (working/'unwanted').exists()
            marker.unlink()
            host.post({'command':'input','event':'keyUp','index':0})
            time.sleep(.15)
            assert not marker.exists(), 'Stray release repeated the command'

            # Existing real worker must stay responsive while a command runs.
            assign(COMMAND, {'mode':'shell','script':"sleep .3; printf '%s' ok | tr a-z A-Z > shell-result",'directory':str(working)})
            tap(host)
            start=time.monotonic(); host.post({'command':'page_create','name':'During command'})
            assert time.monotonic()-start < .25
            for _ in range(100):
                if (working/'shell-result').exists():break
                time.sleep(.03)
            assert (working/'shell-result').read_text() == 'OK'

            assign(COMMAND, {'mode':'shell','script':'exit 7','directory':str(working)})
            tap(host)
            host.wait(lambda s:'failed' in s['system_status'] and '7' in s['system_status'],'nonzero exit reported')

            # Konsole runs on the private Wayland socket and private session bus.
            settings.update(terminal=True, terminal_program='konsole', terminal_args='--separate -e')
            assign(COMMAND,settings); tap(host)
            report=wait_file(marker)
            assert report == {'cwd':str(working),'args':['two words',literal],'tty':True}, report

            def clipboard():
                result=subprocess.run(['/usr/bin/python','-c',READ_CLIPBOARD],capture_output=True,text=True,timeout=8,check=True)
                return json.loads(result.stdout)
            def capture(settings):
                assign(COMMAND,settings);tap(host)
                host.wait(lambda s:slot(s,COMMAND).get('command_run',{}).get('state') not in [None,'running'],'capture finishes')
                return slot(host.state(),COMMAND)['command_run']

            text='Exact output 🌍\nwith trailing newline\n'
            source=root/'stdout.txt';source.write_text(text)
            output_settings={'program':'/bin/cat','args':shlex.quote(str(source)),'output':'clipboard','timeout_seconds':2,'directory':str(working)}
            captured=capture(output_settings)
            assert captured['can_copy'] and captured['state']=='succeeded'
            assert clipboard()==text
            assert 'stdout' not in captured and 'stderr' not in captured, 'Snapshot contains output contents'
            context=slot(host.state(),COMMAND)['context'];old_id=captured['id']
            host.post({'command':'command_copy_output','context':context,'id':old_id})
            host.wait(lambda s:'Command output copied' in s['system_status'],'explicit output copy')
            assert clipboard()==text

            for script,expected in [('printf failure; exit 7','failed'),("printf '\\377'",'succeeded'),("printf '\\000'",'succeeded'),('sleep 5','timed_out')]:
                captured=capture({'mode':'shell','script':script,'output':'clipboard','timeout_seconds':1})
                assert captured['state']==expected and not captured['can_copy'],captured
                assert clipboard()==text,'Failed or invalid command replaced clipboard'

            assign(COMMAND,{'mode':'shell','script':'sleep 5 & wait','output':'clipboard','timeout_seconds':5});tap(host)
            host.wait(lambda s:slot(s,COMMAND).get('command_run',{}).get('state')=='running','running captured command')
            running=slot(host.state(),COMMAND)
            host.post({'command':'command_cancel','context':running['context'],'id':running['command_run']['id']})
            host.wait(lambda s:slot(s,COMMAND)['command_run']['state']=='cancelled','captured command stopped')
            assert clipboard()==text
            captured=capture({'program':'/bin/true','output':'clipboard','timeout_seconds':2})
            assert captured['can_copy'] and clipboard()==''
            # Outputs are runtime-only, including the activity log.
            assert text not in (root/'state/session.json').read_text()
            assert text not in (root/'host.log').read_text()

            bindir=root/'command-bin';bindir.mkdir()
            (bindir/'deckforge=env-python').symlink_to('/usr/bin/python')
            envscript=root/'env-reader.py';envresult=root/'env-proof.json'
            envscript.write_text('import os,json,sys\nfrom pathlib import Path\nr={k:os.environ.get(k) for k in ["DECKFORGE_ENV_VALUE","DECKFORGE_ENV_EMPTY","DECKFORGE_ENV_REMOVE","DECKFORGE_ENV_KEEP","PATH","QT_QPA_PLATFORM"]}\nr["tty"]=sys.stdin.isatty()\nPath(sys.argv[1]).write_text(json.dumps(r))\nprint(json.dumps(r))\n')
            literal_env="  $DECKFORGE_ENV_KEEP 'quoted' $(not-a-command)\n🌍  "
            overrides=[{'name':'DECKFORGE_ENV_VALUE','value':literal_env},{'name':'DECKFORGE_ENV_EMPTY','value':''},{'name':'DECKFORGE_ENV_REMOVE','unset':True,'value':'retained inactive value'},{'name':'PATH','value':str(bindir)},{'name':'QT_QPA_PLATFORM','value':'deckforge-invalid-target-backend'}]
            for terminal in [False,True]:
                envresult.unlink(missing_ok=True)
                assign(COMMAND,{'program':'deckforge=env-python','args':shlex.join([str(envscript),str(envresult)]),'environment':overrides,'output':'show','terminal':terminal,'terminal_program':'konsole','terminal_args':'--separate -e'})
                tap(host);report=wait_file(envresult)
                assert report=={'DECKFORGE_ENV_VALUE':literal_env,'DECKFORGE_ENV_EMPTY':'','DECKFORGE_ENV_REMOVE':None,'DECKFORGE_ENV_KEEP':'inherited-kept','PATH':str(bindir),'QT_QPA_PLATFORM':'deckforge-invalid-target-backend','tty':terminal},report
                if not terminal:host.wait(lambda s:slot(s,COMMAND)['command_run']['state']=='succeeded','environment capture completed')
            # A following command inherits the original environment, not previous overrides.
            envresult.unlink(missing_ok=True)
            assign(COMMAND,{'program':'/usr/bin/python','args':shlex.join([str(envscript),str(envresult)]),'output':'show'})
            tap(host);report=wait_file(envresult)
            assert report['DECKFORGE_ENV_VALUE'] is None and report['DECKFORGE_ENV_REMOVE']=='inherited-remove' and report['PATH']==os.environ['PATH'] and report['QT_QPA_PLATFORM']=='wayland',report
            host.wait(lambda s:slot(s,COMMAND)['command_run']['state']=='succeeded','inherited environment command completed')
            pidfile=root/'capture.pid'
            settings={'mode':'shell','script':'echo $$ > '+shlex.quote(str(pidfile))+'; sleep 10','output':'show','timeout_seconds':30}
            assign(COMMAND,settings);tap(host)
            for _ in range(100):
                if pidfile.exists():break
                time.sleep(.02)
            pid=int(pidfile.read_text())
            host.stop(); host=None
            for _ in range(100):
                stat=Path(f'/proc/{pid}/stat')
                if not stat.exists() or stat.read_text().rsplit(')',1)[1].lstrip().startswith('Z'):break
                time.sleep(.01)
            else:raise AssertionError('Captured command survived host shutdown')
            host=Host(binary, [], root/'state', root/'restart.log').ready()
            assert slot(host.state(),COMMAND)['settings']==settings
            assert slot(host.state(),COMMAND).get('command_run') is None
            print(json.dumps({'clipboard_unicode_exact':True,'empty_clipboard':True,'no_keyboard_needed_for_copy':True,'program_arguments_literal':True,'working_directory':True,'shell_pipe_redirection':True,'nonzero_exit_reported':True,'terminal_tty':True,'settings_restart':True,'captured_output_clipboard':True,'failure_preserves_clipboard':True,'timeout_and_stop':True,'shutdown_stops_capture':True,'outputs_not_persisted_or_logged':True,'environment_literal_background_and_terminal':True,'target_only_environment_and_path':True,'empty_and_unset_environment':True,'host_environment_preserved':True,'usb_access':False,'user_clipboard_access':False}))
        finally:
            if host: host.stop()
            compositor.terminate(); compositor.wait(timeout=5)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    parser.add_argument('--output',type=Path,default=Path('target/os-actions'))
    parser.add_argument('--inside',type=Path)
    args=parser.parse_args();binary=args.binary.resolve()
    if args.inside: inside(binary,args.inside);return
    args.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-os-actions-') as d:
        root=Path(d);runtime=root/'runtime';runtime.mkdir(mode=0o700)
        env=os.environ.copy();env.pop('DISPLAY',None)
        env.update(XDG_RUNTIME_DIR=str(runtime),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),XDG_STATE_HOME=str(root/'private-state'),WAYLAND_DISPLAY='deckforge-test',GDK_BACKEND='wayland',QT_QPA_PLATFORM='wayland',LIBGL_ALWAYS_SOFTWARE='1')
        env.update(DECKFORGE_ENV_REMOVE='inherited-remove',DECKFORGE_ENV_KEEP='inherited-kept')
        process=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--binary',str(binary),'--inside',str(root)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
        try:
            stdout,stderr=process.communicate(timeout=50)
            (args.output/'interop.log').write_text(stdout+'\n'+stderr)
            for name in ['kwin.log','host.log','restart.log']:
                if (root/name).exists():(args.output/name).write_text((root/name).read_text())
            assert process.returncode==0,stdout+'\n'+stderr
            proof=json.loads(stdout.strip().splitlines()[-1]);proof['tested_binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
            (args.output/'interop.json').write_text(json.dumps(proof,indent=2)+'\n')
        finally:
            # Only this test's session group, including a terminal left after failure.
            try: os.killpg(process.pid,signal.SIGTERM)
            except ProcessLookupError: pass
            process.wait(timeout=5)
    print('PASS: isolated Wayland clipboard, literal program arguments, shell pipeline, working folder, real Konsole TTY, exit errors and restart. No user clipboard or USB access.')

if __name__=='__main__':main()
