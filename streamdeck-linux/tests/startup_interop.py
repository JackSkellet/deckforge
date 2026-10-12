#!/usr/bin/env python3
"""Validate XDG login startup with an actual GIO launch and isolated user config.
No real login/logout, user autostart change, USB access or OS input injection.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/deckforge'))
    args = parser.parse_args()
    source = args.binary.resolve(strict=True)
    for tool in ('gio', 'desktop-file-validate'):
        assert shutil.which(tool), f'{tool} is required'
    user_entry = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'autostart/deckforge.desktop'
    def fingerprint(path):
        return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None
    original = fingerprint(user_entry)
    with tempfile.TemporaryDirectory(prefix='deckforge-startup-') as directory:
        root = Path(directory)
        binary = root / 'Deck forge % $ ` \\" binary'
        shutil.copy2(source, binary)
        state = root / 'session space = % $ ` \\" path'
        state.mkdir()
        env = os.environ.copy()
        env['XDG_CONFIG_HOME'] = str(root / 'config')
        env['XDG_DATA_HOME'] = str(root / 'data')
        entry = root / 'config/autostart/deckforge.desktop'
        def call(kind, command, required=True):
            argv=[str(binary),kind,command,'--state',str(state)]
            if kind=='startup':argv.append('--simulate')
            result = subprocess.run(argv, env=env,capture_output=True,text=True,timeout=5)
            if required:
                assert result.returncode == 0,result.stderr
                return json.loads(result.stdout)
            return result
        assert call('startup','status')['enabled'] is False
        assert not entry.parent.exists(), 'Read-only status created startup files'
        enabled = call('startup','enable')
        assert enabled['enabled'] and enabled['this_session'],enabled
        text = entry.read_text()
        assert ' --simulate\n' in text and ' --background --state ' in text,text
        assert 'Hidden=false\n' in text
        subprocess.run(['desktop-file-validate',str(entry)],check=True,capture_output=True,text=True)
        assert call('startup','enable') == enabled and entry.read_text() == text
        log = root/'launch.log'
        started = False
        try:
            with log.open('w') as output:
                subprocess.run(['gio','launch',str(entry)],env=env,stdout=output,
                               stderr=subprocess.STDOUT,timeout=8,check=True)
            deadline = time.monotonic()+15
            while time.monotonic()<deadline:
                result = call('window','status',False)
                if result.returncode == 0:
                    status=json.loads(result.stdout);started=True;break
                time.sleep(.05)
            assert started,log.read_text()
            assert not status['window_open'] and status['editor_generation'] == 0,status
            commandline=Path(f"/proc/{status['pid']}/cmdline").read_bytes().split(b'\0')
            assert os.fsencode(binary) in commandline and os.fsencode(state) in commandline and b'--simulate' in commandline,commandline
            session=json.loads((state/'session.json').read_text())
            assert 'simulated-streamdeck-plus' in json.dumps(session)
            call('window','quit')
            deadline=time.monotonic()+10
            while time.monotonic()<deadline and call('window','status',False).returncode==0:time.sleep(.05)
            assert call('window','status',False).returncode!=0,'Owned startup host failed to quit'
            started=False
        except Exception as error:
            raise AssertionError(str(error)+"\n"+log.read_text()+"\n"+entry.read_text()) from error
        finally:
            if started:call('window','quit',False)
        assert not call('startup','disable')['enabled']
        disabled=entry.read_text()
        assert 'Hidden=true\n' in disabled and '\nExec=' not in disabled
        subprocess.run(['desktop-file-validate',str(entry)],check=True,capture_output=True,text=True)
        foreign='[Desktop Entry]\nType=Application\nName=Custom\nExec=custom-program\n'
        entry.write_text(foreign)
        assert call('startup','enable',False).returncode!=0 and call('startup','disable',False).returncode!=0
        assert entry.read_text()==foreign,'Custom entry was overwritten'
    assert fingerprint(user_entry)==original,'Real user startup preference changed'
    print('PASS: default off/read-only, enable/disable, desktop validation, actual GIO launch with special-character paths, exact state and simulation arguments, background without window, normal quit and foreign-file preservation. User startup untouched; no USB or login/logout.')


if __name__=='__main__':main()
