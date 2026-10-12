#!/usr/bin/env python3
"""Plugin launch/crash/restart isolation using owned Node children, without USB."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from system_interop import Host
from sample_interop import require, SAMPLE_COMMIT, COUNTER_LOCK_SHA256
import hashlib

JS = r'''import {createRequire} from 'node:module';import{appendFileSync}from'node:fs';
const WebSocket=createRequire(__ROOT__)('ws'),proof=__PROOF__,args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const record=(kind,extra={})=>appendFileSync(proof,JSON.stringify({kind,pid:process.pid,...extra})+'\n');
const ws=new WebSocket('ws://127.0.0.1:'+args['-port']);
ws.on('open',()=>{ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']}));record('registered',{launch:args['-pluginUUID']});});
ws.on('message',data=>{const m=JSON.parse(data);record(m.event,{context:m.context,payload:m.payload});});
setInterval(()=>{if(ws.readyState===1)ws.send(JSON.stringify({event:'getGlobalSettings',context:args['-pluginUUID'],id:String(Date.now())}));},200);
'''


def rows(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines(keepends=True) if line.endswith('\n')]


def plugin_state(host, identifier):
    return next(p for p in host.state()['plugins'] if p['id'] == identifier)


def wait(predicate, description):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.04)
    raise AssertionError(description)


def fixture(root, name, source, node='20'):
    path = root / (name + '.sdPlugin')
    path.mkdir()
    identifier = 'com.deckforge.runtime.' + name
    manifest = {'UUID': identifier, 'Name': name, 'Version': '1.2.3', 'CodePath': 'plugin.mjs',
                'Nodejs': {'Version': node}, 'Actions': [{'UUID': identifier + '.action', 'Name': name,
                                                       'Controllers': ['Keypad'], 'States': [{}]}]}
    (path / 'manifest.json').write_text(json.dumps(manifest))
    (path / 'plugin.mjs').write_text(source)
    return path, identifier, manifest


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/debug/deckforge')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    samples = project.parent / 'evidence/official-samples'
    package = samples / 'counter/package.json'
    require(subprocess.check_output(['git', '-C', str(samples), 'rev-parse', 'HEAD'], text=True).strip() == SAMPLE_COMMIT, 'Official dependency checkout changed')
    require(hashlib.sha256((package.parent / 'package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256, 'Official dependency lock changed')
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'boundary': 'Simulation only; generated plugins use pinned official sample generic ws dependency', 'sample_commit': SAMPLE_COMMIT}
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-runtime-') as folder:
        root = Path(folder)
        proof = root / 'proof.jsonl'
        source = JS.replace('__ROOT__', json.dumps(str(package))).replace('__PROOF__', json.dumps(str(proof)))
        good, good_id, _ = fixture(root, 'healthy', source)
        bad, bad_id, bad_manifest = fixture(root, 'runtime', source, '999')
        crash, crash_id, _ = fixture(root, 'crash', "console.error('fixture-crash-detail');process.exit(23);")
        missing = root / 'missing.sdPlugin'
        malformed = root / 'malformed.sdPlugin'
        malformed.mkdir(); (malformed / 'manifest.json').write_text('{')
        host = Host(binary, [good, bad, crash, missing, malformed], root / 'state', root / 'host.log')
        try:
            host.ready()
            host.wait(lambda state: next(p for p in state['plugins'] if p['id']==good_id)['connected'], 'Healthy plugin registers despite failures')
            host.wait(lambda state: next(p for p in state['plugins'] if p['id']==crash_id)['status']=='failed', 'Crash is reaped')
            initial = host.state()['plugins']
            require(len(initial) == 5, 'Unresolved paths disappeared')
            require(plugin_state(host, bad_id)['status']=='failed', 'Missing runtime not visible')
            require('Node.js 999' in plugin_state(host, bad_id)['error'], 'Missing runtime error not actionable')
            require(plugin_state(host, bad_id)['version']=='1.2.3', 'Version missing')
            require(all(p['path'] for p in initial), 'Plugin paths missing')
            healthy_pid = plugin_state(host, good_id)['pid']
            installed, installed_id, _ = fixture(root, 'installed-unavailable', source, '999')
            host.post({'command': 'install_plugin', 'path': str(installed)})
            require(plugin_state(host, installed_id)['status'] == 'failed', 'Installed runtime failure disappeared')
            require('Node.js 999' in plugin_state(host, installed_id)['error'], 'Installed failure lacks runtime guidance')
            require(plugin_state(host, good_id)['pid'] == healthy_pid, 'Failed install restarted healthy plugin')
            require(plugin_state(host, crash_id)['pid'] is None, 'Exited child not reaped')
            require('fixture-crash-detail' in '\n'.join(host.state()['logs']), 'Crash log tail missing')
            saved = json.loads((root / 'state/session.json').read_text())
            require(str(missing) in saved['plugins'] and str(malformed) in saved['plugins'], 'Unresolved paths not persisted')
            host.post({'command':'assign','controller':'Keypad','index':1,'uuid':bad_id+'.action'})
            context = next(s['context'] for s in host.state()['slots'] if s['index']==1 and s['controller']=='Keypad')
            host.post({'command':'settings','context':context,'settings':{'kept':73}})
            # Repair the runtime and restart explicitly. Other plugins remain untouched.
            bad_manifest['Nodejs']['Version']='20'
            (bad / 'manifest.json').write_text(json.dumps(bad_manifest))
            host.post({'command':'restart_plugin','id':bad_id})
            host.wait(lambda state: next(p for p in state['plugins'] if p['id']==bad_id)['connected'], 'Explicit restart recovers missing runtime')
            wait(lambda:any(r['kind']=='willAppear' and r.get('context')==context and r.get('payload',{}).get('settings')=={'kept':73} for r in rows(proof)), 'Restart lost saved settings')
            require(plugin_state(host, good_id)['pid']==healthy_pid, 'Unrelated healthy plugin restarted')
            repaired_pid = plugin_state(host, bad_id)['pid']
            # Live restart rotates both child and registration. Old registration cannot mutate globals.
            wait(lambda:any(r['kind']=='registered' and r['pid']==healthy_pid for r in rows(proof)), 'Registration proof missing')
            old_launch = next(r['launch'] for r in rows(proof) if r['kind']=='registered' and r['pid']==healthy_pid)
            host.post({'command':'restart_plugin','id':good_id})
            host.wait(lambda state: next(p for p in state['plugins'] if p['id']==good_id)['connected'], 'Live restart reconnects')
            new_pid = plugin_state(host, good_id)['pid']
            require(new_pid != healthy_pid, 'Restart reused old child')
            require(not Path(f'/proc/{healthy_pid}').exists(), 'Old child was not reaped')
            require(plugin_state(host, bad_id)['pid']==repaired_pid, 'Restart affected another plugin')
            stale_js = r'''const WebSocket=require(process.argv[1]);const ws=new WebSocket(process.argv[2]);
let closed=false;ws.on('open',()=>{ws.send(JSON.stringify({event:'registerPlugin',uuid:process.argv[3]}));ws.send(JSON.stringify({event:'setGlobalSettings',payload:{stalePoison:true}}));});
ws.on('close',()=>{closed=true;process.exit(0)});ws.on('error',()=>{});setTimeout(()=>process.exit(closed?0:2),1500);'''
            ws_path = subprocess.check_output(['node','-e','console.log(require.resolve("ws",{paths:[process.argv[1]]}))',str(package.parent)], text=True).strip()
            subprocess.run(['node','-e',stale_js,ws_path,host.base.replace('http:','ws:'),old_launch], check=True, timeout=4)
            time.sleep(.25)
            require(plugin_state(host, good_id)['connected'] and plugin_state(host, good_id)['pid']==new_pid, 'Stale socket disconnected replacement')
            saved = json.loads((root / 'state/session.json').read_text())
            require(not saved.get('globals',{}).get(good_id,{}).get('stalePoison'), 'Stale registration mutated settings')
            # Restart an early-crashing child after repairing only its entry code.
            (crash / 'plugin.mjs').write_text(source)
            host.post({'command':'restart_plugin','id':crash_id})
            host.wait(lambda state: next(p for p in state['plugins'] if p['id']==crash_id)['connected'], 'Crashed plugin did not recover')
            # Repair a missing folder; placeholder identity is replaced only after valid reload.
            missing_id = next(p['id'] for p in initial if p['path']==str(missing))
            recovered, recovered_id, _ = fixture(root, 'missing', source)
            require(recovered == missing, 'Fixture path mismatch')
            host.post({'command':'restart_plugin','id':missing_id})
            host.wait(lambda state: any(p['id']==recovered_id and p['connected'] for p in state['plugins']), 'Missing directory recovery failed')
            current = host.state()['plugins']
            report['initial'] = initial
            report['recovered'] = current
            report['settings_preserved'] = next(s['settings'] for s in host.state()['slots'] if s['context']==context)
            pids = [p['pid'] for p in current if p['pid']]
            host.stop()
            wait(lambda:all(not Path(f'/proc/{pid}').exists() for pid in pids), 'Shutdown left plugin children')
            report['children_reaped'] = True
        except Exception:
            print((root / 'host.log').read_text())
            raise
        finally:
            host.stop()
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: startup/install failure isolation, visible runtime/crash/missing/malformed errors, explicit repair/restart, stale WebSocket rejection, independent plugin PIDs, preserved settings/path references, owned child cleanup. Simulation only.')


if __name__ == '__main__':
    main()
