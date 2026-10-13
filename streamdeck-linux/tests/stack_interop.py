#!/usr/bin/env python3
"""Dial Stack lifecycle, routing, live PNGs and profile persistence; no USB opened."""
import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import tempfile
import time
import zipfile
from PIL import Image,ImageChops
from system_interop import Host
from sample_interop import require,slot
STACK='com.elgato.streamdeck.dial.stack'
PROBE='com.deckforge.stack-probe.encoder'
FEEDBACK='com.elgato.layouts.feedback'

def create_probe(path,require_root):
    path.mkdir()
    (path/'plugin.js').write_text(r'''import {createRequire} from 'node:module';
const WebSocket=createRequire(__ROOT__)('ws');const args={};for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);const send=(event,context,payload)=>ws.send(JSON.stringify({event,context,payload}));
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{const m=JSON.parse(data);if(!m.payload)return;
if(['willAppear','willDisappear','dialDown','dialUp','dialRotate','touchTap','propertyInspectorDidAppear'].includes(m.event))send('logMessage',args['-pluginUUID'],{message:'STACK '+JSON.stringify({event:m.event,context:m.context,payload:m.payload})});
if(m.event==='willAppear')send('setFeedback',m.context,{title:m.payload.settings.tag,value:m.payload.settings.value,indicator:{value:m.payload.settings.value}});
if(m.event==='dialRotate'){const value=(m.payload.settings.value||0)+m.payload.ticks;send('setSettings',m.context,{...m.payload.settings,value});send('setFeedback',m.context,{value,indicator:{value}});}
if(m.event==='willDisappear')setTimeout(()=>{send('setSettings',m.context,{...m.payload.settings,late:true});send('setFeedback',m.context,{title:'Hidden update'});},80);
});'''.replace('__ROOT__',json.dumps(str(require_root))))
    (path/'manifest.json').write_text(json.dumps({'UUID':'com.deckforge.stack-probe','Name':'Stack probe','CodePath':'plugin.js','Nodejs':{'Version':'20'},'SDKVersion':2,'Actions':[{'UUID':PROBE,'Name':'Encoder probe','Controllers':['Encoder'],'Encoder':{'layout':'$B1'},'States':[{}],'SupportedInMultiActions':False,'PropertyInspectorPath':'pi.html'}]}))
    (path/'pi.html').write_text('''<!doctype html><html><body>Stack child inspector<script>window.connectElgatoStreamDeckSocket=(port,id,event,info,action)=>{const ws=new WebSocket('ws://127.0.0.1:'+port);ws.onopen=()=>ws.send(JSON.stringify({event,uuid:id}));};</script></body></html>''')
    return path

def events(state,kind=None):
    rows=[json.loads(line.split('STACK ',1)[1]) for line in state['logs'] if 'STACK ' in line]
    return [r for r in rows if kind is None or r['event']==kind]

def press(host,down=True):host.post({'command':'input','event':'dialDown' if down else 'dialUp','index':0})
def click(host):press(host);press(host,False)
def rotate(host,ticks):host.post({'command':'input','event':'dialRotate','index':0,'ticks':ticks})
def current(host):return slot(host.state(),STACK)
def image(parent):return Image.open(io.BytesIO(base64.b64decode(parent['preview']))).convert('RGB')
def save(host,parent,**settings):
    raw=parent['raw'];raw['Settings']=dict(parent['settings'],**settings)
    host.post({'command':'stack_save','context':parent['context'],'raw':raw})

