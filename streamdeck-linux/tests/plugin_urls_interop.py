#!/usr/bin/env python3
"""SDK and native PI openUrl dispatch to a test-owned opener; no external browser/network."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from native_editor_interop import environment
from system_interop import Host
from sample_interop import COUNTER_LOCK_SHA256, require

PLUGIN = 'com.deckforge.url-probe'
ACTION = PLUGIN + '.key'
NODE_URL = 'https://example.invalid/NodeSetup?code=PRIVATE_NODE&x=%252F&x=a+b#Keep'
PI_URL = 'https://example.invalid/InspectorSetup?code=PRIVATE_PI'

class TestHost(Host):
    def __init__(self, binary, plugin, state, log, env):
        self.log, self.output = log, log.open('w')
        self.process = subprocess.Popen([str(binary), 'serve', '--simulate', '--state', str(state), '--port', '0', '--plugin', str(plugin)],
                                        env=env, stdout=self.output, stderr=subprocess.STDOUT, start_new_session=True)
        self.base = self.token = None

def records(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line] if path.exists() else []

def wait(host, predicate, message):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        host.check_process()
        if result := predicate(): return result
        time.sleep(.025)
    raise AssertionError(message)

def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    sample = project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256, 'SDK fixture identity changed')
    sdk = sample/'node_modules/@elgato/streamdeck/dist/plugin/index.js'
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-urls-') as directory:
        root = Path(directory)
        plugin = root/(PLUGIN+'.sdPlugin'); plugin.mkdir()
        fake_bin = root/'bin'; fake_bin.mkdir()
        opened, proof = root/'opened.jsonl', root/'proof.jsonl'
        opener = fake_bin/'xdg-open'
        opener.write_text('#!/usr/bin/python3\nimport json,sys\nfrom pathlib import Path\nwith Path('+repr(str(opened))+').open("a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\n')
        opener.chmod(0o700)
        source = '''import streamDeck from __SDK__;
import {appendFileSync} from 'node:fs';
streamDeck.ui.onSendToPlugin(ev=>appendFileSync(__PROOF__,JSON.stringify(ev.payload)+'\\n'));
await streamDeck.connect();
await streamDeck.system.openUrl(__URL__);
await streamDeck.system.openUrl('javascript:PRIVATE_BAD');
'''.replace('__SDK__', json.dumps(sdk.as_uri())).replace('__PROOF__', json.dumps(str(proof))).replace('__URL__', json.dumps(NODE_URL))
        (plugin/'plugin.mjs').write_text(source)
        inspector = '''<!doctype html><meta charset="utf-8"><title>Owned URL inspector test</title><script>
window.connectElgatoStreamDeckSocket=(port,uuid,event,info,actionInfo)=>{
const ws=new WebSocket(`ws://127.0.0.1:${port}`);
ws.onopen=()=>{ws.send(JSON.stringify({event,uuid}));
ws.send(JSON.stringify({event:'openUrl',payload:{url:__URL__}}));
ws.send(JSON.stringify({event:'openUrl',payload:{url:'file:///PRIVATE_BAD'}}));
ws.send(JSON.stringify({event:'sendToPlugin',payload:{piComplete:true}}));};};
</script>'''.replace('__URL__', json.dumps(PI_URL))
        (plugin/'pi.html').write_text(inspector)
        (plugin/'manifest.json').write_text(json.dumps({'UUID':PLUGIN,'Name':'URL probe','Version':'1.0.0','SDKVersion':2,'Software':{'MinimumVersion':'7.1'},'CodePath':'plugin.mjs','Nodejs':{'Version':'20'},'Actions':[{'UUID':ACTION,'Name':'URL probe','Controllers':['Keypad'],'States':[{}],'PropertyInspectorPath':'pi.html'}]}))
        env = environment(root); env['PATH'] = str(fake_bin) + os.pathsep + env.get('PATH','')
        host = TestHost(binary, plugin, root/'state', root/'host.log', env)
        try:
            host.ready(); host.wait(lambda s: s['plugins'][0]['connected'], 'SDK plugin registration')
            wait(host, lambda: records(opened), 'Contextless SDK openUrl did not invoke the opener')
            host.post({'command':'assign','controller':'Keypad','index':0,'uuid':ACTION})
            context = host.state()['slots'][0]['context']
            host.post({'command':'open_inspector','context':context})
            wait(host, lambda: any(r.get('piComplete') for r in records(proof)), 'Native inspector did not finish its requests')
            wait(host, lambda: len(records(opened)) == 2, 'PI openUrl did not invoke the opener')
            require(records(opened) == [[NODE_URL], [PI_URL]], 'Wrong opener arguments or a forbidden scheme was opened')
            host.post({'command':'close_inspector','context':context})
            require(all(secret not in json.dumps(host.state()) + host.log.read_text() for secret in ('PRIVATE_NODE','PRIVATE_PI','PRIVATE_BAD')), 'URL credential leaked to host logs/snapshot')
            report = {'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sdk_lock_sha256':COUNTER_LOCK_SHA256,
                      'boundary':'Temporary simulated host and native PI; fake xdg-open records arguments only; no external network/browser/USB.',
                      'contextless_plugin_open_url':True,'contextless_native_pi_open_url':True,'encoded_query_preserved':True,
                      'unsupported_schemes_rejected':True,'no_credentials_in_host_logs_or_snapshot':True}
        finally:
            host.stop()
            require(host.process.returncode == 0, 'Host shutdown failed')
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: unchanged SDK openUrl and native PI route to the owned fake opener; unsafe schemes rejected, URL data kept out of host logs.')

if __name__ == '__main__': main()
