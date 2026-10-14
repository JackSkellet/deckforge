#!/usr/bin/env python3
"""Native Galleon simulator controls and active-row pixels with an owned SDK probe."""
import argparse,json,subprocess,tempfile
from PIL import Image
from pathlib import Path
from native_editor_interop import cleanup,environment
from models_interop import make_plugin,rows,PLUGIN,ACTION
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<250;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge,snapshot=()=>b.request('snapshot'),cmd=p=>b.request('command',p);
const initial=await snapshot();if(!initial.simulate||initial.device_model!=='galleon')throw Error('Requires simulated Galleon');
await until(async()=>(await snapshot()).plugins.every(p=>p.connected),'probe connected');
for(const index of [0,2,3])await cmd({command:'assign',controller:'Encoder',index,uuid:'com.deckforge.model-probe.action'});
const button=name=>document.querySelector(`button[aria-label="${name}"]`);
(await until(()=>{const e=button('Press left dial');return e&&!e.disabled&&e},'enabled physical control')).click();
await until(async()=>(await snapshot()).logs.filter(line=>line.includes('DialPress')).length>=2,'atomic down/up');
button('Switch left dial row').click();
await until(async()=>JSON.stringify((await snapshot()).encoder_routes)==='[2,3]','left route switched');
function marked(index){const image=document.querySelector('.encoder-route-markers');if(!image?.complete||image.naturalWidth!==720)return false;const canvas=document.createElement('canvas');canvas.width=720;canvas.height=384;const ctx=canvas.getContext('2d');ctx.drawImage(image,0,0);const p=ctx.getImageData(16,index===0?100:270,1,1).data;return p[3]>200;}
await until(()=>marked(2)&&!marked(0),'active-row border moved');
// Holding previews the other row without changing the routed action.
await cmd({command:'input',physical:true,event:'dialDown',index:0});
await until(()=>marked(0)&&!marked(2),'pending row preview');
if(JSON.stringify((await snapshot()).encoder_routes)!=='[2,3]')throw Error('Preview changed routing');
await cmd({command:'physical_dial_cancel',index:0});
await until(()=>marked(2)&&!marked(0),'cancel restores selected border');
button('Turn right dial right').click();
await until(async()=>(await snapshot()).logs.some(line=>line.includes('DialRotate { index: 3')),'right physical control maps lower row');
const image=document.querySelector('.encoder-route-markers'), rect=image.getBoundingClientRect();
return {viewport:[innerWidth,innerHeight],marker_rect:{x:rect.x,y:rect.y,width:rect.width,height:rect.height},native_controls:true,atomic_click:true,active_routes:(await snapshot()).encoder_routes,active_row_pixels:true,usb_access:false};
'''
def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/galleon/native-routes');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-native-routes-') as d:
        root=Path(d);plugin=root/(PLUGIN+'.sdPlugin');events=make_plugin(plugin,project);script=root/'test.js';proof=root/'proof.json';script.write_text(SCRIPT);env=environment(root)
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/'routes.png'))
        with (a.output/'native.log').open('w') as log:
            process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','galleon','--state',str(root/'state'),'--plugin',str(plugin)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                code=process.wait(timeout=40);report=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No native result'}
                assert code==0 and report.get('ok'),report
                inputs=[e for e in rows(events) if e['event'] in ['dialDown','dialUp','dialRotate']]
                assert [e['event'] for e in inputs]==['dialDown','dialUp','dialRotate'],inputs
                assert inputs[0]['payload']['coordinates']=={'column':0,'row':0}
                assert inputs[2]['payload']['coordinates']=={'column':1,'row':1}
                capture=Image.open(a.output/'routes.png').convert('RGB');r=report['result']['marker_rect'];scale=capture.width/report['result']['viewport'][0]
                left=round((r['x']+r['width']*20/720)*scale);right=round((r['x']+r['width']*348/720)*scale);top=round((r['y']+r['height']*192/384)*scale)
                white=max(sum(min(capture.getpixel((x,y)))>140 for x in range(left,right)) for y in range(max(0,top-2),min(capture.height,top+3)))
                assert white>=(right-left)*.6,{'visible_white_pixels':white,'image_width':right-left,'rect':r}
                report['visible_border_verified']=True
                report['sdk_events_verified']=True;(a.output/'native.json').write_text(json.dumps(report,indent=2)+'\n')
            finally:cleanup(process)
    print('PASS: native Galleon press/rotate/row controls, balanced SDK events and active-row border and cancelled hold preview; no USB.')
if __name__=='__main__':main()
