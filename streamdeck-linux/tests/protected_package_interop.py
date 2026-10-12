#!/usr/bin/env python3
"""Authenticate and verify a supplied protected package in temporary directories.

No original executable is launched: the simulated host must diagnose the supplied
Windows/macOS backend as unsupported, with no child PID. User state is never used.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile
from system_interop import Host

PACKAGE_HASH = '9d063e9b3616693093e565f762cdc65bd5638bdbb9f42e535e1c738b429302cd'
ROOT = 'com.elgato.discord.sdPlugin'
# Required/optional membership was established from the authenticated pinned
# package. These are filenames, not decrypted manifest or vendor-code fixtures.
REQUIRED = ROOT+'/ESDDiscord.exe'
OPTIONAL = ROOT+'/images/category.svg'

def variant(source, target, member, mode):
    with zipfile.ZipFile(source) as incoming, zipfile.ZipFile(target,'w',zipfile.ZIP_DEFLATED) as output:
        for item in incoming.infolist():
            if item.filename==member and mode=='missing':continue
            data=incoming.read(item)
            if item.filename==member:data+=b'\0fixture alteration'
            output.writestr(item,data)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--package',type=Path,required=True)
    parser.add_argument('--reference-app',type=Path,required=True)
    parser.add_argument('--report',type=Path)
    args=parser.parse_args()
    binary,package,reference=(p.resolve(strict=True)for p in(args.binary,args.package,args.reference_app))
    assert hashlib.sha256(package.read_bytes()).hexdigest()==PACKAGE_HASH
    report={'package_sha256':PACKAGE_HASH,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'reference_sha256':hashlib.sha256(reference.read_bytes()).hexdigest(),'scope':'Inert temporary installs and simulated host; no vendor executable, actual Discord or USB use.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-protected-') as directory:
        root=Path(directory);wrong=root/'wrong.exe';wrong.write_bytes(b'not the original app')
        def install(source,label,ref,expected=None):
            target=root/label;command=[str(binary),'install',str(source),'--destination',str(target)]
            if ref:command+=['--reference-app',str(ref)]
            result=subprocess.run(command,capture_output=True,text=True,timeout=30)
            if expected:
                assert result.returncode!=0 and expected in result.stderr,result.stderr
                assert not target.exists(),'Rejected package left a partial installation'
            else:assert result.returncode==0,result.stderr
            return target/ROOT
        install(package,'absent',None,'Elgato-protected')
        install(package,'wrong',wrong,'Unsupported original')
        installed=install(package,'valid',reference)
        with zipfile.ZipFile(package) as archive:original_manifest=archive.read(ROOT+'/manifest.json')
        assert (installed/'manifest.json').read_bytes()==original_manifest
        for mode in ['tampered','missing']:
            changed=root/(mode+'.streamDeckPlugin');variant(package,changed,REQUIRED,mode)
            install(changed,mode,reference,'integrity mismatch'if mode=='tampered'else'Required plugin file missing')
        changed=root/'optional.streamDeckPlugin';variant(package,changed,OPTIONAL,'tampered')
        optional=install(changed,'optional-valid',reference)
        assert (optional/'manifest.json').read_bytes()==original_manifest
        report.update(valid_inert_install=True,original_manifest_preserved=True,missing_required_rejected=True,tampered_required_rejected=True,optional_tamper_skipped_like_reference=True)
        state=root/'state';state.mkdir();(state/'session.json').write_text(json.dumps({'slots':[],'plugins':[],'reference_app':str(reference)}))
        host=Host(binary,[installed],state,root/'host.log')
        try:
            host.ready();plugin=host.state()['plugins'][0]
            assert plugin['id']=='com.elgato.discord' and plugin['status']=='failed',plugin
            assert plugin['pid']is None and 'Native Windows/macOS' in plugin['error'],plugin
            assert sum(a['uuid'].startswith('com.elgato.discord.')for a in host.state()['actions'])==17
            target=installed/'ESDDiscord.exe';original=target.read_bytes()
            def restart_failure(fragment):
                host.post({'command':'restart_plugin','id':'com.elgato.discord'},expected=400)
                current=host.state()['plugins'][0];assert current['pid']is None and fragment in current['error'],current
            target.write_bytes(original+b'\0');restart_failure('integrity mismatch')
            target.unlink();restart_failure('unavailable or unsafe')
            outside=root/'outside-required';outside.write_bytes(original);target.symlink_to(outside);restart_failure('unavailable or unsafe');target.unlink()
            import os
            os.mkfifo(target);restart_failure('not a regular file');target.unlink()
            target.write_bytes(original)
            host.post({'command':'restart_plugin','id':'com.elgato.discord'},expected=400)
            current=host.state()['plugins'][0];assert current['pid']is None and 'Native Windows/macOS' in current['error'],current
            assert (installed/'manifest.json').read_bytes()==original_manifest
            report.update(installed_files_rechecked_on_restart=True,required_symlink_rejected=True,required_fifo_rejected=True,native_backend_unexecuted=True,action_count=17)
        finally:host.stop();assert host.process.returncode==0
    assert hashlib.sha256(package.read_bytes()).hexdigest()==PACKAGE_HASH
    if args.report:args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: authenticated required-file integrity, inert install, negative archives, installed-file rechecks and optional-file semantics. Original backend remained unsupported and unexecuted.')
if __name__=='__main__':main()
