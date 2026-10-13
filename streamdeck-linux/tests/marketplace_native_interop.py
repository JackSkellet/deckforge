#!/usr/bin/env python3
"""Check the real native Preferences/Marketplace bridge against Elgato while signed out.

Uses a temporary simulated state and a fresh browser profile. No credentials,
purchase, plugin download, user state or physical Stream Deck are used.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from native_editor_interop import environment

SCRIPT = r"""
const until=async fn=>{for(let n=0;n<200;n++){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,50));}throw Error('Marketplace native check timed out');};
const button=(root,text)=>[...root.querySelectorAll('button')].find(b=>b.textContent.trim()===text);
button(document,'Preferences').click();
const dialog=await until(()=>document.querySelector('dialog[open]'));
button(dialog,'Marketplace').click();
await until(()=>dialog.querySelector('[aria-label="Elgato Marketplace"]'));
const state=await until(async()=>{const s=await window.deckforge.request('snapshot');return s.marketplace.authenticated===false&&!s.marketplace.busy&&s;});
if(state.marketplace.error)throw Error(state.marketplace.error);
if(state.marketplace.account!==null)throw Error('Fresh profile unexpectedly signed in');
if(!button(dialog,'Sign in to Marketplace'))throw Error('Missing native sign-in control');
if(!dialog.textContent.includes('Not signed in'))throw Error('Native session result did not render');
return {signedOut:true,signInControl:true,preferencesVisible:true,sessionError:null};
"""


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--capture', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    report, capture = args.report.resolve(), args.capture.resolve()
    report.parent.mkdir(parents=True, exist_ok=True)
    capture.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-marketplace-ui-') as tmp:
        root = Path(tmp)
        script, result = root/'probe.js', root/'result.json'
        script.write_text(SCRIPT)
        env = environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script), DECKFORGE_WEBVIEW_TEST_RESULT=str(result), DECKFORGE_CAPTURE=str(capture))
        run = subprocess.run([str(binary), 'desktop', '--simulate', '--state', str(root/'state')], env=env, capture_output=True, text=True, timeout=45)
        assert run.returncode == 0, run.stderr[-2000:]
        proof = json.loads(result.read_text())
        assert proof['ok'], proof
        assert capture.is_file()
        report.write_text(json.dumps({'ok': True, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'boundary': 'Native WebKit, real anonymous Elgato session endpoint, isolated simulated state; authenticated download requires user acceptance', 'result': proof['result']}, indent=2)+'\n')
        print(report)


if __name__ == '__main__':
    main()
