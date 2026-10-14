#!/usr/bin/env python3
"""Model geometry through the real host, plugin protocol, profiles and restart. No USB."""
import argparse
import base64
import io
import json
from pathlib import Path
import subprocess
import tempfile
import time
import zipfile
from PIL import Image
from system_interop import Host

MODELS = {
    'mini': (3, 2, 0, 1, '20GAI9901'),
    'classic': (5, 3, 0, 0, '20GBA9901'),
    'classic-2017': (5, 3, 0, 0, '20GAA9901'),
    'xl': (8, 4, 0, 2, '20GAT9901'),
    'plus': (4, 2, 4, 7, '20GBD9901'),
    'plus-xl': (9, 4, 6, 13, '20GBX9901'),
    'neo': (4, 2, 0, 9, '20GBJ9901'),
    'pedal': (3, 1, 0, 5, '20GBF9901'),
    'studio': (16, 2, 2, 10, '20GBO9901'),
    'galleon': (3, 4, 4, 12, 'GRETSCH'),
}
PLUGIN = 'com.deckforge.model-probe'
ACTION = PLUGIN + '.action'

class ModelHost(Host):
    def __init__(self, binary, plugin, state, log, model=None):
        self.log, self.output = log, log.open('w')
        self.process = subprocess.Popen([str(binary), 'serve', '--simulate', '--state', str(state), '--port', '0',
            '--plugin', str(plugin), *(['--model', model] if model else [])],
            stdout=self.output, stderr=subprocess.STDOUT, start_new_session=True)
        self.base = self.token = None

def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line] if path.exists() else []

def wait(host, fn):
    end = time.monotonic() + 10
    while time.monotonic() < end:
        host.check_process()
        value = fn()
        if value: return value
        time.sleep(.02)
    raise AssertionError('Plugin probe timed out: ' + host.log.read_text()[-3000:])

def make_plugin(root, project):
    root.mkdir()
    sdk = project.parent/'evidence/official-samples/counter/package.json'
    proof = root/'events.jsonl'
    code = r'''import {createRequire} from 'node:module';import {appendFileSync} from 'node:fs';
const WebSocket=createRequire(__SDK__)('ws'),proof=__PROOF__,args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
appendFileSync(proof,JSON.stringify({event:'launch',info:JSON.parse(args['-info'])})+'\n');
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',bytes=>{const m=JSON.parse(bytes);appendFileSync(proof,JSON.stringify(m)+'\n');
if(m.event==='willAppear'&&m.payload.controller==='Encoder'&&JSON.parse(args['-info']).devices.some(d=>d.id===m.device&&d.type===10)){
ws.send(JSON.stringify({event:'setFeedbackLayout',context:m.context,payload:{layout:'no-studio-display.json'}}));
ws.send(JSON.stringify({event:'setFeedback',context:m.context,payload:{value:42,title:'Studio dial'}}));
}
if(m.event==='willAppear'&&m.payload.controller==='Neo'){
ws.send(JSON.stringify({event:'setFeedbackLayout',context:m.context,payload:{layout:'neo.json'}}));
ws.send(JSON.stringify({event:'setFeedback',context:m.context,payload:{title:'Neo probe'}}));
}});'''
    (root/'plugin.mjs').write_text(code.replace('__SDK__',json.dumps(str(sdk))).replace('__PROOF__',json.dumps(str(proof))))
    (root/'manifest.json').write_text(json.dumps({'UUID':PLUGIN,'Name':'Model probe','CodePath':'plugin.mjs','SDKVersion':2,'Nodejs':{'Version':'20'},
        'Actions':[{'UUID':ACTION,'Name':'Model action','Controllers':['Keypad','Encoder','Neo'],'States':[{}]}]}))
    (root/'neo.json').write_text(json.dumps({'id':'neo-probe','controller':'Neo','items':[
        {'key':'title','type':'text','rect':[0,0,232,50],'value':'Initial','font':{'size':20},'alignment':'center'}]}))
    return proof

