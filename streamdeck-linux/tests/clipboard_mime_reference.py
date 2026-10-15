#!/usr/bin/env python3
"""Full-MIME transport reference and snapshot race probe in an owned Wayland session."""
import argparse,hashlib,json,os,signal,subprocess,sys,tempfile,time
from pathlib import Path

def inside(root,test_binary):
 assert root.is_absolute() and root.name.startswith('deckforge-mime-') and os.environ['XDG_RUNTIME_DIR']==str(root/'runtime')
 with (root/'kwin.log').open('w') as log:
  kwin=subprocess.Popen(['kwin_wayland','--virtual','--no-lockscreen','--no-global-shortcuts','--socket','deckforge-private-mime'],env={**os.environ,'QT_QPA_PLATFORM':'offscreen'},stdout=log,stderr=log)
  try:
   deadline=time.monotonic()+8
   while not (root/'runtime/deckforge-private-mime').exists():
    assert kwin.poll() is None and time.monotonic()<deadline,'Private compositor did not start';time.sleep(.025)
   result=subprocess.run([str(test_binary),'private_wayland_multi_mime_reference','--ignored','--nocapture'],env={**os.environ,'DECKFORGE_PRIVATE_MIME_TEST':str(root),'DECKFORGE_PRIVATE_KWIN_PID':str(kwin.pid)},capture_output=True,text=True,timeout=20)
   (root/'worker.log').write_text(result.stdout+'\n'+result.stderr);assert result.returncode==0,result.stdout+'\n'+result.stderr
  finally:
   kwin.terminate();kwin.wait(timeout=5)
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--test-binary',type=Path,required=True);p.add_argument('--output',type=Path,default=Path('target/clipboard-mime-reference'));p.add_argument('--inside',type=Path);a=p.parse_args();test_binary=a.test_binary.resolve()
 if a.inside:inside(a.inside,test_binary);return
 a.output.mkdir(parents=True,exist_ok=True)
 (a.output/'proof.json').unlink(missing_ok=True)
 with tempfile.TemporaryDirectory(prefix='deckforge-mime-') as d:
  root=Path(d);(root/'runtime').mkdir(mode=0o700);(root/'tmp').mkdir(mode=0o700);env=os.environ.copy()
  for key in ['DISPLAY','WAYLAND_SOCKET']:env.pop(key,None)
  env.update(XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),TMPDIR=str(root/'tmp'),WAYLAND_DISPLAY='deckforge-private-mime',LIBGL_ALWAYS_SOFTWARE='1')
  with (a.output/'run.log').open('w') as log:
   proc=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),'--inside',d,'--test-binary',str(test_binary)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
   try:code=proc.wait(timeout=35)
   finally:
    try:
     os.killpg(proc.pid,signal.SIGCONT)
     os.killpg(proc.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    proc.wait(timeout=5)
  for name in ['kwin.log','worker.log','source.log']:
   if (root/name).exists():(a.output/name).write_text((root/name).read_text())
  assert code==0,(a.output/'run.log').read_text()[-5000:];proof=json.loads((root/'proof.json').read_text())
 proof['test_binary_sha256']=hashlib.sha256(test_binary.read_bytes()).hexdigest();proof['wl_clipboard_rs']='0.9.3';proof['source_sha256']={name:hashlib.sha256(Path(name).read_bytes()).hexdigest() for name in ['src/system_clipboard.rs','src/system_clipboard/mime_wayland.rs','src/system_clipboard/mime_reference.rs']};(a.output/'proof.json').write_text(json.dumps(proof,indent=2)+'\n')
 print('PASS: five MIME formats survive text takeover and restoration; independent clipboard reads demonstrably mix generations. Private protocol reference only; production restore remains disabled.')
if __name__=='__main__':main()
