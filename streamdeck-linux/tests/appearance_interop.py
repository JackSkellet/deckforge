#!/usr/bin/env python3
"""Real host appearance, SDK precedence, state isolation and portable export.
Only generated local fixtures and simulated device input are used.
"""
import argparse
import base64
import io
import json
from pathlib import Path
import tempfile
import time
import uuid
import zipfile
from PIL import Image
from system_interop import Host

PLUGIN = 'com.deckforge.appearance-probe'
ACTION = PLUGIN+'.key'


def at(host):
    return next(s for s in host.state()['slots'] if s['controller'] == 'Keypad' and s['index'] == 0)


def rows(path):
    if not path.exists(): return []
    return [json.loads(line) for line in path.read_text().splitlines(keepends=True) if line.endswith('\n')]


def picture(host):
    return Image.open(io.BytesIO(base64.b64decode(at(host)['preview']))).convert('RGB')


def green_y(host):
    image = picture(host)
    return [y for y in range(image.height) for x in range(image.width)
            if (lambda c:c[1]>100 and c[1]>c[0]+40 and c[1]>c[2]+40)(image.getpixel((x,y)))]


def main():
    project = Path(__file__).resolve().parents[1]
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary',type=Path,default=project/'target/release/deckforge')
    ap.add_argument('--report',type=Path)
    args=ap.parse_args(); binary=args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-appearance-') as directory:
        root=Path(directory); plugin=root/(PLUGIN+'.sdPlugin');plugin.mkdir()
        proof=plugin/'events.jsonl'
        sdk=project.parent/'evidence/official-samples/counter/package.json'
        code=r'''import {createRequire} from 'node:module';import {appendFileSync} from 'node:fs';
const WebSocket=createRequire(__SDK__)('ws'),proof=__PROOF__,seen=new Set(),args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',bytes=>{const m=JSON.parse(bytes);appendFileSync(proof,JSON.stringify(m)+'\n');
 const r=m.payload?.settings?.probe;if(m.event!=='didReceiveSettings'||!r||seen.has(r.id))return;seen.add(r.id);
 ws.send(JSON.stringify({event:r.event,context:m.context,payload:r.payload}));
 ws.send(JSON.stringify({event:'getSettings',context:m.context,id:r.id}));});'''
        (plugin/'plugin.mjs').write_text(code.replace('__SDK__',json.dumps(str(sdk))).replace('__PROOF__',json.dumps(str(proof))))
        states=[{'Image':'baseline.svg','Title':'Baseline '+str(i),'FontFamily':'DejaVu Sans','FontSize':12,
                 'TitleColor':'#ffffff','TitleAlignment':'middle','FutureState':i} for i in range(2)]
        (plugin/'baseline.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120"><rect width="120" height="120" fill="#102030"/></svg>')
        alternate=b'<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120"><rect width="120" height="120" fill="#0405aa"/></svg>'
        (plugin/'alternate.svg').write_bytes(alternate)
        (plugin/'manifest.json').write_text(json.dumps({'UUID':PLUGIN,'Name':'Appearance probe','CodePath':'plugin.mjs','Nodejs':{'Version':'20'},'SDKVersion':2,
            'Actions':[{'UUID':ACTION,'Name':'Styled key','Controllers':['Keypad'],'States':states,'DisableAutomaticStates':True}]}))
        icon=root/'custom.svg';original=b'<svg xmlns="http://www.w3.org/2000/svg" width="180" height="180"><rect width="180" height="180" fill="#c8140a"/></svg>';icon.write_bytes(original)
        state=root/'state';host=Host(binary,[plugin],state,root/'host.log').ready()
        try:
            host.wait(lambda s: s['plugins'][0]['connected'],'probe registration')
            host.post({'command':'assign','controller':'Keypad','index':0,'uuid':ACTION})
            context=at(host)['context']
            def edit(**fields):
                host.post({'command':'appearance','context':context,'expected_state':at(host)['state'],**fields})
            def sdk_update(event,payload):
                marker=str(uuid.uuid4()); settings=dict(at(host)['settings']);settings['probe']={'id':marker,'event':event,'payload':payload}
                host.post({'command':'settings','context':context,'settings':settings})
                host.wait(lambda s:any(e.get('id')==marker and e.get('event')=='didReceiveSettings' for e in rows(proof)),'SDK command processed')
            sdk_update('setTitle',{'title':'Plugin title'})
            assert at(host)['title']=='Plugin title'
            edit(title='Ag',title_parameters={'fontSize':16,'titleColor':'#00ff00','titleAlignment':'top'})
            style=at(host)['appearance']['title_parameters']
            assert style['fontFamily']=='DejaVu Sans' and style['fontSize']==16
            assert at(host)['appearance']['title_custom']
            top=green_y(host);assert top and sum(top)/len(top)<40
            edit(title_parameters={'titleAlignment':'bottom','fontUnderline':True})
            bottom=green_y(host);assert bottom and sum(bottom)/len(bottom)>70
            assert at(host)['appearance']['title_parameters']['titleColor']=='#00ff00'
            changes=[r for r in rows(proof) if r.get('event')=='titleParametersDidChange']
            host.wait(lambda s:len([r for r in rows(proof) if r.get('event')=='titleParametersDidChange'])>=2,'title parameter events')
            payload=[r['payload'] for r in rows(proof) if r.get('event')=='titleParametersDidChange'][-1]
            assert payload['coordinates']=={'column':0,'row':0} and 'settings' in payload and 'resources' in payload
            assert 'isInMultiAction' not in payload and payload['state']==0
            sdk_update('setTitle',{'title':'Latest plugin title'})
            assert at(host)['title']=='Ag'
            edit(image_path=str(icon),title_parameters={'showTitle':False})
            sdk_update('setImage',{'image':'baseline.svg'})
            image=picture(host)
            assert image.tobytes()==bytes([200,20,10])*image.width*image.height,'User icon or hidden title lost precedence'
            sdk_update('setTitle',{'title':'Inactive SDK','state':1})
            sdk_update('setImage',{'image':'alternate.svg','state':1})
            assert at(host)['state']==0 and at(host)['title']=='Ag'
            sdk_update('setState',{'state':1})
            assert at(host)['title']=='Inactive SDK' and picture(host).getpixel((0,0))==(4,5,170)
            sdk_update('setState',{'state':0})
            # Editing another state must not flip the active hardware state.
            edit(state=1,title='Second',title_parameters={'titleColor':'#ff8800','fontStyle':'Bold Italic'})
            assert at(host)['state']==0 and at(host)['title']=='Ag'
            host.post({'command':'appearance','context':context,'expected_state':1,'title_parameters':{'showTitle':True}},expected=400)
            sdk_update('setState',{'state':1})
            assert at(host)['title']=='Second' and at(host)['appearance']['title_parameters']['fontStyle']=='Bold Italic'
            sdk_update('setState',{'state':0})
            assert at(host)['title']=='Ag' and not at(host)['appearance']['title_parameters']['showTitle']
            exported=root/'styled.streamDeckProfile';host.post({'command':'export_profile','path':str(exported)})
            with zipfile.ZipFile(exported) as archive:
                data=[archive.read(name) for name in archive.namelist()]
                assert original in data
                assert alternate not in data, 'Transient SDK image became a user override in the exported profile'
                assert all(b'"appearances"' not in archive.read(name) for name in archive.namelist() if name.endswith('manifest.json'))
            # Both suppressed SDK updates remain available for an explicit reset.
            edit(reset_title=True,reset_image=True,title_parameters={'showTitle':True})
            assert at(host)['title']=='Latest plugin title'
            assert not at(host)['appearance']['title_custom'] and not at(host)['appearance']['image_custom']
            assert picture(host).getpixel((0,0))==(16,32,48)
            host.post({'command':'import_profile','path':str(exported)})
            context=at(host)['context']
            assert at(host)['title']=='Ag' and at(host)['appearance']['image_custom']
            sdk_update('setImage',{'image':'baseline.svg'})
            sdk_update('setTitle',{'title':'Imported SDK update'})
            assert at(host)['title']=='Ag' and picture(host).getpixel((0,0))==(200,20,10)
            host.stop();host=Host(binary,[],state,root/'restart.log').ready()
            assert at(host)['title']=='Ag' and at(host)['appearance']['image_custom']
            assert at(host)['appearance']['title_parameters']['fontUnderline']
            report={'passed':True,'sparse_style_and_pixels':True,'sdk_event_payload':True,'user_override_and_reset':True,
                    'inactive_state_isolation':True,'portable_svg_and_styles':True,'restart':True,
                    'boundary':'Real Rust host and local WebSocket probe; no USB, OS input, external network or user state.'}
            if args.report:args.report.write_text(json.dumps(report,indent=2)+'\n')
        finally:host.stop()
    print('PASS: styled pixels, title events, user/SDK precedence and reset, inactive-state edits, SVG export/reimport and restart; simulation only.')


if __name__=='__main__':main()
