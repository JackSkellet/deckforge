#!/usr/bin/env python3
"""Check cropper layout at desktop and minimum native-window sizes in Chromium.

The image service is synthetic; native_cropper_interop.py verifies the real host.
"""
import base64,io,json,re,subprocess,tempfile
from pathlib import Path
from PIL import Image
project=Path(__file__).resolve().parents[1];output=project/'target/cropper/layout';output.mkdir(parents=True,exist_ok=True)
image=Image.new('RGB',(400,100),'red');image.paste('blue',(200,0,400,100));data=io.BytesIO();image.save(data,format='PNG');png=base64.b64encode(data.getvalue()).decode()
with tempfile.TemporaryDirectory(prefix='.crop-layout-',dir=project/'frontend') as d:
 root=Path(d);entry=root/'entry.tsx';script=output/'layout.js';html=output/'layout.html'
 entry.write_text('''import {useState} from 'react';import {createRoot} from 'react-dom/client';import {ImageCropper} from '../src/ImageCropper';
window.deckforge={request:async()=>({width:400,height:100,frames:2,frame:0,source_hash:'fixture',preview:__PNG__}),onSnapshot:()=>()=>{}};
function Page(){const[edit,setEdit]=useState({device:'owned',context:'crop',path:'/fixture.gif',frame:0,expected:null});return <ImageCropper edit={edit} onChange={setEdit} onClose={()=>{}} onSaved={()=>{}}/>}createRoot(document.getElementById('root')).render(<Page/>);
setTimeout(()=>{const dialog=document.querySelector('dialog').getBoundingClientRect();const save=[...document.querySelectorAll('button')].find(b=>b.textContent==='Set Image').getBoundingClientRect();document.getElementById('proof').textContent=JSON.stringify({viewport:[innerWidth,innerHeight],dialog:{left:dialog.left,right:dialog.right,top:dialog.top,bottom:dialog.bottom},save:{left:save.left,right:save.right,top:save.top,bottom:save.bottom}})},1000);
'''.replace('__PNG__',json.dumps(png)))
 subprocess.run([str(project/'frontend/node_modules/.bin/esbuild'),str(entry),'--bundle','--jsx=automatic','--platform=browser','--outfile='+str(script)],check=True,capture_output=True)
 html.write_text('<!doctype html><html><head><link rel="stylesheet" href="'+(project/'frontend/src/style.css').as_uri()+'"></head><body><div id="root"></div><pre id="proof" hidden></pre><script src="'+script.as_uri()+'"></script></body></html>')
 reports=[]
 for width,height in [(1180,850),(760,620)]:
  capture=output/f'{width}x{height}.png'
  with tempfile.TemporaryDirectory(prefix='deckforge-crop-chrome-') as profile:
   call=subprocess.run(['chromium','--headless','--disable-gpu','--disable-background-networking','--no-first-run','--user-data-dir='+profile,f'--window-size={width},{height}','--virtual-time-budget=3000','--screenshot='+str(capture),'--dump-dom',html.as_uri()],capture_output=True,text=True,timeout=25)
   assert call.returncode==0,call.stderr[-1000:]
   match=re.search(r'<pre id="proof" hidden="">(.*?)</pre>',call.stdout);assert match,call.stdout[-1500:];report=json.loads(match.group(1))
   for name in ['dialog','save']:
    box=report[name];assert box['left']>=0 and box['right']<=report['viewport'][0]+1 and box['top']>=0 and box['bottom']<=report['viewport'][1]+1,report
   reports.append(report)
 (output/'layout.json').write_text(json.dumps(reports,indent=2)+'\n')
print('PASS: cropper and Save remain within desktop and minimum-size Chromium viewports.')
