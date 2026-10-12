#!/usr/bin/env python3
"""Portable platform entries register through the unchanged official SDK; no USB."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

from system_interop import Host
from sample_interop import SAMPLE_COMMIT, COUNTER_LOCK_SHA256, require


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/debug/deckforge')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    samples = project.parent / 'evidence/official-samples'
    sdk = samples / 'counter/node_modules/@elgato/streamdeck/dist/plugin/index.js'
    require(subprocess.check_output(['git', '-C', str(samples), 'rev-parse', 'HEAD'], text=True).strip() == SAMPLE_COMMIT, 'Official dependency checkout changed')
    require(hashlib.sha256((samples / 'counter/package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256, 'Official dependency lock changed')
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'sample_commit': SAMPLE_COMMIT,
              'sdk_entry_sha256': hashlib.sha256(sdk.read_bytes()).hexdigest(),
              'boundary': 'Generated manifests/entry wrappers; unchanged official SDK. Simulation only; no USB.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-entry-') as folder:
        root = Path(folder)
        paths = []
        proof_paths = []
        for extension in ('mjs', 'cjs'):
            plugin = root / (extension + '.sdPlugin')
            plugin.mkdir(); (plugin / 'bin').mkdir()
            proof = plugin / 'proof.json'
            proof_paths.append(proof)
            identifier = 'com.deckforge.entry.' + extension
            manifest = {'UUID': identifier, 'Name': extension, 'Version': '1.0.0', 'Nodejs': {'Version': '20'},
                        'CodePathWin': 'bin/plugin.' + extension,
                        'Actions': [{'UUID': identifier + '.action', 'Name': extension, 'Controllers': ['Keypad'], 'States': [{}]}]}
            if extension == 'cjs':
                manifest['CodePathMac'] = manifest['CodePathWin']
            (plugin / 'manifest.json').write_text(json.dumps(manifest))
            # .cjs must remain CommonJS even inside an ES-module package.
            (plugin / 'package.json').write_text('{"type":"module"}')
            body = (f"const {{default:sd}}=await import({json.dumps(sdk.as_uri())});"
                    "await sd.connect();const settings=await sd.settings.getGlobalSettings();"
                    f"fs.writeFileSync({json.dumps(str(proof))},JSON.stringify({{pid:process.pid,cwd:process.cwd(),execArgv:process.execArgv,settings}}));")
            source = ("const fs=require('node:fs');(async()=>{" + body + "})().catch(error=>{console.error(error);process.exit(1)});"
                      if extension == 'cjs' else "import fs from 'node:fs';" + body)
            (plugin / ('bin/plugin.' + extension)).write_text(source)
            paths.append(plugin)
        ambiguous = root / 'ambiguous.sdPlugin'; ambiguous.mkdir()
        (ambiguous / 'manifest.json').write_text(json.dumps({'UUID': 'com.deckforge.entry.ambiguous', 'Name': 'Ambiguous',
            'Version':'1.0.0','Nodejs':{'Version':'20'},'CodePathWin':'a.js','CodePathMac':'b.js','Actions':[]}))
        (ambiguous / 'a.js').write_text('throw Error("must not run")'); (ambiguous / 'b.js').write_text('throw Error("must not run")')
        paths.append(ambiguous)
        binary_plugin = root / 'native.sdPlugin'; binary_plugin.mkdir()
        (binary_plugin / 'manifest.json').write_text(json.dumps({'UUID':'com.deckforge.entry.native','Name':'Native','Version':'1.0.0',
            'CodePath':'plugin.exe','CodePathWin':'portable.js','Nodejs':{'Version':'20'},'Actions':[]}))
        (binary_plugin / 'plugin.exe').write_bytes(b'MZ'+b'\0'*64)
        (binary_plugin / 'portable.js').write_text('throw Error("generic binary must not fall back")')
        paths.append(binary_plugin)
        host = Host(binary, paths, root / 'state', root / 'host.log')
        try:
            host.ready()
            host.wait(lambda state: all(next(p for p in state['plugins'] if p['id']=='com.deckforge.entry.'+extension)['connected'] for extension in ('mjs','cjs')), 'Win-only MJS and shared-platform CJS SDK registration')
            host.wait(lambda _: all(path.exists() for path in proof_paths), 'Official SDK global-settings acknowledgements')
            plugins = host.state()['plugins']
            proofs = [json.loads(path.read_text()) for path in proof_paths]
            for extension, proof, path in zip(('mjs','cjs'), proofs, paths[:2]):
                state = next(p for p in plugins if p['id']=='com.deckforge.entry.'+extension)
                require(state['entry_path']==str(path / ('bin/plugin.'+extension)), 'Resolved entry metadata incorrect')
                require(state['entry_field']=='CodePathWin', 'Unexpected platform selection')
                require(Path(state['runtime_path']).is_absolute(), 'Runtime executable was not resolved')
                require(proof['execArgv']==['--enable-source-maps','--no-global-search-paths'], 'Documented Node launch flags missing')
                require(proof['cwd']==str(path), 'Plugin working directory changed')
                require(proof['settings']=={}, 'SDK request did not reach host')
            for identifier, message in [('ambiguous','Ambiguous'),('native','Native Windows/macOS')]:
                state = next(p for p in plugins if p['id']=='com.deckforge.entry.'+identifier)
                require(state['status']=='failed' and state['pid'] is None, 'Unsafe/ambiguous entry executed')
                require(message in state['error'], state['error'])
            report['plugins'] = plugins; report['proofs'] = proofs
        except Exception:
            print((root / 'host.log').read_text())
            raise
        finally:
            host.stop()
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: unchanged official SDK registered from CodePathWin-only MJS and shared Win/Mac CJS; flags/cwd/runtime metadata correct; ambiguous and generic native entries remained failed without execution. Simulation only.')


if __name__ == '__main__':
    main()
