#!/usr/bin/env python3
"""Render untouched Plus Encoder overrides from the supplied installer in WebKit."""
import argparse,base64,json,subprocess,tempfile,zipfile
from pathlib import Path
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));const until=async(f,name)=>{for(let i=0;i<150;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge,cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot');if(!(await snapshot()).simulate)throw Error('Requires simulation');
__IMPORT__
let s=await snapshot();const profile=s.library.profiles.find(p=>p.id===s.library.active_profile);
for(const page of profile.pages){await cmd({command:'page_select',id:page.id});s=await snapshot();if(s.slots.some(s=>s.controller==='Encoder'&&s.raw.Encoder?.background))break;}
const tip=s.slots.find(s=>s.controller==='Encoder'&&s.raw.Encoder?.background);if(!tip||tip.raw.Encoder.IconVisible!==false)throw Error('Original hidden-icon background missing');
const expected=new Image(),actual=new Image();expected.src=__EXPECTED__;actual.src='data:image/png;base64,'+tip.preview;await until(()=>expected.complete&&actual.complete,'PNG decoded');
const pageBackground=new Image();if(s.encoder_background){pageBackground.src='data:image/png;base64,'+s.encoder_background;await until(()=>pageBackground.complete,'page background decoded');}
function pixels(image,compose){const c=document.createElement('canvas');c.width=200;c.height=100;const x=c.getContext('2d');if(compose){x.fillStyle='black';x.fillRect(0,0,200,100);if(s.encoder_background)x.drawImage(pageBackground,tip.index*200,0,200,100,0,0,200,100);}x.drawImage(image,0,0);return x.getImageData(0,0,200,100).data;}
const a=pixels(actual,false),e=pixels(expected,true);let delta=0;for(let i=0;i<a.length;i++)delta=Math.max(delta,Math.abs(a[i]-e[i]));if(delta>1)throw Error('Original tip composition changed: '+delta);
const icon=s.slots.find(s=>s.controller==='Encoder'&&s.raw.Encoder?.Icon);if(!icon?.software_icon)throw Error('Original custom dial icon missing');
const control=[...document.querySelectorAll('.deck-dial')][tip.index];await until(()=>control.querySelector('img')?.src===actual.src,'original tip in DOM');control.click();
await until(()=>control.getAttribute('aria-pressed')==='true','select original tip');
__EXPORT__
return {original_background_max_channel_delta:delta,original_icon_visible:true,hidden_icon_respected:true,usb_access:false};
'''
def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/encoder-assets/native');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    source=project.parent/'evidence/elgato-7.6/raw/flsysF8Fq6CoTq162E1zc6fyeIlKcw'
    with zipfile.ZipFile(source) as z:tip=z.read(next(n for n in z.namelist() if n.endswith('35BC43FCJ120WB9GK09RWQT5LOZ.png')))
    reports=[]
    with tempfile.TemporaryDirectory(prefix='deckforge-encoder-assets-') as d:
        root=Path(d);script=root/'test.js';proof=root/'proof.json';export=root/'roundtrip.streamDeckProfile'
        for restart in [False,True]:
            script.write_text(SCRIPT.replace('__IMPORT__','' if restart else "await cmd({command:'import_profile',path:"+json.dumps(str(source))+"});").replace('__EXPECTED__',json.dumps('data:image/png;base64,'+base64.b64encode(tip).decode())).replace('__EXPORT__','' if restart else "await cmd({command:'export_profile',path:"+json.dumps(str(export))+"});"))
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/('restart.png' if restart else 'native.png')))
            with (a.output/('restart.log' if restart else 'native.log')).open('w') as log:
                process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','plus','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No native result'};assert code==0 and report.get('ok'),report;reports.append(report)
                finally:cleanup(process)
        with zipfile.ZipFile(export) as z:
            assert tip in [z.read(n) for n in z.namelist() if n.endswith('.png')]
            raw=[json.loads(z.read(n)) for n in z.namelist() if n.endswith('manifest.json')]
            actions=[v for r in raw for c in r.get('Controllers',[]) for v in c.get('Actions',{}).values()]
            assert any(v.get('Encoder',{}).get('IconVisible') is False and v['Encoder'].get('background') for v in actions)
    (a.output/'native.json').write_text(json.dumps({'runs':reports,'original_png_and_metadata_preserved':True},indent=2)+'\n');print('PASS: untouched original Plus dial background pixels, custom icon, hidden feedback icon, export and restart; no USB.')
if __name__=='__main__':main()
