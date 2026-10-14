#!/usr/bin/env python3
"""Run an owned WebP background through the real host, editor, export and restart."""
import argparse,json,subprocess,tempfile,zipfile
from pathlib import Path
from PIL import Image
from native_editor_interop import cleanup,environment
SCRIPT=r'''
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const until=async(f,name)=>{for(let i=0;i<150;i++){const v=await f();if(v)return v;await pause(30)}throw Error('Timed out: '+name)};
const b=window.deckforge,cmd=p=>b.request('command',p),snapshot=()=>b.request('snapshot');
const initial=await snapshot();if(!initial.simulate||initial.device_model!=='galleon')throw Error('Requires simulated Galleon');
__SET__
async function observe(){const colors=new Set();await until(()=>{const image=document.querySelector('.deck-dial > img');if(!image?.complete||image.naturalWidth!==320)return false;const c=document.createElement('canvas');c.width=320;c.height=160;const ctx=c.getContext('2d');ctx.drawImage(image,0,0);const p=ctx.getImageData(10,10,1,1).data;if(p[0]>220&&p[2]<20)colors.add('red');if(p[2]>220&&p[0]<20)colors.add('blue');return colors.size===2},'both animated native colors');return [...colors].sort()}
const colors=await observe();
__PAGE__
return {colors,full_size_widgets:true,restart:__RESTART__,usb_access:false};
'''
def main():
    project=Path(__file__).resolve().parents[1];p=argparse.ArgumentParser();p.add_argument('--binary',type=Path,default=project/'target/release/deckforge');p.add_argument('--output',type=Path,default=project/'target/galleon/native-movie');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-native-movie-') as d:
        root=Path(d);source=root/'owned.webp';export=root/'export.streamDeckProfile'
        red=Image.new('RGB',(720,384),'red');blue=Image.new('RGB',(720,384),'blue');red.save(source,save_all=True,append_images=[blue],duration=[150,300],loop=0,lossless=True);original=source.read_bytes()
        results=[]
        for restart in [False,True]:
            script=root/'test.js';proof=root/'proof.json'
            setup='' if restart else "await cmd({command:'page_background',profile:initial.library.active_profile,id:initial.library.active_page,path:"+json.dumps(str(source))+"});"
            page='' if restart else "const page=(await snapshot()).library.active_page;await cmd({command:'export_profile',path:"+json.dumps(str(export))+"});await cmd({command:'page_create',name:'Still page'});await until(async()=>!(await snapshot()).encoder_background,'stop on page departure');await pause(400);if((await snapshot()).encoder_background)throw Error('Late background reappeared');await cmd({command:'page_select',id:page});await observe();"
            script.write_text(SCRIPT.replace('__SET__',setup).replace('__PAGE__',page).replace('__RESTART__',str(restart).lower()))
            env=environment(root);env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(script),DECKFORGE_WEBVIEW_TEST_RESULT=str(proof),DECKFORGE_CAPTURE=str(a.output.resolve()/('restart.png' if restart else 'movie.png')))
            with (a.output/('restart.log' if restart else 'native.log')).open('w') as log:
                process=subprocess.Popen([str(a.binary.resolve()),'desktop','--simulate','--model','galleon','--state',str(root/'state')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
                try:
                    code=process.wait(timeout=40);report=json.loads(proof.read_text()) if proof.exists() else {'ok':False,'error':'No native report'};assert code==0 and report.get('ok'),report;results.append(report)
                finally:cleanup(process)
            if not restart:
                with zipfile.ZipFile(export) as z:assert original in [z.read(n) for n in z.namelist() if n.endswith('.webp')]
                source.unlink()
        (a.output/'native.json').write_text(json.dumps({'runs':results,'source_webp_preserved':True,'restart_without_original_file':True,'usb_access':False},indent=2)+'\n')
    print('PASS: native animated WebP colors, page departure/return, byte-identical export and restart without original file; no USB.')
if __name__=='__main__':main()
