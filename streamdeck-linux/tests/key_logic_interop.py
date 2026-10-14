#!/usr/bin/env python3
"""Key Logic through a real simulated host and unchanged official Counter."""
import argparse,hashlib,json,subprocess,tempfile,time,zipfile
from pathlib import Path
from system_interop import Host
from sample_interop import COUNTER,SAMPLE_COMMIT,require,slot
from trigger_interop import make_probe,child,mark,events,wait_for,PROBE
LOGIC='com.elgato.streamdeck.keys.logic'
def main():
    p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--report',type=Path,default=Path('target/key-logic/process.json'));a=p.parse_args();a.report.parent.mkdir(parents=True,exist_ok=True)
    samples=Path(__file__).resolve().parents[2]/'evidence/official-samples';counter=samples/'counter/com.elgato.counter.sdPlugin'
    require(subprocess.check_output(['git','-C',str(samples),'rev-parse','HEAD'],text=True).strip()==SAMPLE_COMMIT,'Wrong sample commit')
    original={p:hashlib.sha256(p.read_bytes()).hexdigest() for p in [counter/'manifest.json',counter/'bin/plugin.js']}
    with tempfile.TemporaryDirectory(prefix='deckforge-keylogic-') as d:
        root=Path(d);probe=make_probe(root/'probe.sdPlugin',samples/'counter/package.json');h=Host(a.binary.resolve(),[probe,counter],root/'state',root/'host.log')
        def parent():return slot(h.state(),LOGIC)
        def edge(down):h.post({'command':'input','event':'keyDown' if down else 'keyUp','index':0})
        def click():edge(True);edge(False)
        try:
            h.ready();h.wait(lambda s:all(p['connected'] for p in s['plugins']),'plugin registration')
            h.post({'command':'assign','controller':'Keypad','index':0,'uuid':LOGIC})
            raw={'UUID':LOGIC,'Future':7,'Actions':[child(Settings={'tag':n}) for n in ['single','double','hold']]}
            h.post({'command':'logic_save','context':parent()['context'],'raw':raw})
            ids=[c['context'] for c in parent()['trigger_children']]
            wait_for(h,lambda:len(events(h,contexts=ids,kinds=['willAppear']))==3,'three child appearances')
            start=mark(h);click();time.sleep(.16);require(not events(h,start,ids,['keyDown']),'Single fired before classification')
            wait_for(h,lambda:len(events(h,start,[ids[0]],['keyUp']))==1,'single pulse')
            rows=events(h,start,ids,['keyDown','keyUp']);require([r['context'] for r in rows]==[ids[0],ids[0]],'Single selected wrong child')
            require(rows[1]['at']-rows[0]['at']>=50,'Single pulse compressed')
            start=mark(h);click();time.sleep(.08);edge(True)
            wait_for(h,lambda:bool(events(h,start,[ids[1]],['keyDown'])),'double starts on second press')
            time.sleep(.08);require(not events(h,start,ids,['keyUp']),'Double released before physical release');edge(False)
            wait_for(h,lambda:len(events(h,start,[ids[1]],['keyUp']))==1,'double release')
            time.sleep(.35);require(len(events(h,start,ids,['keyDown','keyUp']))==2,'Double also fired single')
            start=mark(h);edge(True);time.sleep(.35);require(not events(h,start,ids,['keyDown']),'Hold fired early')
            wait_for(h,lambda:bool(events(h,start,[ids[2]],['keyDown'])),'hold threshold');edge(False)
            wait_for(h,lambda:len(events(h,start,[ids[2]],['keyUp']))==1,'held release')
            for e in events(h,contexts=ids,kinds=['keyDown','keyUp']):
                require(e['payload']['controller']=='Keypad' and e['payload']['isInMultiAction'] is False,'Wrong child protocol')
            start=mark(h);edge(True);h.post({'command':'page_create','name':'Away'});time.sleep(.6);edge(False)
            require(not events(h,start,ids,['keyDown','keyUp']),'Departing pending gesture fired');h.post({'command':'page_step','direction':-1})
            # Counter opts out of Multi Actions; its independent Key Logic eligibility defaults true.
            raw=parent()['raw'];raw['Actions'][0]=child(COUNTER,Settings={'count':0,'incrementBy':3})
            h.post({'command':'logic_save','context':parent()['context'],'raw':raw});click()
            h.wait(lambda s:slot(s,LOGIC)['trigger_children'][0]['settings'].get('count')==3,'unchanged official Counter')
            saved=parent()['raw'];export=root/'logic.streamDeckProfile';h.post({'command':'export_profile','path':str(export)})
            h.post({'command':'import_profile','path':str(export)})
            require(parent()['raw']['Actions'][0]['Settings']['count']==3,'Export/import lost child settings')
            require(parent()['raw']['Future']==7,'Export/import lost unknown parent field')
            h.stop();h=Host(a.binary.resolve(),[probe,counter],root/'state',root/'restart.log');h.ready()
            require(parent()['trigger_children'][0]['settings']['count']==3,'Restart lost child')
            report={'ok':True,'single_pulse_ms':rows[1]['at']-rows[0]['at'],'double_and_hold':True,'departure_cancellation':True,'official_counter_increment':3,'export_import_restart':True,'original_windows_export_acceptance':False}
            a.report.write_text(json.dumps(report,indent=2)+'\n')
        finally:h.stop()
    require(all(hashlib.sha256(p.read_bytes()).hexdigest()==digest for p,digest in original.items()),'Official fixture modified')
    print('PASS: Key Logic gesture timing, balanced SDK events, cancellation, unchanged Counter, export/import and restart. Simulated device; no USB or desktop input.')
if __name__=='__main__':main()
