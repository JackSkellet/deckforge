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
    'xl': (8, 4, 0, 2, '20GAT9901'),
    'plus': (4, 2, 4, 7, '20GBD9901'),
    'plus-xl': (9, 4, 6, 13, '20GBX9901'),
    'neo': (4, 2, 0, 9, '20GBJ9901'),
    'pedal': (3, 1, 0, 5, '20GBF9901'),
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
                    assert event['payload']['coordinates']=={'column':dials-1,'row':0}
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
    if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: all seven model layouts, final key coordinates, dial limits, Neo feedback, export/import and persisted-model restart; no USB.')

if __name__=='__main__':main()
