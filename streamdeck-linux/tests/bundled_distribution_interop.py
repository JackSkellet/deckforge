#!/usr/bin/env python3
"""Validate the shipped archive, isolated install, and native bundled-plugin button."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import zipfile
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const bridge=window.deckforge,pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(fn,label)=>{for(let n=0;n<200;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+label)};
const initial=await bridge.request('snapshot');if(!initial.simulate||initial.plugins.length)throw Error('Requires a fresh simulated session');
const packages=await bridge.request('bundled_plugins');if(packages.length!==1||packages[0].id!=='com.elgato.discord')throw Error('Bundled package discovery failed');
[...document.querySelectorAll('button')].find(b=>b.textContent.trim().startsWith('Plugins')).click();
const install=await until(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Install Discord for Deckforge'),'bundled Install button');install.click();
const installed=await until(async()=>{const s=await bridge.request('snapshot');return s.plugins.some(p=>p.id==='com.elgato.discord'&&p.connected)&&s},'native Discord registration');
if(installed.actions.filter(a=>a.plugin==='com.elgato.discord').length!==17)throw Error('Discord action catalog incomplete');
await until(()=>[...document.querySelectorAll('button')].some(b=>b.textContent.trim()==='Already installed'&&b.disabled),'existing install protected');
return {native_install_button:true,discord_registered:true,action_count:17,existing_install_protected:true,usb_access:false};
'''


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive',type=Path,required=True)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    report={'ok':False,'archive_sha256':hashlib.sha256(args.archive.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix='deckforge-distribution-') as temporary:
        root=Path(temporary)
        with tarfile.open(args.archive) as archive:
            members=archive.getmembers()
            assert all(m.isfile() and not Path(m.name).is_absolute() and '..' not in Path(m.name).parts for m in members)
            assert sum(m.size for m in members)<512*1024*1024
            archive.extractall(root,filter='data')
        distribution=next(root.glob('deckforge-linux-*'))
        hashes=json.loads((distribution/'SHA256SUMS.json').read_text())
        assert set(hashes)=={'deckforge','assets/deckforge.svg','assets/deckforge.png','assets/deckforge-mime.xml','scripts/install-desktop.py','LICENSE','README.md','bundled-plugins/discord-for-deckforge.streamDeckPlugin'}
        for name,digest in hashes.items():assert hashlib.sha256((distribution/name).read_bytes()).hexdigest()==digest
        with zipfile.ZipFile(distribution/'bundled-plugins/discord-for-deckforge.streamDeckPlugin') as package:
            assert not any('auth.json' in name or 'plugin-data/' in name or 'session.json' in name for name in package.namelist())
        # Install under a fresh HOME. Existing session/private data are opaque to the installer.
        home=root/'home';home.mkdir();data=home/'data';(data/'deckforge').mkdir(parents=True)
        sentinel=data/'deckforge/session.json';sentinel.write_text('retained user state')
        private=data/'deckforge/plugin-data/private';private.mkdir(parents=True)
        secret=private/'discord-auth.json';secret.write_text('private sentinel');secret.chmod(0o600)
        existing=data/'deckforge/plugins/com.elgato.discord.sdPlugin';existing.mkdir(parents=True)
        (existing/'manifest.json').write_text('existing plugin sentinel')
        env=environment(root)
        env.update(HOME=str(home),XDG_DATA_HOME=str(data),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(root/'probe.js'),DECKFORGE_WEBVIEW_TEST_RESULT=str(root/'proof.json'),DECKFORGE_CAPTURE=str(root/'native.png'))
        subprocess.run(['python3',str(distribution/'scripts/install-desktop.py')],env=env,check=True,stdout=subprocess.PIPE,text=True)
        assert sentinel.read_text()=='retained user state' and secret.read_text()=='private sentinel'
        assert (existing/'manifest.json').read_text()=='existing plugin sentinel'
        assert (data/'deckforge/bundled-plugins/discord-for-deckforge.streamDeckPlugin').is_file()
        binary=home/'.local/bin/deckforge'
        assert hashlib.sha256(binary.read_bytes()).hexdigest()==hashes['deckforge']
        (root/'probe.js').write_text(SCRIPT)
        state=root/'fresh-state'
        with (root/'native.log').open('w') as log:
            process=subprocess.Popen([str(binary),'desktop','--simulate','--state',str(state)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=40)==0,(root/'native.log').read_text()
                report['native']=result(root/'proof.json')['result']
            finally:cleanup(process)
        assert not list(state.rglob('discord-auth.json')),'Fresh plugin created authorization without Connect'
        plugin=state/'plugins/com.elgato.discord.sdPlugin/bin/deckforge-discord'
        assert plugin.is_file() and os.access(plugin,os.X_OK)
        report.update(ok=True,checksums_verified=True,isolated_install_preserves_existing=True,authorization_absent=True,host_sha256=hashes['deckforge'])
        args.report.parent.mkdir(parents=True,exist_ok=True)
        args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: distribution, isolated installer, native bundled install, 17 actions and no authorization state.')


if __name__=='__main__':main()
