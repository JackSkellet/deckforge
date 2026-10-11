#!/usr/bin/env python3
"""Exercise real System actions with local HTTP/program fixtures, no USB or shortcuts.

The app runs its real Rust executor. Only test-owned localhost requests and a
benign temporary marker program execute. No default browser is opened.
"""
import argparse
import http.server
import json
from pathlib import Path
import re
import tempfile
import threading
import time
import zipfile
from urllib.request import urlopen

from sample_interop import RunningHost, require, slot, tap

PREFIX = 'com.elgato.streamdeck.system.'

class Host(RunningHost):
    def ready(self):
        until = time.monotonic() + 20
        while time.monotonic() < until:
            self.check_process()
            match = re.search(r'Editor (http://127\.0\.0\.1:\d+)', self.log.read_text())
            if match:
                self.base = match.group(1)
                with urlopen(self.base, timeout=3) as response:
                    html = response.read().decode()
                self.token = re.search(r"const\s+token\s*=\s*['\"]([^'\"]+)['\"]", html).group(1)
                return self
            time.sleep(.04)
        raise AssertionError(self.log.read_text())

class Requests(http.server.BaseHTTPRequestHandler):
    paths = []
    def do_GET(self):
        self.paths.append(self.path)
        if self.path == '/slow':
            time.sleep(1)
        self.send_response(503 if self.path == '/failure' else 204)
        self.end_headers()
    def log_message(self, *args):
        pass


def assign(host, name, settings):
    host.post({'command':'assign','controller':'Keypad','index':0,'uuid':PREFIX+name})
    context = slot(host.state(),PREFIX+name)['context']
    host.post({'command':'settings','context':context,'settings':settings})
    return context


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'))
    args=parser.parse_args()
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Requests)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    base=f'http://127.0.0.1:{server.server_port}'
    try:
        with tempfile.TemporaryDirectory(prefix='deckforge-system-') as directory:
            root=Path(directory)
            host=Host(args.binary.resolve(),[],root/'state',root/'host.log')
            try:
                host.ready()
                require(not host.state()['slots'],'Builtins should not auto-assign themselves on a new deck')
                assign(host,'website',{'path':base+'/ok','openInBrowser':False})
                host.post({'command':'input','event':'keyDown','index':0})
                require(not Requests.paths,'Website triggered before release')
                host.post({'command':'input','event':'keyUp','index':0})
                host.wait(lambda s:'204' in s['system_status'],'real local GET')
                host.post({'command':'input','event':'keyUp','index':0})
                require(Requests.paths==['/ok'],'Unowned release repeated action')
                assign(host,'website',{'path':base+'/failure','openInBrowser':False})
                tap(host)
                host.wait(lambda s:'failed' in s['system_status'] and '503' in s['system_status'],'HTTP error surfaced')
                assign(host,'website',{'path':base+'/slow','openInBrowser':False})
                tap(host)
                start=time.monotonic()
                host.post({'command':'page_create','name':'Concurrent page'})
                require(time.monotonic()-start < .7,'Slow GET blocked native host commands')
                marker=root/'marker'
                program=root/'program with spaces'
                program.write_text('#!/usr/bin/python3\nfrom pathlib import Path\nPath('+repr(str(marker))+').write_text("launched")\n')
                program.chmod(0o700)
                assign(host,'open',{'path':str(program)})
                tap(host)
                host.wait(lambda s:marker.exists(),'actual test program launch')
                require(marker.read_text()=='launched','Exact program did not run')
                assign(host,'open',{'path':'C:\\Windows\\notepad.exe'})
                host.post({'command':'input','event':'keyDown','index':0})
                host.post({'command':'input','event':'keyUp','index':0},expected=400)
                # Release after leaving a page must not execute its website.
                assign(host,'website',{'path':base+'/cancelled','openInBrowser':False})
                host.post({'command':'input','event':'keyDown','index':0})
                host.post({'command':'page_create','name':'After held control'})
                host.post({'command':'input','event':'keyUp','index':0})
                require('/cancelled' not in Requests.paths,'Departing action fired on late release')
                # A shipped null Settings value must remain null on export.
                profile=root/'null.streamDeckProfile'
                with zipfile.ZipFile(profile,'w') as archive:
                    archive.writestr('manifest.json',json.dumps({'Name':'Null settings','Controllers':[{'Type':'Keypad','Actions':{'0,0':{'UUID':PREFIX+'keybrightness','Settings':None,'States':[]}}}]}))
                host.post({'command':'import_profile','path':str(profile)})
                require(slot(host.state(),PREFIX+'keybrightness')['settings']=={},'Null not normalized for editor')
                exported=root/'export.streamDeckProfile'
                host.post({'command':'export_profile','path':str(exported)})
                with zipfile.ZipFile(exported) as archive:
                    actions=[a for name in archive.namelist() if name.endswith('manifest.json') for c in json.loads(archive.read(name)).get('Controllers',[]) for a in c.get('Actions',{}).values()]
                require(any(a['UUID']==PREFIX+'keybrightness' and a['Settings'] is None for a in actions),'Export lost raw null settings')
            finally:
                host.stop()
                require(host.process.returncode==0,'Host did not shut down cleanly')
        print('PASS: real local GET success/failure, responsive host during slow HTTP, exact Linux program launch, Windows path rejection, release ownership and null-settings export preservation.')
        print('No USB, browser, keyboard shortcuts or media controls executed by this suite.')
    finally:
        server.shutdown()
        server.server_close()

if __name__=='__main__':
    main()
