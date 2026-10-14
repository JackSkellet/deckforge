#!/usr/bin/env python3
"""Verify real WebKit backgrounds using an unchanged image from the supplied profile."""
import argparse,json,subprocess,tempfile,zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<200;i++){const v=await f();if(v)return v;await pause(40)}throw Error('Timed out: '+name)};
const b=window.deckforge,cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot');
if(!(await snapshot()).simulate)throw Error('Requires simulation');
await cmd({command:'import_profile',path:__SOURCE__});
await until(async()=>(await snapshot()).encoder_background,'background decoded');
const initial=await snapshot(),page=initial.library.active_page;
function pixel(image,x,y){const c=document.createElement('canvas');c.width=image.naturalWidth;c.height=image.naturalHeight;const ctx=c.getContext('2d');ctx.drawImage(image,0,0);return [...ctx.getImageData(x,y,1,1).data].slice(0,3)};
const base=new Image();base.src='data:image/png;base64,'+initial.encoder_background;await until(()=>base.complete,'base loaded');
const images=()=>[...document.querySelectorAll('.deck-dial > img')];
await until(()=>images().length===4&&images().every(i=>i.complete),'assigned and empty crops');
if(images()[0].naturalWidth!==320)throw Error('Wrong assigned widget size');
const expected=pixel(base,24,24),actual=pixel(images()[0],0,0);
if(actual.some((v,i)=>Math.abs(v-expected[i])>3))throw Error('Background hidden behind assigned feedback: '+JSON.stringify({actual,expected}));
const lowerExpected=pixel(base,24,200),lower=pixel(images()[2],0,0);
if(lower.some((v,i)=>Math.abs(v-lowerExpected[i])>3))throw Error('Wrong lower crop');
await cmd({command:'page_create',name:'No background'});
await until(async()=>!(await snapshot()).encoder_background,'clear on page change');
await cmd({command:'page_select',id:page});
await until(()=>document.querySelector('.dial-grid').dataset.background==='true'&&images().length===4,'restored on return');
const change={command:'page_background',profile:initial.library.active_profile,id:page,path:__CHOSEN__};
await cmd(change);
await until(async()=>{const s=await snapshot();return s.encoder_background_configured&&s.encoder_background!==initial.encoder_background},'chosen image saved');
document.querySelector('summary[aria-label="Page options"]').click();
const remove=()=>[...document.querySelectorAll('.page-menu button')].find(b=>b.textContent==='Remove display background');
(await until(()=>{const b=remove();return b&&!b.disabled&&b},'remove background enabled')).click();
await until(async()=>!(await snapshot()).encoder_background_configured,'native remove button');
await cmd(change);
await until(async()=>(await snapshot()).encoder_background_configured,'chosen background restored');
const chosen=new Image();chosen.src='data:image/png;base64,'+(await snapshot()).encoder_background;await until(()=>chosen.complete,'chosen pixels');
if(JSON.stringify(pixel(chosen,0,0))!=='[150,20,80]')throw Error('Chosen image did not reach display');
await cmd({command:'export_profile',path:__EXPORT__});
const rect=document.querySelector('.dial-grid').getBoundingClientRect();
return {background:true,selection_and_native_removal:true,assigned_and_empty_crops:true,page_change:true,grid:{x:rect.x,y:rect.y,width:rect.width,height:rect.height},viewport:[innerWidth,innerHeight],usb_access:false};
'''
def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/galleon/native-background');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(project.parent/'evidence/elgato-7.6/raw/flsKaSPXYxUUvmKtC3TdqgYiqQcKKY') as z:
        image=z.read(next(n for n in z.namelist() if n.endswith('089AGGSPQT3QDD7DJOBRFFKKQGZ.png')))
    with tempfile.TemporaryDirectory(prefix='deckforge-background-') as d:
        root=Path(d);source=root/'background.streamDeckProfile';export=root/'roundtrip.streamDeckProfile';chosen=root/'chosen.png';Image.new('RGB',(720,384),(150,20,80)).save(chosen);chosen_bytes=chosen.read_bytes()
        with zipfile.ZipFile(source,'w') as z:
            z.writestr('manifest.json',json.dumps({'Name':'Original background','Device':{'Model':'GRETSCH'},'Controllers':[{'Type':'Encoder','Background':'Images/background.png','Actions':{'0,0':{'UUID':'com.elgato.streamdeck.system.hotkey','Settings':{},'States':[{}]}}}]}));z.writestr('Images/background.png',image)
        script=root/'test.js';proof=root/'proof.json';script.write_text(SCRIPT.replace('__SOURCE__',json.dumps(str(source))).replace('__EXPORT__',json.dumps(str(export))).replace('__CHOSEN__',json.dumps(str(chosen))))
        env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/'background.png'))
        with (a.output/'native.log').open('w') as log:
            process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','galleon','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                code=process.wait(timeout=40);report=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No native result'};assert code==0 and report.get('ok'),report
                with zipfile.ZipFile(export) as z:
                    exported=[z.read(n) for n in z.namelist() if n.endswith('.png')];assert image in exported and chosen_bytes in exported
                capture=Image.open(a.output/'background.png').convert('RGB');r=report['result']['grid'];scale=capture.width/report['result']['viewport'][0]
                # Original background is purple in the bottom gutter; the old blank canvas is gray.
                x=round((r['x']+r['width']/2)*scale);y=round((r['y']+r['height']-2)*scale);rgb=capture.getpixel((x,y))
                assert rgb[2]>rgb[1]+20,{'gutter_pixel':rgb}
                report['visible_background_verified']=True;report['original_png_export_preserved']=True;(a.output/'native.json').write_text(json.dumps(report,indent=2)+'\n')
            finally:cleanup(process)
        source.unlink();chosen.unlink()
        script.write_text("const s=await window.deckforge.request('snapshot');if(!s.encoder_background||s.encoder_background_tiles?.length!==4)throw Error('Background lost on restart');return {restart:true};")
        env['DECKFORGE_CAPTURE']=str(a.output.resolve()/'restart.png')
        with (a.output/'restart.log').open('w') as log:
            process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            try:
                assert process.wait(timeout=40)==0
                restarted=json.loads(proof.read_text());assert restarted.get('ok') and restarted['result']['restart'],restarted
                report['restart_without_original_source']=True;(a.output/'native.json').write_text(json.dumps(report,indent=2)+'\n')
            finally:cleanup(process)
    print('PASS: native background setting/removal, original-image crops, restart and byte-identical export; no USB.')
if __name__=='__main__':main()
