#!/usr/bin/env python3
"""Action Wheel key lifecycle, paced selection, native settings and portable profiles. No USB."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import time
import zipfile
from system_interop import Host
from sample_interop import require, COUNTER, COUNTER_LOCK_SHA256
from multi_interop import make_probe, ACTION, events

WHEEL='com.elgato.streamdeck.keys.stack'

def wheel(host):return next(s for s in host.state()['slots'] if s['uuid']==WHEEL)
def children(host):return wheel(host)['multi_children'][0]
def input_(host,event,**extra):host.post({'command':'input','event':event,'index':0,**extra})
def save(host,raw):host.post({'command':'wheel_save','context':wheel(host)['context'],'raw':raw})
def selected(host,index):host.wait(lambda s:next(x for x in s['slots'] if x['uuid']==WHEEL)['settings'].get('CurrentIdx')==index,'wheel selection')
def configure(host,action,count):
 raw={'FutureWheel':73,'Settings':{'CurrentIdx':0,'InvertRotation':False,'ExtTitles':False,'ControlOffset':0},'Actions':[{'ActionID':f'wheel-{i}','UUID':action,'Name':f'Item {i+1}','Settings':{'count':0,'incrementBy':i+1},'State':0,'States':[{},{}],'FutureChild':i} for i in range(count)]}
 save(host,raw); return raw

def main():
 project=Path(__file__).resolve().parents[1]
 ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge');ap.add_argument('--output',type=Path,required=True);args=ap.parse_args();binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
 sample=project.parent/'evidence/official-samples/counter'; require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'Official sample changed')
 with tempfile.TemporaryDirectory(prefix='deckforge-wheel-') as directory:
  root=Path(directory);state=root/'state';probe=make_probe(root/'probe.sdPlugin',sample/'package.json')
  host=Host(binary,[probe,sample/'com.elgato.counter.sdPlugin'],state,root/'host.log').ready()
  try:
   host.wait(lambda s:all(p['connected'] for p in s['plugins']),'plugin registrations')
   host.post({'command':'assign','controller':'Encoder','index':0,'uuid':WHEEL});configure(host,ACTION,6)
   ids=[c['context'] for c in children(host)]
   host.wait(lambda s:all(any(e['context']==id for e in events(s,'willAppear')) for id in ids),'all wheel key children appeared')
   for entry in events(host.state(),'willAppear'):
    if entry['context'] in ids:require(entry['payload']['controller']=='Keypad' and entry['payload']['isInMultiAction'] is False,'Wheel child has wrong SDK controller')
   appearance_count=len([e for e in events(host.state(),'willAppear') if e['context'] in ids])
   input_(host,'dialRotate',ticks=127);selected(host,5)
   require(len([e for e in events(host.state(),'willAppear') if e['context'] in ids])==appearance_count,'Selection restarted child lifecycle')
   require(not [e for e in events(host.state(),'keyDown') if e['context'] in ids],'Rotation activated a key')
   input_(host,'dialDown');host.wait(lambda s:any(e['context']==ids[5] for e in events(s,'keyDown')),'selected physical push')
   input_(host,'dialRotate',ticks=1);time.sleep(.15);require(wheel(host)['settings']['CurrentIdx']==5,'Held turn selected another action')
   input_(host,'dialUp');host.wait(lambda s:any(e['context']==ids[5] for e in events(s,'keyUp')),'selected release')
   prior=len([e for e in events(host.state(),'keyDown') if e['context']==ids[5]])
   input_(host,'touchTap',x=100,y=35,hold=False)
   host.wait(lambda s:len([e for e in events(s,'keyUp') if e['context']==ids[5]])==2,'tap released')
   downs=[e for e in events(host.state(),'keyDown') if e['context']==ids[5]];ups=[e for e in events(host.state(),'keyUp') if e['context']==ids[5]]
   require(len(downs)==prior+1 and 85<=ups[-1]['at']-downs[-1]['at']<=300,'Wheel tap did not use a 100 ms key hold')
   input_(host,'touchTap',x=25,y=35,hold=False);selected(host,4)
   raw=copy.deepcopy(wheel(host)['raw']);raw['Settings']['InvertRotation']=True;raw['Settings']['ExtTitles']=True;save(host,raw)
   input_(host,'dialRotate',ticks=1);selected(host,3)
   input_(host,'touchTap',x=25,y=35,hold=False);selected(host,4)
   page=host.state()['library']['active_page'];input_(host,'dialDown')
   host.post({'command':'page_create','name':'Away'})
   host.wait(lambda s:any(e['context']==ids[4] for e in events(s,'keyUp')),'page-hide release')
   input_(host,'dialUp');host.post({'command':'page_select','id':page})
   require([c['context'] for c in children(host)]==ids,'Page navigation changed child identities')
   parent=wheel(host);host.post({'command':'wheel_save','context':parent['context'],'raw':{'Actions':[{'UUID':WHEEL}]}},expected=400)
   require(wheel(host)['raw']==parent['raw'],'Rejected wheel save changed its data')
   configure(host,COUNTER,2);counter_ids=[c['context'] for c in children(host)]
   host.wait(lambda s:all(c['title']=='0' for c in next(x for x in s['slots'] if x['uuid']==WHEEL)['multi_children'][0]),'unchanged Counter titles')
   input_(host,'dialDown');input_(host,'dialUp');host.wait(lambda s:next(x for x in s['slots'] if x['uuid']==WHEEL)['multi_children'][0][0]['settings']['count']==1,'Counter consumed physical push')
   input_(host,'touchTap',x=145,y=35,hold=False);selected(host,1)
   input_(host,'touchTap',x=120,y=35,hold=False);host.wait(lambda s:next(x for x in s['slots'] if x['uuid']==WHEEL)['multi_children'][0][1]['settings']['count']==2,'Counter consumed two-item centre tap')
   exported=root/'wheel.streamDeckProfile';host.post({'command':'export_profile','path':str(exported)})
   with zipfile.ZipFile(exported) as z:
    records=[a for n in z.namelist() if n.endswith('manifest.json') for c in json.loads(z.read(n)).get('Controllers',[]) for a in (c.get('Actions') or {}).values()]
   record=next(a for a in records if a.get('UUID')==WHEEL)
   require(len(record['Actions'])==2 and record['FutureWheel']==73 and record['Actions'][1]['FutureChild']==1,'Export lost flat wheel data')
   # A wheel inside a Dial Stack receives touch input; changing the selected
   # stack child cancels the old wheel's pulse instead of activating the new one.
   stack_id='com.elgato.streamdeck.dial.stack'
   host.post({'command':'assign','controller':'Encoder','index':1,'uuid':stack_id})
   stack=next(s for s in host.state()['slots'] if s['uuid']==stack_id and s['index']==1)
   nested=[{'UUID':WHEEL,'ActionID':f'nested-wheel-{i}','Settings':{'CurrentIdx':0},'States':[{}],'Actions':[{'UUID':COUNTER,'ActionID':'nested-counter','Settings':{'count':count,'incrementBy':1},'States':[{}]}]} for i,count in enumerate([10,30])]
   host.post({'command':'stack_save','context':stack['context'],'raw':{'Settings':{'CurrentIdx':0},'Actions':nested}})
   def nested_stack(snapshot):return next(s for s in snapshot['slots'] if s['uuid']==stack_id and s['index']==1)
   host.wait(lambda s:nested_stack(s)['stack_children'][0]['multi_children'][0][0]['title']=='10','nested wheel Counter appearance')
   host.post({'command':'input','event':'touchTap','index':1,'x':300,'y':35,'hold':False})
   host.post({'command':'input','event':'dialDown','index':1});host.post({'command':'input','event':'dialUp','index':1})
   host.wait(lambda s:nested_stack(s)['settings']['CurrentIdx']==1 and nested_stack(s)['stack_children'][1]['multi_children'][0][0]['title']=='30','stack switched wheels')
   time.sleep(.2)
   state_stack=nested_stack(host.state())
   require(state_stack['stack_children'][0]['multi_children'][0][0]['settings']['count']==11,'Nested wheel did not activate its selected Counter')
   require(state_stack['stack_children'][1]['multi_children'][0][0]['settings']['count']==30,'Retired wheel pulse activated the replacement')
  finally:host.stop()
  host=Host(binary,[],state,root/'restart.log').ready()
  try:
   host.wait(lambda s:all(p['connected'] for p in s['plugins']),'restart')
   require([c['context'] for c in children(host)]==counter_ids,'Restart changed wheel child identities')
   require(wheel(host)['settings']['CurrentIdx']==1,'Restart lost selection')
   require([c['settings']['count'] for c in children(host)]==[1,2],'Restart lost independent Counter settings')
   host.post({'command':'import_profile','path':str(exported)})
   require([c['settings']['count'] for c in children(host)]==[1,2],'Profile import lost child settings')
  finally:host.stop()
  original=project.parent/'evidence/elgato-7.6/raw/flsLUxNAo3HxRBo8yay7EbNtFyJC9c';require(hashlib.sha256(original.read_bytes()).hexdigest()=='e4ce1ac8947e28266b852a0860c03c9437b7ed2a2e2bd5044e1b94a502a72675','Original profile changed')
  host=Host(binary,[],root/'original',root/'original.log',model='plus-xl').ready()
  try:
   host.post({'command':'import_profile','path':str(original)})
   saved=json.loads((root/'original/session.json').read_text());profile=next(p for p in saved['library']['profiles'] if p['id']==host.state()['library']['active_profile']);page=next(p for p in profile['pages'] if any(s['uuid']==WHEEL for s in p['slots']))
   host.post({'command':'page_select','id':page['id']});require(len(children(host))==6,'Original wheel lost children')
   untouched=root/'original-wheel.streamDeckProfile';host.post({'command':'export_profile','path':str(untouched)})
   def original_record(path):
    with zipfile.ZipFile(path) as z:
     return next(a for n in z.namelist() if n.endswith('manifest.json') for c in json.loads(z.read(n)).get('Controllers',[]) for a in (c.get('Actions') or {}).values() if a.get('UUID')==WHEEL)
   before,after=original_record(original),original_record(untouched)
   require(before['Actions']==after['Actions'] and before['Settings']==after['Settings'],'Original wheel content changed on export')
  finally:host.stop()
 (output/'report.json').write_text(json.dumps({'ok':True,'usb_access':False,'key_children':6,'tap_hold_ms':ups[-1]['at']-downs[-1]['at'],'counter_independence':[1,2],'original_profile_roundtrip':True,'nested_stack_pulse_isolation':True,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()},indent=2)+'\n')
 print('PASS: wheel selection, six live key children, press/tap ownership and timing, invert, page release, unchanged Counter, export/restart and original Plus XL wheel; no USB.')

if __name__=='__main__':main()
