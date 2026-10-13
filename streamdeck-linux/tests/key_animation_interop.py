#!/usr/bin/env python3
"""Animated key artwork and Deckforge's SDK GIF extension, using unchanged SDK calls. No USB."""
import argparse,base64,hashlib,json,tempfile,time,uuid,zipfile
from pathlib import Path
from PIL import Image
from display_targets_interop import make_plugin,wait,rows,action,decode,KEY,DIAL
from sample_interop import require,COUNTER_LOCK_SHA256
from system_interop import Host

RED=(255,0,0);BLUE=(0,0,255);GREEN=(0,255,0)
def gif(path,loop=0):
 a=Image.new('RGB',(32,32),RED);b=Image.new('RGB',(32,32),BLUE)
 a.save(path,save_all=True,append_images=[b],duration=[80,120],loop=loop)
def colors(host,seconds=.65,controller="Keypad"):
 found=set();end=time.monotonic()+seconds
 while time.monotonic()<end:
  found.add(decode(action(host,controller)['software_icon' if controller=='Encoder' else 'preview']).getpixel((0,0)));time.sleep(.025)
 return found

def main():
 project=Path(__file__).resolve().parents[1];ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge');ap.add_argument('--report',type=Path,required=True);args=ap.parse_args();binary=args.binary.resolve(strict=True)
 sample=project.parent/'evidence/official-samples/counter';require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'SDK fixture changed')
 sdk=sample/'node_modules/@elgato/streamdeck';tracked=['dist/plugin/actions/key.js','dist/plugin/connection.js'];hashes={n:hashlib.sha256((sdk/n).read_bytes()).hexdigest() for n in tracked}
 report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sdk_version':json.loads((sdk/'package.json').read_text())['version'],'usb_access':False,'checks':[]}
 with tempfile.TemporaryDirectory(prefix='deckforge-key-animation-') as temporary:
  root=Path(temporary);plugin,proof=make_plugin(root,sdk);animated=plugin/'motion.gif';gif(animated);original=animated.read_bytes();Image.new('RGB',(32,32),GREEN).save(plugin/'green.png')
  state=root/'state';host=Host(binary,[plugin],state,root/'host.log').ready()
  def call(method,value,controller="Keypad",**options):
   item=action(host,controller);nonce=str(uuid.uuid4());host.post({'command':'settings','context':item['context'],'settings':dict(item['settings'],displayProbe={'nonce':nonce,'method':method,'value':value,'options':options})})
   wait(host,proof,lambda:any(r.get('nonce')==nonce and r['kind']=='done' for r in rows(proof)),'SDK image call')
  try:
   wait(host,proof,lambda:host.state()['plugins'][0]['connected'],'SDK registration');host.post({'command':'assign','controller':'Keypad','index':0,'uuid':KEY})
   call('setImage','motion.gif',state=0);require(colors(host)=={RED,BLUE},'SDK file did not animate')
   call('setImage','green.png',state=0,target=1);require(colors(host)=={RED,BLUE},'Hardware write changed software animation')
   call('setImage','green.png',state=0,target=2);require(colors(host)=={GREEN},'Static replacement did not stop software animation')
   call('setImage','data:image/gif;base64,'+base64.b64encode(original).decode(),state=0,target=2);require(colors(host)=={RED,BLUE},'SDK data URL did not animate')
   call('setImage','green.png',state=1);call('setState',1);require(colors(host)=={GREEN},'Inactive animated state leaked')
   call('setState',0);require(colors(host)=={RED,BLUE},'Returning to animated state failed')
   host.post({'command':'assign','controller':'Encoder','index':0,'uuid':DIAL})
   lcd=action(host,'Encoder')['preview'];call('setImage','motion.gif',controller='Encoder')
   require(colors(host,controller='Encoder')=={RED,BLUE},'Encoder app icon did not animate')
   require(action(host,'Encoder')['preview']==lcd,'Animated app icon changed LCD feedback')
   report['checks'].append('unchanged SDK file/data URL, targeted writes, static replacement, state changes and animated encoder app icons')
   # Cached SDK bytes survive the source file disappearing and a restart without resending setImage.
   host.post({'command':'settings','context':action(host)['context'],'settings':{}});animated.unlink()
  finally:host.stop()
  host=Host(binary,[],state,root/'restart-sdk.log').ready()
  try:
   require(colors(host)=={RED,BLUE},'SDK animation cache did not survive restart')
   report['checks'].append('SDK original bytes survive source deletion and restart')
   custom=root/'custom.gif';gif(custom);custom_bytes=custom.read_bytes();item=action(host)
   host.post({'command':'appearance','context':item['context'],'expected_state':item['state'],'image_path':str(custom)})
   require(colors(host)=={RED,BLUE},'User GIF did not animate')
   call('setImage','green.png',state=0);require(colors(host)=={RED,BLUE},'SDK replaced user animation')
   exported=root/'animated.streamDeckProfile';host.post({'command':'export_profile','path':str(exported)})
   with zipfile.ZipFile(exported) as z:require(any(n.endswith('.gif') and z.read(n)==custom_bytes for n in z.namelist()),'Export flattened GIF bytes')
   custom.unlink()
  finally:host.stop()
  host=Host(binary,[],state,root/'restart-user.log').ready()
  try:
   require(colors(host)=={RED,BLUE},'User GIF restart failed');host.post({'command':'import_profile','path':str(exported)})
   require(colors(host)=={RED,BLUE},'Imported GIF did not animate')
   report['checks'].append('user override, original byte export, source deletion, restart and imported profile animation')
   item=action(host);host.post({'command':'appearance','context':item['context'],'expected_state':item['state'],'reset_image':True})
   require(len(colors(host))==1,'Reset did not retire imported animation')
   report['checks'].append('reset retires imported playback')
  finally:host.stop()
 require(all(hashlib.sha256((sdk/n).read_bytes()).hexdigest()==h for n,h in hashes.items()),'Official SDK modified')
 report['sdk_hashes']=hashes;report['ok']=True;args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2)+'\n')
 print('PASS:', '; '.join(report['checks']))
if __name__=='__main__':main()