def main():
    project=Path(__file__).resolve().parents[1]
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    ap.add_argument('--report',type=Path)
    args=ap.parse_args();binary=args.binary.resolve(strict=True);report=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-models-') as directory:
        root=Path(directory);plugin=root/(PLUGIN+'.sdPlugin');proof=make_plugin(plugin,project)
        for model,(columns,rows_count,dials,sdk_type,profile_model) in MODELS.items():
            state=root/model;host=ModelHost(binary,plugin,state,root/(model+'.log'),model).ready()
            try:
                host.wait(lambda s:s['plugins'][0]['connected'],'plugin registration')
                snap=host.state();assert snap['device_model']==model
                assert snap['device_geometry']['keys']==columns*rows_count
                launch=wait(host,lambda: next((e for e in reversed(rows(proof)) if e['event']=='launch'),None))
                info=launch['info']['devices'][0]
                assert info['type']==sdk_type and info['size']=={'columns':columns,'rows':rows_count}, info
                index=columns*rows_count-1
                host.post({'command':'assign','controller':'Keypad','index':index,'uuid':ACTION})
                key=next(s for s in host.state()['slots'] if s['controller']=='Keypad' and s['index']==index)
                host.post({'command':'input','event':'keyDown','index':index})
                host.post({'command':'input','event':'keyUp','index':index})
                event=wait(host,lambda:next((e for e in reversed(rows(proof)) if e['event']=='keyUp' and e.get('context')==key['context']),None))
                assert event['payload']['coordinates']=={'column':columns-1,'row':rows_count-1},event
                host.post({'command':'assign','controller':'Keypad','index':index+1,'uuid':ACTION},expected=400)
                host.post({'command':'input','event':'keyDown','index':index+1},expected=400)
                host.post({'command':'assign','controller':'Encoder','index':dials,'uuid':ACTION},expected=400)
                if dials:
                    host.post({'command':'assign','controller':'Encoder','index':dials-1,'uuid':ACTION})
                    dial=next(s for s in host.state()['slots'] if s['controller']=='Encoder' and s['index']==dials-1)
                    host.post({'command':'input','event':'dialRotate','index':dials-1,'ticks':1})
                    event=wait(host,lambda:next((e for e in reversed(rows(proof)) if e['event']=='dialRotate' and e.get('context')==dial['context']),None))
                    assert event['payload']['coordinates']==({'column':1,'row':1} if model=='galleon' else {'column':dials-1,'row':0})
                    if model=='galleon':
                        assert snap['device_geometry']['physical_dials']==2
                        assert (snap['device_geometry']['encoder_columns'],snap['device_geometry']['encoder_rows'])==(2,2)
                        assert Image.open(io.BytesIO(base64.b64decode(dial['preview']))).size==(320,160)
                        assert Image.open(io.BytesIO(base64.b64decode(key['preview']))).size==(160,160)
                        for unsupported in ('touchTap','touchLongTap','swipe'):
                            host.post({'command':'input','event':unsupported,'index':0},expected=400)
                        host.post({'command':'assign','controller':'Encoder','index':3,'uuid':'com.elgato.streamdeck.keys.adaptor'})
                        parent=next(s for s in host.state()['slots'] if s['controller']=='Encoder' and s['index']==3)
                        host.post({'command':'trigger_save','context':parent['context'],'raw':{'Actions':[{'Actions':[]},{'UUID':ACTION,'ActionID':'bottom-right-key','Settings':{}},{'Actions':[]}]}})
                        child=next(s for s in host.state()['slots'] if s['controller']=='Encoder' and s['index']==3)['trigger_children'][1]
                        host.post({'command':'input','event':'dialDown','index':3})
                        host.post({'command':'input','event':'dialUp','index':3})
                        nested=wait(host,lambda:next((e for e in reversed(rows(proof)) if e['event']=='keyUp' and e.get('context')==child['context']),None))
                        assert nested['payload']['coordinates']=={'column':1,'row':1},nested
                    if model=='studio':
                        snap=host.wait(lambda s:any(x['controller']=='Encoder' and x['feedback'].get('value')==42 for x in s['slots']),'Studio feedback metadata retained without LCD layout')
                        encoder=next(x for x in snap['slots'] if x['controller']=='Encoder')
                        assert encoder['preview']=='' and encoder['layout']=='no-studio-display.json'
                        assert not snap['device_capabilities']['strip'] and not snap['device_capabilities']['touchscreen']
                        assert Image.open(io.BytesIO(base64.b64decode(key['preview']))).size==(144,112)
                        for event in ('touchTap','touchLongTap','swipe'):
                            host.post({'command':'input','event':event,'index':0},expected=400)
                        for event in ('dialDown','dialUp'):
                            host.post({'command':'input','event':event,'index':1})
                            message=wait(host,lambda:next((e for e in reversed(rows(proof)) if e['event']==event and e.get('context')==dial['context']),None))
                            assert message['payload']['coordinates']=={'column':1,'row':0}
                if model=='neo':
                    host.post({'command':'assign','controller':'Neo','index':1,'uuid':ACTION})
                    snap=host.wait(lambda s:any(x['controller']=='Neo' and x['feedback'].get('title')=='Neo probe' for x in s['slots']),'Neo feedback rendering')
                    neo=next(x for x in snap['slots'] if x['controller']=='Neo')
                    assert Image.open(io.BytesIO(base64.b64decode(neo['preview']))).size==(232,50)
                export=root/(model+'.streamDeckProfile')
                host.post({'command':'export_profile','path':str(export)})
                with zipfile.ZipFile(export) as archive:
                    manifests=[json.loads(archive.read(n)) for n in archive.namelist() if n.endswith('manifest.json')]
                assert any(m.get('Device',{}).get('Model')==profile_model for m in manifests),manifests
                assert any(f'{columns-1},{rows_count-1}' in c.get('Actions',{}) for m in manifests for c in m.get('Controllers',[]) if c['Type']=='Keypad')
                if dials:
                    assert any(('1,1' if model=='galleon' else f'{dials-1},0') in c.get('Actions',{}) for m in manifests for c in m.get('Controllers',[]) if c['Type']=='Encoder')
                host.post({'command':'import_profile','path':str(export)})
                assert any(s['controller']=='Keypad' and s['index']==index for s in host.state()['slots'])
            finally:
                host.stop()
                assert host.process.returncode==0
            restarted=ModelHost(binary,plugin,state,root/(model+'-restart.log')).ready()
            try:
                snap=restarted.state();assert snap['device_model']==model
                assert any(s['controller']=='Keypad' and s['index']==index for s in snap['slots'])
                assert snap['device_capabilities']['transport_supported'] is True
            finally:restarted.stop()
            report.append({'model':model,'keys':columns*rows_count,'dials':dials,'sdk_type':sdk_type,'profile_model':profile_model,'restart':True,'usb_access':False})
        # The two 15-key hardware generations share profile coordinates, even
        # though their USB transports and brightness curves differ.
        for source,target in [('classic','classic-2017'),('classic-2017','classic')]:
            state=root/('cross-'+target);host=ModelHost(binary,plugin,state,root/('cross-'+target+'.log'),target).ready()
            try:
                host.post({'command':'import_profile','path':str(root/(source+'.streamDeckProfile'))})
                snap=host.state();profile=next(p for p in snap['library']['profiles'] if p['id']==snap['library']['active_profile'])
                assert profile['model']==source and profile['compatible'] is True
                assert snap['device_model']==target
                assert any(s['controller']=='Keypad' and s['index']==14 for s in snap['slots'])
            finally:host.stop()
    if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: all ten model layouts, final key coordinates, dial limits, Neo feedback, screenless Studio encoders, Galleon 2D widgets/nested coordinates, export/import and persisted-model restart and 15-key cross-generation profiles; no USB.')

if __name__=='__main__':main()
