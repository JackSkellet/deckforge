#!/usr/bin/env python3
"""Native HTML ES modules and local fetch; optional unchanged Home Assistant startup.

Owned loopback SDK server supplies empty globals. No Home Assistant instance,
credentials, hardware or saved user profile is used. Plugin files are read only.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

PROJECT = Path(__file__).resolve().parents[1]
SERVER = r'''
const fs=require('node:fs');const {createRequire}=require('node:module');
const requireSample=createRequire(process.argv[2]);const {WebSocketServer}=requireSample('ws');
const server=new WebSocketServer({host:'127.0.0.1',port:0});
const write=value=>fs.appendFileSync(process.argv[4],JSON.stringify(value)+'\n');
server.on('listening',()=>fs.writeFileSync(process.argv[3],String(server.address().port)));
server.on('connection',ws=>{ws.on('message',data=>{const m=JSON.parse(data);write({event:m.event,probe:m.event==='setGlobalSettings'&&m.payload?.moduleProbe?m.payload:null});
if(m.event==='getGlobalSettings')ws.send(JSON.stringify({event:'didReceiveGlobalSettings',payload:{settings:{}}}));
if(m.event==='getSettings')ws.send(JSON.stringify({event:'didReceiveSettings',context:'fixture-context',action:'de.perdoctus.streamdeck.homeassistant.entity',payload:{settings:{}}}));
});});
'''
MODULE = '''import {value} from './dependency.mjs';
window.connectElgatoStreamDeckSocket=async(port,uuid,event)=>{
 const ws=new WebSocket(`ws://127.0.0.1:${port}`);
 ws.onopen=async()=>{ws.send(JSON.stringify({event,uuid}));try {
 const data=await (await fetch('../data/settings.json')).json();
 const dynamic=await import('./dynamic.mjs');
 ws.send(JSON.stringify({event:'setGlobalSettings',payload:{moduleProbe:true,static:value,dynamic:dynamic.value,fetch:data.value,origin:location.origin}}));
 }catch(e){ws.send(JSON.stringify({event:'setGlobalSettings',payload:{moduleProbe:true,error:String(e)}}));}};
};'''

def stop(child):
    if child and child.poll() is None:
        child.terminate()
        try: child.wait(timeout=5)
        except subprocess.TimeoutExpired: child.kill();child.wait(timeout=5)

def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

def run_case(binary, entry, root, case, output, sample_package, expect_module=False, inspector=False):
    case.mkdir()
    server_script=case/'sdk.cjs';server_script.write_text(SERVER)
    port_file=case/'port';events=case/'events.jsonl'
    env=os.environ.copy()
    for key in ('XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_DATA_HOME','XDG_CACHE_HOME'):
        env[key]=str(case/key.lower())
    server=helper=None
    try:
        server=subprocess.Popen(['node',str(server_script),str(sample_package),str(port_file),str(events)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        deadline=time.monotonic()+5
        while not port_file.exists() and time.monotonic()<deadline:
            if server.poll() is not None: raise AssertionError('Owned SDK server failed')
            time.sleep(.02)
        assert port_file.exists(),'SDK server did not bind'
        info={'application':{'platform':'linux','platformVersion':'linux','version':'7.6.0','language':'en','font':'DejaVu Sans'},'plugin':{'uuid':'de.perdoctus.streamdeck.homeassistant','version':'3.8.4.66'},'devices':[],'devicePixelRatio':1,'colors':{}}
        cmd=[str(binary),'webview','--path',str(entry),'--port',port_file.read_text(),'--uuid','fixture-registration','--register-event','registerPropertyInspector' if inspector else 'registerPlugin','--info',json.dumps(info)]
        # Older builds are intentionally tested before the new optional root flag.
        if root is not None: cmd+=['--root',str(root)]
        if inspector:cmd+=['--action-info',json.dumps({'action':'de.perdoctus.streamdeck.homeassistant.entity','context':'fixture-context','device':'fixture-device','payload':{'settings':{}}})]
        logfile=output/(case.name+'.log')
        with logfile.open('w') as log:
            helper=subprocess.Popen(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
            deadline=time.monotonic()+12
            success=False
            while time.monotonic()<deadline:
                seen=rows(events)
                if expect_module:
                    probe=next((r['probe'] for r in seen if r.get('probe')),None)
                    if probe:
                        assert probe.get('static')==17 and probe.get('dynamic')==23 and probe.get('fetch')==31,probe
                        success=True;break
                elif any(r['event']==('registerPropertyInspector' if inspector else 'registerPlugin') for r in seen) and any(r['event']=='getGlobalSettings' for r in seen):
                    success=True;break
                if helper.poll() is not None: break
                time.sleep(.04)
            result={'entry_sha256':hashlib.sha256(entry.read_bytes()).hexdigest(),'registered':any(r['event'].startswith('register') for r in rows(events)),'registration_count':sum(r['event'].startswith('register') for r in rows(events)),'success':success,'events':[r['event'] for r in rows(events)],'probe':next((r['probe'] for r in rows(events) if r.get('probe')),None),'exit_code':helper.poll()}
            if not success:result['diagnostic']=logfile.read_text()[-1500:]
            return result
    finally:
        stop(helper);stop(server)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=PROJECT/'target/debug/deckforge')
    parser.add_argument('--home-assistant',type=Path)
    parser.add_argument('--before',action='store_true')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();binary=args.binary.resolve(strict=True);output=args.output.resolve();output.mkdir(parents=True,exist_ok=True)
    sample=PROJECT.parent/'evidence/official-samples/counter/package.json'
    report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'before':args.before,'vendor_files_modified':False,'credentials_used':False,'home_assistant_servers_configured':False}
    with tempfile.TemporaryDirectory(prefix='deckforge-html-modules-') as temporary:
        temp=Path(temporary);plugin=temp/'plugin';(plugin/'assets').mkdir(parents=True);(plugin/'data').mkdir()
        (plugin/'plugin.html').write_text('<!doctype html><script type="module" crossorigin src="./assets/main.mjs"></script>')
        (plugin/'assets/main.mjs').write_text(MODULE)
        (plugin/'assets/dependency.mjs').write_text('export const value=17;')
        (plugin/'assets/dynamic.mjs').write_text('export const value=23;')
        (plugin/'data/settings.json').write_text('{"value":31}')
        report['module']=run_case(binary,plugin/'plugin.html',None if args.before else plugin,temp/'module',output,sample,True)
        if not args.before:
            (plugin/'delayed.html').write_text('<!doctype html><script type="module" src="./assets/delayed.mjs"></script>')
            (plugin/'assets/delayed.mjs').write_text('await new Promise(resolve=>setTimeout(resolve,200));\n'+MODULE)
            report['delayed']=run_case(binary,plugin/'delayed.html',plugin,temp/'delayed',output,sample,True)
            assert report['delayed']['success'] and report['delayed']['registration_count']==1,report['delayed']
            (plugin/'missing.html').write_text('<!doctype html><title>No registration callback</title>')
            report['missing_callback']=run_case(binary,plugin/'missing.html',plugin,temp/'missing',output,sample)
            assert not report['missing_callback']['registered'] and report['missing_callback']['exit_code']==1,report['missing_callback']
        if args.home_assistant:
            ha=args.home_assistant.resolve(strict=True)
            for name,inspector in [('plugin',False),('pi',True)]:
                report['homeassistant_'+name]=run_case(binary,ha/(name+'.html'),None if args.before else ha,temp/('homeassistant_'+name),output,sample,inspector=inspector)
        (output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
        cases=[v for k,v in report.items() if k in ('module','homeassistant_plugin','homeassistant_pi')]
        assert all(v['success']!=args.before for v in cases),report
        print('PASS: '+('old runtime reproduces module and callback startup failures' if args.before else 'native modules, local fetch and selected HTML plugins register'))

if __name__=='__main__':main()
