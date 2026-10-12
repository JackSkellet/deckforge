#!/usr/bin/env python3
"""Install and run our native Discord plugin through the actual Rust host.

No Discord account/socket/auth operation is requested. All input is simulated.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import threading
import time
import zipfile

from system_interop import Host
from sample_interop import require

ID = 'com.elgato.discord'


class InspectorProof:
    """Observe only offline flags from our temporary inspector, without SDK writes."""
    def __init__(self, plugin):
        self.events = []
        owner = self
        class Receiver(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                size = int(self.headers.get('Content-Length', '0'))
                require(0 < size <= 2048, 'Oversized inspector proof')
                owner.events.append(json.loads(self.rfile.read(size)))
                self.send_response(204)
                self.send_header('Access-Control-Allow-Origin', '*')
                self.end_headers()
            def log_message(self, *args):
                pass
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Receiver)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        endpoint = f'http://127.0.0.1:{self.server.server_port}/offline-proof'
        # Append a test observer to the installed temporary copy. The native UI,
        # registration, plugin and Host all execute normally. No auth values or
        # arbitrary messages are transmitted, and no plugin settings are changed.
        script = plugin/'ui/app.js'
        script.write_text(script.read_text() + """
const fixtureConnect = window.connectElgatoStreamDeckSocket;
window.connectElgatoStreamDeckSocket = (...args) => {
 fixtureConnect(...args);
 const context = JSON.parse(args[4]).context;
 socket.addEventListener('message', event => {
  const message = JSON.parse(event.data);
  if (message.event !== 'sendToPropertyInspector') return;
  const p = message.payload ?? {};
  fetch(ENDPOINT, {method:'POST', headers:{'Content-Type':'text/plain'}, body:JSON.stringify({
   context, disabled:p.disabled===true, unauthorized:p.unauthorized===true,
   offline:p.auth?.connected===false && p.auth?.authenticated===false,
   rejected:p.error==="Connect and authorize Discord in this action's settings."
  })}).catch(()=>{});
 });
};
""".replace('ENDPOINT', json.dumps(endpoint)))

    def wait(self, host, context, field, after=0):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            host.check_process()
            if any(e.get('context') == context and e.get(field) is True for e in self.events[after:]):
                return
            time.sleep(.02)
        raise AssertionError(f'No explicit inspector {field} proof for {context}')

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def assert_offline_icons(host, proof):
    checked = []
    for index, suffix in enumerate(('videotoggle', 'streamtoggle', 'channel.text'), 1):
        action = ID + '.' + suffix
        host.post({'command':'assign','controller':'Keypad','index':index,'uuid':action})
        context = next(s['context'] for s in host.state()['slots'] if s['uuid'] == action)
        host.post({'command':'open_inspector','context':context})
        proof.wait(host, context, 'offline')
        proof.wait(host, context, 'disabled')
        proof.wait(host, context, 'unauthorized')
        start = len(proof.events)
        host.post({'command':'input','event':'keyDown','index':index})
        # Wait for the actual Rust plugin to reject the down event as offline.
        # Otherwise a delayed setState(0) could mask a host keyUp auto-toggle.
        proof.wait(host, context, 'rejected', after=start)
        host.wait(lambda s: next(a for a in s['slots'] if a['context']==context)['state']==0,
                  suffix+' offline baseline')
        time.sleep(.1)
        host.post({'command':'input','event':'keyUp','index':index})
        until = time.monotonic() + .3
        while time.monotonic() < until:
            state = next(s for s in host.state()['slots'] if s['context']==context)['state']
            require(state == 0, f'{suffix} falsely alternated its offline icon on release')
            time.sleep(.02)
        host.post({'command':'close_inspector','context':context})
        checked.append(suffix)
    return checked


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=root/'target/release/deckforge')
    parser.add_argument('--package',type=Path,default=root/'target/release/discord-for-deckforge.streamDeckPlugin')
    parser.add_argument('--report',type=Path)
    args=parser.parse_args()
    binary,package=args.binary.resolve(strict=True),args.package.resolve(strict=True)
    report={'host_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'package_sha256':hashlib.sha256(package.read_bytes()).hexdigest(),
            'boundary':'Real native ELF child, host/PI and persistence. Simulation only; no Discord connection or auth requested.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-native-discord-') as directory:
        temp=Path(directory)
        subprocess.run([str(binary),'install',str(package),'--destination',str(temp/'plugins')],check=True,capture_output=True,text=True)
        plugin=temp/'plugins'/f'{ID}.sdPlugin'
        executable=plugin/'bin/deckforge-discord'
        require(executable.read_bytes()[:4]==b'\x7fELF','Plugin is not a native Linux executable')
        require(executable.stat().st_mode&0o100 != 0,'Installer dropped executable permission')
        manifest=json.loads((plugin/'manifest.json').read_text())
        require(all(a.get('DisableAutomaticStates') is True for a in manifest['Actions']),
                'Discord action manifest permits blind automatic state changes')
        proof=InspectorProof(plugin)
        host=Host(binary,[plugin],temp/'state',temp/'host.log')
        try:
            host.ready()
            snapshot=host.wait(lambda s:any(p['id']==ID and p['connected'] for p in s['plugins']),'native Discord registration')
            p=next(p for p in snapshot['plugins'] if p['id']==ID)
            require(Path(f'/proc/{p["pid"]}/exe').resolve()==executable,'Host did not run the native plugin directly')
            require(len([a for a in snapshot['actions'] if a['plugin']==ID])==17,'Original action identities missing')
            first_pid=p['pid']
            report['native_pid']=first_pid
            report['plugin_sha256']=hashlib.sha256(executable.read_bytes()).hexdigest()
            for controller,index,uuid in [('Keypad',0,ID+'.mute'),('Encoder',0,ID+'.volumecontrol')]:
                host.post({'command':'assign','controller':controller,'index':index,'uuid':uuid})
            selected=next(s for s in host.state()['slots'] if s['uuid']==ID+'.mute')
            context=selected['context']
            host.post({'command':'settings','context':context,'settings':{'state':0,'foreignSetting':'retained'}})
            host.post({'command':'open_inspector','context':context})
            host.wait(lambda s:any(i['context']==context and i['connected'] for i in s['inspectors']),'native settings inspector')
            host.post({'command':'input','event':'keyDown','index':0})
            host.post({'command':'input','event':'keyUp','index':0})
            private=temp/'state/plugin-data'/hashlib.sha256(ID.encode()).hexdigest()
            require(stat.S_IMODE(private.stat().st_mode)==0o700,'Plugin data directory is not private')
            require(not (private/'discord-auth.json').exists(),'Actions wrote credentials without Connect')
            report['offline_icons_no_automatic_toggle']=assert_offline_icons(host,proof)
            auth_file=private/'discord-auth.json'
            fixtures=['native-secret-not-exported','native-token-not-exported','native-refresh-not-exported']
            auth_file.write_text(json.dumps({'client_id':'123456789012345678','client_secret':fixtures[0],
                'redirect_uri':'','scopes':['rpc'],'token':{'access_token':fixtures[1],
                'refresh_token':fixtures[2],'expires_at':int(time.time())+3600}}))
            auth_file.chmod(0o600)
            host.post({'command':'restart_plugin','id':ID})
            snapshot=host.wait(lambda s:any(p['id']==ID and p['connected'] and p['pid']!=first_pid for p in s['plugins']),'native plugin restart')
            require(not Path(f'/proc/{first_pid}').exists(),'Previous plugin child was not reaped')
            saved=next(s for s in snapshot['slots'] if s['uuid']==ID+'.mute')
            require(saved['context']==context and saved['settings']['foreignSetting']=='retained','Restart lost compatible settings/context')
            require(not snapshot['inspectors'],'Restart left stale inspector bindings')
            export=temp/'portable.streamDeckProfile'
            host.post({'command':'export_profile','path':str(export)})
            with zipfile.ZipFile(export) as archive:
                data=b''.join(archive.read(n) for n in archive.namelist() if n.endswith('.json'))
                require(b'client_secret' not in data and b'access_token' not in data,'Export contains credentials')
                session=(temp/'state/session.json').read_bytes()
                require(all(secret.encode() not in data+session for secret in fixtures),'Private credentials leaked into host state/profile export')
            require('SDK command rejected' not in (temp/'host.log').read_text(),'Native plugin sent rejected SDK commands')
        finally:
            host.stop()
            proof.close()
            require(host.process.returncode==0,'Host did not shut down cleanly')
        host=Host(binary,[],temp/'state',temp/'restart.log')
        try:
            host.ready()
            state=host.wait(lambda s:any(p['id']==ID and p['connected'] for p in s['plugins']),'saved native plugin restart')
            require(next(s for s in state['slots'] if s['uuid']==ID+'.mute')['settings']['foreignSetting']=='retained','Host restart lost native plugin settings')
        finally:
            host.stop()
    if args.report:
        args.report.parent.mkdir(parents=True,exist_ok=True)
        args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: native ELF package install/registration,17 compatible actions, native inspector, offline video/share/text icons do not auto-toggle, no auto-connect, private data path, child restart/reaping, settings/context retention and credential-free profile export. No real Discord or USB operation.')


if __name__=='__main__':
    main()
