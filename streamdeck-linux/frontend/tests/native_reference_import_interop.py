#!/usr/bin/env python3
"""Actual native importer API and configured UI with an isolated private cache.

The native picker is not intercepted or driven here; its UI command pipeline is
covered by ReferenceInstaller.test.tsx. Native import, validation and errors run
normally. No vendor executable, account login, USB or user configuration is used.
"""
import argparse
import hashlib
import json
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

PROJECT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT / "tests"))
from native_editor_interop import cleanup, environment, result

SCRIPT = r'''
const expected=__EXPECTED__, bridge=window.deckforge;
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(fn,name)=>{for(let n=0;n<800;n++){const value=await fn();if(value)return value;await pause(40)}throw Error('Timed out: '+name)};
const snapshot=()=>bridge.request('snapshot');
if(!(await snapshot()).simulate)throw Error('Simulation required');
await bridge.request('command',{command:'reference_app',path:expected.reference});
await until(async()=>(await snapshot()).reference_app===expected.reference,'valid prior reference');
[...document.querySelectorAll('.header-tools button')].find(button=>button.textContent.trim().startsWith('Plugins')).click();
await until(()=>document.querySelector('.protected-reference'),'native importer section');
if(!document.querySelector('.protected-reference').textContent.includes('7.6.0.23012 macOS'))throw Error('Supported installer boundary is not visible');
let invalid_error='';try{await bridge.request('reference_import',{path:expected.invalid,expected:expected.reference})}catch(error){invalid_error=String(error)}
if(!invalid_error||(await snapshot()).reference_app!==expected.reference)throw Error('Invalid import did not preserve prior reference');
let stale_error='';try{await bridge.request('reference_import',{path:expected.invalid,expected:null})}catch(error){stale_error=String(error)}
if(!stale_error.includes('Reference changed')||(await snapshot()).reference_app!==expected.reference)throw Error('Stale importer did not preserve current reference');
let settled=false;
const importing=bridge.request('reference_import',{path:expected.installer,expected:expected.reference}).finally(()=>{settled=true});
await pause(30);
const probeStarted=performance.now();const during=await snapshot();const snapshot_ms=performance.now()-probeStarted;
const retained=await bridge.request('ui_state_get');const saveStarted=performance.now();await bridge.request('ui_state_set',{state:retained});const ui_state_set_ms=performance.now()-saveStarted;
const pending_during_probes=!settled;
if(!pending_during_probes||during.reference_app!==expected.reference)throw Error('Import completed before responsiveness probes could run');
if(snapshot_ms>1500||ui_state_set_ms>1500)throw Error('Native editor requests blocked during extraction');
let concurrent_error='';try{await bridge.request('reference_import',{path:expected.invalid,expected:expected.reference})}catch(error){concurrent_error=String(error)}
if(!/already|progress|busy|running/i.test(concurrent_error))throw Error('Concurrent importer was not rejected by busy guard: '+concurrent_error);
const imported=await importing;
if(imported.version!=='7.6.0.23012'||!imported.path||!/mac/i.test(imported.platform))throw Error('Native import metadata missing');
await until(async()=>(await snapshot()).reference_app===imported.path,'real installer extraction and registration');
await until(()=>document.querySelector('.protected-reference [role=status]')?.textContent.includes('Configured'),'configured UI reflects native import');
await until(()=>document.querySelector('.protected-reference .reference-path')?.title===imported.path,'friendly reference path reflects imported cache');
document.querySelector('.protected-reference').scrollIntoView({block:'center'});
return{invalid_error,invalid_preserves_prior:true,stale_error,stale_preserves_prior:true,actual_native_import_api:true,picker_button_exercised:false,picker_button_coverage:'separate frontend unit tests',pending_during_probes,snapshot_ms,ui_state_set_ms,concurrent_error,import_metadata:imported,configured_ui_visible:true,reference:imported.path,simulate:true,vendor_code_executed:false,usb_access:false};
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=PROJECT / "target/debug/deckforge")
    parser.add_argument("--installer", type=Path, required=True)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, installer, reference, output = args.binary.resolve(strict=True), args.installer.resolve(strict=True), args.reference.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    report = {"ok": False,"binary_sha256":digest(binary),"installer_sha256":digest(installer),"reference_sha256":digest(reference)}
    with tempfile.TemporaryDirectory(prefix="deckforge-native-reference-") as temporary:
        root = Path(temporary)
        invalid = root / "invalid.pkg"
        invalid.write_bytes(b"inert fixture; not an installer")
        script, proof = root / "fixture.js", root / "proof.json"
        script.write_text(SCRIPT.replace("__EXPECTED__",json.dumps({"installer":str(installer),"reference":str(reference),"invalid":str(invalid)})))
        env = environment(root)
        env.update(DECKFORGE_CAPTURE=str(output / "reference.png"),DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof))
        log_path = output / "reference.log"
        with log_path.open("w") as log:
            process = subprocess.Popen([str(binary),"desktop","--simulate","--state",str(root / "state")],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=70)==0,log_path.read_text()
                value=result(proof)
                assert value,log_path.read_text()
                cached=Path(value["result"]["reference"])
                assert cached.is_relative_to(root / "state/references")
                assert stat.S_IMODE(cached.stat().st_mode)==0o600
                assert stat.S_IMODE(cached.parent.stat().st_mode)==0o700
                assert digest(cached)==digest(reference)
                persisted=json.loads((root / "state/session.json").read_text())
                assert persisted["reference_app"]==str(cached)
                report.update(ok=True,native=value["result"],cached_sha256=digest(cached),cache_mode="0600",directory_mode="0700",persisted=True)
            except Exception as error:
                report["error"]=str(error)
                raise
            finally:
                cleanup(process)
                (output / "reference.json").write_text(json.dumps(report,indent=2)+"\n")
    print("PASS: actual native installer import, cached reference/hash/permissions, invalid/stale preservation, responsiveness, concurrent-job rejection and Configured UI; picker covered separately; no vendor execution or user state.")


if __name__ == "__main__":
    main()
