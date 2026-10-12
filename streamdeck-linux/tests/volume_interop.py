#!/usr/bin/env python3
"""System Volume dial + live display against an owned virtual sink. No USB or real-output writes."""
import argparse
import base64
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid
from PIL import Image,ImageChops
from system_interop import Host
from sample_interop import require,slot
MEDIA='com.elgato.streamdeck.system.multimedia'

def pactl(*args):return subprocess.check_output(['pactl',*args],text=True,timeout=5).strip()
def sink_info(name):return next(s for s in json.loads(pactl('--format=json','list','sinks')) if s['name']==name)
def preview(state):return Image.open(io.BytesIO(base64.b64decode(slot(state,MEDIA)['preview']))).convert('RGB')
def close(actual,expected):return abs(actual-expected)<.02

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));parser.add_argument('--evidence',type=Path);args=parser.parse_args()
    original_default=pactl('get-default-sink');name='deckforge_volume_'+uuid.uuid4().hex
    module=pactl('load-module','module-null-sink','sink_name='+name,'sink_properties=device.description=Deckforge_volume_test')
    previous=os.environ.get('DECKFORGE_VOLUME_TEST_SINK');os.environ['DECKFORGE_VOLUME_TEST_SINK']=name
    try:
        if pactl('get-default-sink')!=original_default:pactl('set-default-sink',original_default)
        pactl('set-sink-volume',name,'40%','20%');pactl('set-sink-mute',name,'0')
        with tempfile.TemporaryDirectory(prefix='deckforge-volume-') as directory:
            root=Path(directory);host=Host(args.binary.resolve(),[],root/'state',root/'host.log')
            try:
                host.ready();host.post({'command':'assign','controller':'Encoder','index':0,'uuid':MEDIA})
                ctx=slot(host.state(),MEDIA)['context']
                initial=host.wait(lambda s:s['volume'] and close(s['volume']['percent'],40) and slot(s,MEDIA)['feedback'].get('value')=='40%','initial live percentage')
                require(initial['volume']['sink']==name,'Worker used another output')
                before=preview(initial)
                host.post({'command':'settings','context':ctx,'settings':{'actionIdx':18,'linuxVolumeStep':2.5}})
                host.post({'command':'input','event':'dialRotate','index':0,'ticks':2})
                changed=host.wait(lambda s:s['volume'] and close(s['volume']['percent'],45) and slot(s,MEDIA)['feedback']['value']=='45%','two configured 2.5% ticks')
                require(ImageChops.difference(before,preview(changed)).getbbox() is not None,'Percentage update did not reach rendered pixels')
                actual=sink_info(name);channels=list(actual['volume'].values())
                require(close(channels[0]['value']/65536*100,45) and close(channels[1]['value']/65536*100,22.5),'Channel balance changed')
                host.post({'command':'input','event':'dialRotate','index':0,'ticks':-1})
                host.wait(lambda s:s['volume'] and close(s['volume']['percent'],42.5) and slot(s,MEDIA)['feedback']['value']=='42.5%','fractional percentage')
                # External changes must refresh without dial input.
                pactl('set-sink-volume',name,'73%');pactl('set-sink-mute',name,'1')
                external=host.wait(lambda s:s['volume'] and close(s['volume']['percent'],73) and s['volume']['muted'] and slot(s,MEDIA)['feedback']['value']=='73%','external volume and mute update')
                require(slot(external,MEDIA)['feedback']['title']=='Volume (muted)','Mute state not visible')
                for event in ['dialDown','dialUp']:host.post({'command':'input','event':event,'index':0})
                host.wait(lambda s:s['volume'] and not s['volume']['muted'],'dial press toggled mute')
                host.post({'command':'settings','context':ctx,'settings':{'actionIdx':18,'linuxVolumeStep':.5}})
                host.post({'command':'input','event':'dialRotate','index':0,'ticks':1})
                host.wait(lambda s:s['volume'] and close(s['volume']['percent'],73.5),'edited step applied immediately')
                # Invalid step is rejected before any volume mutation.
                host.post({'command':'settings','context':ctx,'settings':{'actionIdx':18,'linuxVolumeStep':0}})
                host.post({'command':'input','event':'dialRotate','index':0,'ticks':1},expected=400)
                require(close(sink_info(name)['volume']['front-left']['value']/65536*100,73.5),'Rejected step changed volume')
                host.post({'command':'settings','context':ctx,'settings':{'actionIdx':18,'linuxVolumeStep':.5}})
                if args.evidence:
                    args.evidence.mkdir(parents=True,exist_ok=True);preview(host.state()).save(args.evidence/'virtual-volume.png')
                    (args.evidence/'metrics.json').write_text(json.dumps({'initial_percent':40,'step':2.5,'two_ticks_percent':changed['volume']['percent'],'balanced_channel_percent':[v['value']/65536*100 for v in channels],'external_percent':external['volume']['percent'],'final_percent':host.state()['volume']['percent'],'default_unchanged':pactl('get-default-sink')==original_default},indent=2)+'\n')
                # Missing target loses feedback instead of falling back to a real output.
                pactl('unload-module',module);module=None
                host.wait(lambda s:s['volume'] is None and s['volume_error'] and slot(s,MEDIA)['feedback']['value']=='--','removed target error')
                module=pactl('load-module','module-null-sink','sink_name='+name,'sink_properties=device.description=Deckforge_volume_test')
                pactl('set-sink-volume',name,'61%')
                host.wait(lambda s:s['volume'] and close(s['volume']['percent'],61) and s['volume_error'] is None and not s['system_status'].startswith('System volume unavailable'),'automatic target recovery')

                if args.evidence:(args.evidence/'host.log').write_text((root/'host.log').read_text())
            finally:host.stop();require(host.process.returncode==0,'Volume worker prevented clean shutdown')
            restart=Host(args.binary.resolve(),[],root/'state',root/'restart.log')
            try:
                restart.ready();restart.wait(lambda s:s['volume'] and close(s['volume']['percent'],61),'fresh volume on restart')
                require(slot(restart.state(),MEDIA)['settings']['linuxVolumeStep']==.5,'Volume step was not saved')
            finally:restart.stop()
        require(pactl('get-default-sink')==original_default,'Desktop default output changed')
        print('PASS: actual Pulse/PipeWire percentage and mute monitoring, configured/fractional tick steps, channel balance, external updates, rendered PNGs, errors and restart. Writes used only the owned virtual sink; no USB or physical output touched.')
    finally:
        if module is not None:pactl('unload-module',module)
        if previous is None:os.environ.pop('DECKFORGE_VOLUME_TEST_SINK',None)
        else:os.environ['DECKFORGE_VOLUME_TEST_SINK']=previous
if __name__=='__main__':main()
