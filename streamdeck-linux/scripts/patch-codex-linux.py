#!/usr/bin/env python3
"""Adapt an authenticated installed Codex plugin for Linux; preserve the original."""
import argparse,hashlib,json,subprocess,tempfile,shutil
from datetime import datetime,timezone
from pathlib import Path

def authenticated_manifest(plugin, reference, work):
    root = Path(__file__).resolve().parents[1]
    source = work / 'verify.rs'
    source.write_text((root / 'plugins/spotify-linux/verify-manifest.rs').read_text().replace(
        'PROTECTED_MANIFEST_SOURCE', str(root / 'src/protected_manifest.rs')))
    deps = root / 'target/release/deps'
    command = ['rustc', '--edition=2024', str(source), '-L', 'dependency=' + str(deps), '-o', str(work / 'verify')]
    for name in ['aes_gcm', 'anyhow', 'base64', 'blake3', 'ed25519_dalek', 'hkdf', 'minicbor', 'serde_json', 'sha2', 'x25519_dalek', 'zeroize', 'libc']:
        libraries = sorted(deps.glob('lib' + name + '-*.rlib'), key=lambda p: p.stat().st_mtime, reverse=True)
        if not libraries:
            raise SystemExit('Build Deckforge release dependencies first: missing ' + name)
        command.extend(['--extern', name + '=' + str(libraries[0])])
    subprocess.run(command, check=True, capture_output=True)
    output = work / 'authenticated.json'
    subprocess.run([str(work / 'verify'), str(plugin), str(reference), str(output)], check=True)
    return json.loads(output.read_text())


ENDPOINT='if("darwin"===e){const e=n();return{platform:"macos",path:null!=e&&w.isAbsolute(t)?w.join(t,"ipc","ipc.sock"):null,available:null!=e&&w.isAbsolute(t),appIdentity:"unified-chatgpt"}}'
LINUX_ENDPOINT='if("darwin"===e||"linux"===e){const i=n();return{platform:e==="linux"?"linux":"macos",path:null!=i&&w.isAbsolute(t)?w.join(t,"ipc","ipc.sock"):null,available:null!=i&&w.isAbsolute(t),appIdentity:"unified-chatgpt"}}'
PATCHES=[
 (ENDPOINT,LINUX_ENDPOINT,1),
 ('"macos"===this.endpoint.platform','["macos","linux"].includes(this.endpoint.platform)',2),
 ('async function jy(e={}){','async function jy(e={}){if((e.platform??process.platform)==="linux"){if(nv(e.env??process.env))return null;return (await import("./linux-discovery.mjs")).discover(e.env??process.env);}',1),
 ('"darwin"===t?"/usr/bin/open":null','"darwin"===t?"/usr/bin/open":"linux"===t?"/usr/bin/xdg-open":null',1),
]
def patched(source):
 for old,new,count in PATCHES:
  if source.count(old)!=count:raise ValueError('Unrecognized Codex plugin build; no files changed: '+old[:60])
  source=source.replace(old,new)
 return source

def install(plugin,reference,target=None):
 source=(plugin/'bin/plugin.js').read_text()
 shim=Path(__file__).resolve().parents[1]/'plugins/codex-linux/linux-discovery.mjs'
 if LINUX_ENDPOINT in source:
  if (plugin/'bin/linux-discovery.mjs').read_bytes()==shim.read_bytes():return {'status':'already installed','target':str(plugin)}
  raise ValueError('Installed adaptation differs; restore the retained original first')
 output=patched(source)
 with tempfile.TemporaryDirectory(prefix='codex-plugin-verify-') as work:manifest=authenticated_manifest(plugin,reference,Path(work))
 if manifest.get('UUID')!='com.elgato.openai-codex':raise ValueError('Unexpected plugin identity')
 target=target or plugin
 stage=Path(tempfile.mkdtemp(prefix='.codex-linux-stage-',dir=target.parent))
 backup=None
 try:
  shutil.copytree(plugin,stage,dirs_exist_ok=True)
  (stage/'bin/plugin.js').write_text(output);shutil.copy2(shim,stage/'bin/linux-discovery.mjs')
  manifest.pop('Files',None);manifest.pop('Marketplace',None)
  manifest['Name']='ChatGPT & Codex for Linux';manifest['Description']='Local Linux adaptation of the installed Elgato ChatGPT & Codex plugin.'
  manifest['OS']=[{'Platform':'linux','MinimumVersion':'0'}]
  (stage/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
  if target.exists():
   if target!=plugin:raise ValueError('Separate staging destination already exists')
   backup=target.parent.parent/'plugin-backups'/('codex-'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ'));backup.parent.mkdir(parents=True,exist_ok=True);target.rename(backup)
  try:stage.rename(target)
  except BaseException:
   if backup:backup.rename(target)
   raise
 finally:
  if stage.exists():shutil.rmtree(stage)
 return {'status':'installed','target':str(target),'backup':str(backup) if backup else None,'original_sha256':hashlib.sha256(source.encode()).hexdigest()}
if __name__=='__main__':
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('plugin',type=Path);p.add_argument('--reference-app',type=Path,required=True);p.add_argument('--target',type=Path);a=p.parse_args()
 print(json.dumps(install(a.plugin.resolve(),a.reference_app.resolve(),a.target.resolve() if a.target else None),indent=2))