def assign(host,action=PROBE,values=(10,20,30)):
    host.post({'command':'assign','controller':'Encoder','index':0,'uuid':STACK})
    parent=current(host)
    raw={'FutureContainer':91,'Settings':{'CurrentIdx':0,'FastNavigation':False,'ShowStackIcon':True},'Actions':[{'UUID':action,'ActionID':f'entry-{i}','Name':f'Control {i+1}','Settings':{'tag':f'Control {i+1}','value':value},'State':0,'States':[{}],'FutureChild':i} for i,value in enumerate(values)]}
    host.post({'command':'stack_save','context':parent['context'],'raw':raw})
    return current(host)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));args=parser.parse_args()
    evidence=Path(__file__).resolve().parents[2]/'evidence';layouts=evidence/'plugins/com.elgato.layouts.sdPlugin'
    with tempfile.TemporaryDirectory(prefix='deckforge-stacks-') as directory:
        root=Path(directory);state=root/'state';probe=create_probe(root/'probe.sdPlugin',evidence/'official-samples/counter/package.json')
        host=Host(args.binary.resolve(),[probe,layouts],state,root/'host.log')
        try:
            host.ready();host.wait(lambda s:all(p['connected'] for p in s['plugins']),'plugins connected')
            parent=assign(host);ids=[c['context'] for c in parent['stack_children']]
            host.wait(lambda s:any(e['context']==ids[0] for e in events(s,'willAppear')),'first stack child appeared')
            require(not any(e['context'] in ids[1:] for e in events(host.state(),'willAppear')),'Inactive stack child appeared')
            first=next(e for e in events(host.state(),'willAppear') if e['context']==ids[0])
            require(first['payload']['controller']=='Encoder' and first['payload']['isInMultiAction'] is False and first['payload']['coordinates']=={'column':0,'row':0},'Stack child is not a normal encoder context')
            rotate(host,7);host.wait(lambda s:slot(s,STACK)['stack_children'][0]['settings']['value']==17,'active child rotation')
            press(host);require(current(host)['settings']['CurrentIdx']==0,'Press-down changed current prematurely');press(host,False)
            host.wait(lambda s:slot(s,STACK)['settings']['CurrentIdx']==1 and slot(s,STACK)['stack_children'][1]['feedback'].get('value')==20,'press release cycled')
            require(not any(e['context'] in ids for e in events(host.state()) if e['event'] in ('dialDown','dialUp')),'Container push leaked to child')
            before=image(current(host));time.sleep(.15)
            require(ImageChops.difference(before,image(current(host))).getbbox() is None,'Inactive feedback replaced active display')
            require(current(host)['stack_children'][0]['settings'].get('late') is True,'Inactive child settings reply lost')
            # Marker affects only its verified overlay rectangle.
            save(host,current(host),ShowStackIcon=False);after=image(current(host));box=ImageChops.difference(before,after).getbbox()
            require(box and box[0]>=160 and box[1]>=8 and box[2]<=186 and box[3]<=34,f'Stack mark changed pixels outside expected region: {box}')
            press(host);rotate(host,3);press(host,False)
            host.wait(lambda s:slot(s,STACK)['stack_children'][1]['settings']['value']==23,'pressed rotation forwarded')
            require(current(host)['settings']['CurrentIdx']==1,'Normal held rotation also cycled')
            forwarded=[e for e in events(host.state(),'dialRotate') if e['context']==ids[1]][-1]
            require(forwarded['payload']['pressed'] is True,'Pressed rotation flag lost')
            # Fast navigation defers selection until release and consumes rotation.
            host.post({'command':'stack_select','context':parent['context'],'index':0})
            save(host,current(host),FastNavigation=True)
            prior=len(events(host.state(),'dialRotate'));press(host);rotate(host,9)
            require(current(host)['settings']['CurrentIdx']==0,'Fast pending child became active before release')
            host.post({'command':'input','event':'touchTap','index':0,'x':155,'y':20,'hold':False})
            host.wait(lambda s:any(e['context']==ids[0] for e in events(s,'touchTap')),'touch stayed on current child')
            touch=[e for e in events(host.state(),'touchTap') if e['context']==ids[0]][-1]
            require(touch['payload']['tapPos']==[155,20],'Touch coordinates changed')
            press(host,False);host.wait(lambda s:slot(s,STACK)['settings']['CurrentIdx']==2,'first fast CW committed current plus two')
            rotate(host,1);time.sleep(.08)
            require(len(events(host.state(),'dialRotate'))==prior,'Fast navigation/cooldown leaked rotation')
            time.sleep(.3);rotate(host,4)
            host.wait(lambda s:slot(s,STACK)['stack_children'][2]['settings']['value']==34,'rotation resumed after deadline')
            # Inactive inspectors require explicit activation first.
            host.post({'command':'open_inspector','context':ids[1]},expected=400)
            host.post({'command':'stack_select','context':parent['context'],'index':1})
            host.post({'command':'open_inspector','context':ids[1]})
            host.wait(lambda s:any(i['context']==ids[1] and i['connected'] for i in s['inspectors']),'selected child inspector')
            # Select another page: departing child disappears, settings remain.
            host.post({'command':'page_create','name':'Other'})
            host.wait(lambda s:any(e['context']==ids[1] for e in events(s,'willDisappear')),'page change child disappearance')
            original_page=json.loads((state/'session.json').read_text())['library']['profiles'][0]['page_order'][0]
            host.post({'command':'page_select','id':original_page})
            require([c['context'] for c in current(host)['stack_children']]==ids,'Child identities changed with page navigation')
            # Unchanged official Layouts plugin renders distinct instances.
            parent=assign(host,FEEDBACK,(40,70));contexts=[c['context'] for c in parent['stack_children']]
            host.wait(lambda s:slot(s,STACK)['stack_children'][0]['feedback'].get('value')==40,'official first feedback')
            original=image(current(host))
            require(original.crop((16,40,64,88)).convert('L').getextrema()[1]>200,'Selected official child icon was not rendered')
            rotate(host,7)
            host.wait(lambda s:slot(s,STACK)['stack_children'][0]['settings'].get('value')==47,'official dial input')
            require(ImageChops.difference(original,image(current(host))).getbbox() is not None,'Official feedback did not change actual pixels')
            click(host);host.wait(lambda s:slot(s,STACK)['stack_children'][1]['feedback'].get('value')==70,'official second instance')
            require(current(host)['stack_children'][0]['settings']['value']==47,'Second instance overwrote first settings')
            output=root/'stack.streamDeckProfile';host.post({'command':'export_profile','path':str(output)})
            with zipfile.ZipFile(output) as archive:
                rows=[a for n in archive.namelist() if n.endswith('manifest.json') for c in json.loads(archive.read(n)).get('Controllers',[]) for a in (c.get('Actions') or {}).values()]
            exported=next(a for a in rows if a.get('UUID')==STACK)
            require(len(exported['Actions'])==2 and exported['Actions'][0]['UUID']==FEEDBACK and exported['Actions'][0]['Settings']['value']==47,'Export did not preserve flat child structure')
            require(exported['FutureContainer']==91 and exported['Actions'][0]['FutureChild']==0,'Unknown stack fields lost')
        finally:host.stop()
        restart=Host(args.binary.resolve(),[],state,root/'restart.log')
        try:
            restart.ready();restart.wait(lambda s:all(p['connected'] for p in s['plugins']),'restart plugins')
            restart.wait(lambda s:slot(s,STACK)['stack_children'][1]['feedback'].get('value')==70,'restart selected child feedback')
            require(current(restart)['settings']['CurrentIdx']==1 and [c['context'] for c in current(restart)['stack_children']]==contexts,'Stack selection/identities lost on restart')
            require(current(restart)['stack_children'][0]['settings']['value']==47,'Inactive settings lost on restart')
            before=current(restart)['raw']
            restart.post({'command':'stack_save','context':current(restart)['context'],'raw':{'Actions':[{'UUID':STACK}]}},expected=400)
            require(current(restart)['raw']==before,'Invalid nested stack modified data')

        finally:restart.stop()
        # The original is a Plus XL profile, not a Plus profile. Exercise it on
        # its own model so a default Plus host cannot hide a truncated import.
        fixture_state=root/'plus-xl-state'
        fixture_host=Host(args.binary.resolve(),[],fixture_state,root/'plus-xl.log',model='plus-xl')
        try:
            fixture_host.ready()
            require(fixture_host.state()['device_model']=='plus-xl','Original fixture needs its actual device model')
            original=evidence/'elgato-7.6/raw/flsLUxNAo3HxRBo8yay7EbNtFyJC9c'
            require(hashlib.sha256(original.read_bytes()).hexdigest()=='e4ce1ac8947e28266b852a0860c03c9437b7ed2a2e2bd5044e1b94a502a72675','Original stack fixture changed')
            fixture_host.post({'command':'import_profile','path':str(original)})
            saved=json.loads((fixture_state/'session.json').read_text())
            profile=next(p for p in saved['library']['profiles'] if p['id']==fixture_host.state()['library']['active_profile'])
            page=next(p for p in profile['pages'] if any(s['uuid']==STACK for s in p['slots']))
            fixture_host.post({'command':'page_select','id':page['id']})
            retained=[s for s in fixture_host.state()['slots'] if s['uuid']==STACK]
            with zipfile.ZipFile(original) as archive:
                original_stacks={a['ActionID']:a for name in archive.namelist() if name.endswith('manifest.json') for c in json.loads(archive.read(name)).get('Controllers',[]) for a in (c.get('Actions') or {}).values() if a.get('UUID')==STACK}
            require(len(retained)==4 and {s['index'] for s in retained}=={1,2,3,4},'Plus XL must retain all four original stacks at their original dial positions')
            for s in retained:
                source=original_stacks[s['raw']['ActionID']]
                require(len(s['stack_children'])==len(source['Actions']),'Original child count changed')
                require(s['settings']==source['Settings'],'Original stack settings changed')
            require(all(not c['available'] for s in retained for c in s['stack_children']),'Unavailable WaveLink incorrectly claimed available')
            untouched=root/'original-stack.streamDeckProfile';fixture_host.post({'command':'export_profile','path':str(untouched)})
            with zipfile.ZipFile(untouched) as archive:
                records=[a for name in archive.namelist() if name.endswith('manifest.json') for c in json.loads(archive.read(name)).get('Controllers',[]) for a in (c.get('Actions') or {}).values()]
            for identity,source in original_stacks.items():
                match=next(a for a in records if a.get('ActionID')==identity)
                require(match['Actions']==source['Actions'] and match['Settings']==source['Settings'],'Original WaveLink stack data changed on export')

        finally:fixture_host.stop()
    print('PASS: selected-only SDK lifecycle, ordinary Encoder payloads, click/pressed turn/touch/fast navigation/cooldown, inactive replies, PI, unchanged Layouts PNGs, flat export/restart and all four original Plus XL stacks. No USB or OS actions executed.')
if __name__=='__main__':main()
