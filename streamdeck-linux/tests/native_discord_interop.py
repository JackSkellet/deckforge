#!/usr/bin/env python3
"""Install and run our native Discord plugin through the actual Rust host.

No Discord account/socket/auth operation is requested. All input is simulated.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time
import zipfile

from system_interop import Host
from sample_interop import require

ID = 'com.elgato.discord'


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
    print('PASS: native ELF package install/registration,17 compatible actions, native inspector, no auto-connect, private data path, child restart/reaping, settings/context retention and credential-free profile export. No real Discord or USB operation.')


if __name__=='__main__':
    main()
