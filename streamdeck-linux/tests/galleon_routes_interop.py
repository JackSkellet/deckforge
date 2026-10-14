#!/usr/bin/env python3
"""Exercise the recovered Galleon physical-to-logical route contract; no USB."""
import argparse,json,tempfile,time,zipfile
from pathlib import Path
from models_interop import ModelHost,make_plugin,rows,wait,ACTION,PLUGIN

def main():
    project=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--report',type=Path,default=project/'target/galleon/routes.json');a=p.parse_args();a.report.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-galleon-routes-') as d:
        root=Path(d);plugin=root/(PLUGIN+'.sdPlugin');proof=make_plugin(plugin,project)
        source=root/'routed.streamDeckProfile'
        with zipfile.ZipFile(source,'w') as z:
            z.writestr('manifest.json',json.dumps({'Name':'Routed Galleon','Device':{'Model':'GRETSCH'},'Controllers':[{'Type':'Encoder','Future':7,'Routers':{'0,0':'0,1','1,0':'1,1'},'Actions':{
                point:{'UUID':ACTION,'ActionID':point,'Settings':{'point':point},'States':[{}]} for point in ['0,0','0,1','1,1']}}]}))
        state=root/'state';h=ModelHost(a.binary.resolve(),plugin,state,root/'host.log','galleon').ready()
        def physical(event,index=0,**extra):h.post({'command':'input','physical':True,'event':event,'index':index,**extra})
        def mark():return len(rows(proof))
        def events(start):return [e for e in rows(proof)[start:] if e['event'] in ['dialDown','dialUp','dialRotate']]
        try:
            h.wait(lambda s:s['plugins'][0]['connected'],'probe registration');h.post({'command':'import_profile','path':str(source)})
            assert h.state()['encoder_routes']==[2,3]
            begin=mark();physical('dialRotate',ticks=2)
            event=wait(h,lambda:next((e for e in events(begin) if e['event']=='dialRotate'),None));assert event['payload']['coordinates']=={'column':0,'row':1}
            begin=mark();physical('dialDown');time.sleep(.12);assert not events(begin)
            physical('dialRotate',ticks=-1);event=wait(h,lambda:next((e for e in events(begin) if e['event']=='dialRotate'),None));assert event['payload']['pressed'] is True
            physical('dialUp');wait(h,lambda:any(e['event']=='dialDown' for e in events(begin)));wait(h,lambda:any(e['event']=='dialUp' for e in events(begin)))
            assert [e['event'] for e in events(begin)]==['dialRotate','dialDown','dialUp']
            begin=mark();physical('dialDown');time.sleep(.6);physical('dialUp');h.wait(lambda s:s['encoder_routes']==[0,3],'row changed after release');assert not events(begin)
            physical('dialRotate',ticks=1);event=wait(h,lambda:next((e for e in events(begin) if e['event']=='dialRotate'),None));assert event['payload']['coordinates']=={'column':0,'row':0}
            begin=mark();physical('dialDown',1);wait(h,lambda:any(e['event']=='dialDown' for e in events(begin)));time.sleep(.55);physical('dialUp',1)
            wait(h,lambda:any(e['event']=='dialUp' for e in events(begin)));assert h.state()['encoder_routes']==[0,3]
            # The first short click's replay remains owned while a quicker second click queues.
            begin=mark();physical('dialDown');time.sleep(.25);physical('dialUp');physical('dialDown');time.sleep(.02);physical('dialUp')
            wait(h,lambda:len(events(begin))>=4);assert [e['event'] for e in events(begin)]==['dialDown','dialUp','dialDown','dialUp']
            begin=mark();physical('dialDown');h.post({'command':'physical_dial_cancel','index':0});physical('dialUp');time.sleep(.1);assert not events(begin)
            page=h.state()['library']['active_page'];begin=mark();physical('dialDown');h.post({'command':'page_create','name':'Away'});time.sleep(.55);physical('dialUp');assert not events(begin)
            h.post({'command':'page_select','id':page});assert h.state()['encoder_routes']==[0,3]
            output=root/'roundtrip.streamDeckProfile';h.post({'command':'export_profile','path':str(output)})
            with zipfile.ZipFile(output) as z:
                controllers=[c for n in z.namelist() if n.endswith('manifest.json') for c in (json.loads(z.read(n)).get('Controllers') or []) if c['Type']=='Encoder']
                controller=next(c for c in controllers if c.get('Future')==7);assert controller['Routers']=={'0,0':'0,0','1,0':'1,1'}
            h.stop();h=ModelHost(a.binary.resolve(),plugin,state,root/'restart.log').ready();assert h.state()['encoder_routes']==[0,3]
            report={'ok':True,'imported_routers':True,'physical_rotation_coordinates':True,'deferred_short_press':True,'hold_switch_after_500ms':True,'single_assignment_direct':True,'unequal_repeated_clicks_balanced':True,'cancel_and_page_departure':True,'export_restart':True,'usb_access':False}
            a.report.write_text(json.dumps(report,indent=2)+'\n')
        finally:h.stop()
    print('PASS: Galleon physical routing, short/long presses, held rotation, independent columns, cancellation, queued click ownership, Routers export and restart; no USB.')
if __name__=='__main__':main()
