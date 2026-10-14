#!/usr/bin/env python3
"""Observe actual owned processes through the host's automatic application artwork."""
import argparse,base64,hashlib,io,json,os,shutil,subprocess,tempfile,time
from pathlib import Path
from unittest.mock import patch
from PIL import Image
from system_interop import Host
UUID='com.elgato.streamdeck.system.openapp'
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/application-artwork'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);children=[];host=None
 with tempfile.TemporaryDirectory(prefix='deckforge-app-art-') as d:
  root=Path(d);exe=root/'owned sleep';shutil.copy2('/usr/bin/sleep',exe)
  apps=root/'data/applications';apps.mkdir(parents=True);icon=root/'icon.svg';icon.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#0000ff"/></svg>')
  desktop=apps/'owned.desktop';desktop.write_text(f'[Desktop Entry]\nType=Application\nName=Owned artwork\nExec="{exe}" 30\nIcon={icon}\n')
  theme=root/'data/icons/hicolor';(theme/'64x64/apps').mkdir(parents=True);(theme/'index.theme').write_text('[Icon Theme]\nName=Hicolor\nDirectories=64x64/apps\n[64x64/apps]\nSize=64\nType=Fixed\nContext=Applications\n');(theme/'64x64/apps/deckforge-art-owned.svg').write_text(icon.read_text().replace('#0000ff','#ff0000'))
  themed=apps/'themed.desktop';themed.write_text(f'[Desktop Entry]\nType=Application\nName=Owned themed artwork\nExec="{exe}" 31\nIcon=deckforge-art-owned\n')
  env={'XDG_DATA_HOME':str(root/'data'),'XDG_CONFIG_HOME':str(root/'config')}
  def start(log):
   with patch.dict(os.environ,env):return Host(a.binary.resolve(),[],root/'state',a.output/log).ready()
  def slot(index=0):return next(s for s in host.state()['slots'] if s['controller']=='Keypad' and s['index']==index)
  def wait(index,expected):
   host.wait(lambda s:next(v for v in s['slots'] if v['index']==index)['application_running'] is expected,'application running '+str(expected))
  def picture(index=0):
   return Image.open(io.BytesIO(base64.b64decode(slot(index)['preview']))).convert('RGB')
  def assign(index,settings):
   host.post({'command':'assign','controller':'Keypad','index':index,'uuid':UUID});host.post({'command':'settings','context':slot(index)['context'],'settings':settings})
  try:
   host=start('host.log');config={'exec':str(exe),'source':str(desktop),'args':'','bring_to_front':True,'long_press':'quit','Future':{'keep':7}}
   assign(0,config);assign(1,{**config,'source':str(themed)})
   wait(0,False);wait(1,False);assert picture().getpixel((60,50))==(0,0,255);assert picture(1).getpixel((60,50))==(255,0,0)
   baseline=(root/'state/session.json').read_bytes();idle=slot()['preview']
   first=subprocess.Popen([str(exe),'30']);children.append(first);wait(0,True);wait(1,False)
   assert picture().getpixel((11,11))==(141,216,188);assert slot()['preview']!=idle
   second=subprocess.Popen([str(exe),'30']);children.append(second);first.terminate();first.wait(timeout=3);wait(0,True)
   second.terminate();second.wait(timeout=3);wait(0,False);assert slot()['preview']==idle
   assert (root/'state/session.json').read_bytes()==baseline,'Process observations changed persisted state'
   # A changed target must not reuse the old icon or running flag.
   wrong=subprocess.Popen([str(exe),'31']);children.append(wrong);wait(1,True);wait(0,False)
   host.post({'command':'settings','context':slot()['context'],'settings':{**config,'source':str(themed)}});wait(0,True);assert picture().getpixel((60,50))==(255,0,0)
   host.stop();host=start('restart.log');wait(0,True);assert picture().getpixel((60,50))==(255,0,0);assert slot()['settings']['Future']=={'keep':7}
   wrong.terminate();wrong.wait(timeout=3);wait(0,False)
   # The manual executable path uses its literal arguments to isolate interpreters.
   assign(0,{'exec':str(exe),'args':'32'});wait(0,False);manual=subprocess.Popen([str(exe),'32']);children.append(manual);wait(0,True);manual.terminate();manual.wait(timeout=3);wait(0,False)
   picture().save(a.output/'idle.png')
   (a.output/'proof.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'absolute_and_hicolor_icons':True,'external_launch_and_exit':True,'multiple_processes':True,'argument_scope':True,'stale_target_rejected':True,'restart':True,'runtime_observations_do_not_persist':True,'user_actions_executed':False,'usb_access':False},indent=2)+'\n')
  finally:
   if host:host.stop()
   for child in children:
    if child.poll() is None:child.kill()
    child.wait(timeout=3)
 print('PASS: automatic app icons, external process start/exit, multiple instances, argument isolation, changed targets and restart. No USB or user state.')
if __name__=='__main__':main()
