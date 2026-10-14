#!/usr/bin/env python3
"""Real host import/export/restart defaults; no actions or keyboard permissions requested."""
import argparse,hashlib,json,tempfile,uuid,zipfile
from pathlib import Path
from system_interop import Host

TEXT='com.elgato.streamdeck.system.text';MULTI='com.elgato.streamdeck.multiactions.routine'
def fixture(path,os_type=None,shared=False):
    profile,main,pins,folder=[str(uuid.uuid4()) for _ in range(4)]
    base=('Profiles/' if os_type is not None or shared else '')+profile+'.sdProfile/'
    def action(name,settings,kind=TEXT):return {'UUID':kind,'ActionID':str(uuid.uuid4()),'Name':name,'Settings':settings,'States':[{'Title':name}],'Future':{'keep':9}}
    entries={'0,0':action('Legacy',{'pastedText':'native legacy 🌍','FutureSetting':7})}
    if not shared:
        entries.update({'1,0':action('Explicit paste',{'pastedText':'explicit','isTypingMode':False}),
            '2,0':{**action('Sequence',{},MULTI),'Actions':[{'Actions':[action('Nested',{'pastedText':'nested'})]}]},
            '3,0':action('Folder',{'ProfileUUID':folder},'com.elgato.streamdeck.profile.openchild'),
            '2,1':{**action('Opaque',{},'unknown.container'),'Actions':[action('Opaque payload',{'pastedText':'opaque'})]},
            '3,1':action('Invalid',{'pastedText':'invalid mode','isTypingMode':None})})
    files={base+'manifest.json':{'Name':'Text defaults','Version':'2.0','Device':{'Model':'Fake/Storage' if shared else '20GBD9901'},'Pages':{'Current':main,'Default':pins,'Pages':[main]},'FutureRoot':5},
        base+f'Profiles/{main}/manifest.json':{'Controllers':[{'Type':'Keypad','Actions':entries}],'FuturePage':6},
        base+f'Profiles/{pins}/manifest.json':{'Controllers':[]},
        base+f'Profiles/{folder}/manifest.json':{'Controllers':[{'Type':'Keypad','Actions':{'1,0':action('Hidden',{'pastedText':'hidden'})}}]},
        base+'retained.bin':b'preserve original asset\0bytes'}
    if os_type is not None or shared:
        files['package.json']={'FormatVersion':1,'RequiredPlugins':[],**({'OSType':os_type} if os_type is not None else {})}
    with zipfile.ZipFile(path,'w',compression=zipfile.ZIP_DEFLATED) as z:
        for name,value in files.items():z.writestr(name,value if isinstance(value,bytes) else json.dumps(value))

def exported_actions(path):
    def walk(value):
        if isinstance(value,dict):
            if 'UUID' in value and 'Name' in value:yield value
            for child in value.values():yield from walk(child)
        elif isinstance(value,list):
            for child in value:yield from walk(child)
    with zipfile.ZipFile(path) as z:
        return {a['Name']:a for name in z.namelist() if name.endswith('manifest.json') for a in walk(json.loads(z.read(name)))}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,default=Path('target/release/deckforge'));p.add_argument('--output',type=Path,default=Path('target/text-defaults'));a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True);binary=a.binary.resolve();host=None
    with tempfile.TemporaryDirectory(prefix='deckforge-text-defaults-') as d:
        root=Path(d);state=root/'state';logs=[]
        def start(name):
            log=root/name;logs.append(log);return Host(binary,[],state,log).ready()
        def key(index=0):return next(s for s in host.state()['slots'] if s['controller']=='Keypad' and s['index']==index)
        try:
            host=start('seed.log');host.post({'command':'assign','controller':'Keypad','index':0,'uuid':TEXT})
            legacy={'pastedText':'Existing Linux paste','isSendingEnter':False,'Future':{'keep':9}}
            host.post({'command':'settings','context':key()['context'],'settings':legacy})
            old_profile=host.state()['library']['active_profile'];host.stop();host=None
            host=start('run.log');assert key()['settings']==legacy
            before=(state/'session.json').read_bytes();output=root/'old-export.streamDeckProfile'
            host.post({'command':'export_profile','path':str(output)})
            assert (state/'session.json').read_bytes()==before and key()['settings']==legacy
            assert exported_actions(output)['Text']['Settings']['isTypingMode'] is False
            host.post({'command':'import_profile','path':str(output)});assert key()['settings']['isTypingMode'] is False
            imported=[]
            for os_type in [None,'Windows','macOS','Linux']:
                source=root/f'{os_type or "plain"}.streamDeckProfile';fixture(source,os_type);original=source.read_bytes()
                previous_profile=host.state()['library']['active_profile']
                host.post({'command':'import_profile','path':str(source)})
                assert host.state()['library']['active_profile']!=previous_profile,'Compatible imported profile was not activated'
                expected=os_type!='Linux';assert key()['settings']=={'pastedText':'native legacy 🌍','FutureSetting':7,'isTypingMode':expected},(os_type,key()['settings'])
                assert key(1)['settings']['isTypingMode'] is False
                assert key(2)['multi_children'][0][0]['settings']['isTypingMode']==expected
                assert 'isTypingMode' not in key(6)['raw']['Actions'][0]['Settings']
                assert key(7)['settings']['isTypingMode'] is None
                assert host.state()['typing_status']['phase']=='disabled'
                exported=root/f'{os_type or "plain"}-export.streamDeckProfile';host.post({'command':'export_profile','path':str(exported)})
                records=exported_actions(exported)
                for name in ['Legacy','Nested','Hidden']:assert records[name]['Settings']['isTypingMode']==expected
                assert 'isTypingMode' not in records['Opaque payload']['Settings']
                assert records['Invalid']['Settings']['isTypingMode'] is None
                with zipfile.ZipFile(exported) as z:assert next(z.read(n) for n in z.namelist() if n.endswith('retained.bin'))==b'preserve original asset\0bytes'
                assert source.read_bytes()==original
                imported.append(host.state()['library']['active_profile'])
            count=len(host.state()['library']['profiles'])
            for index,os_type in [(4,None),(5,'Linux')]:
                source=root/f'shared-{index}.streamDeckAction';fixture(source,os_type,shared=True);original=source.read_bytes()
                host.post({'command':'slot_file_clipboard','path':str(source)})
                host.post({'command':'slot_paste','controller':'Keypad','index':index,'expected_target':None})
                assert key(index)['settings']['isTypingMode']==(os_type!='Linux')
                assert len(host.state()['library']['profiles'])==count and source.read_bytes()==original
            host.post({'command':'profile_select','id':old_profile});assert key()['settings']==legacy
            shared=root/'old-export.streamDeckAction';host.post({'command':'slot_export','context':key()['context'],'path':str(shared)})
            assert exported_actions(shared)['Text']['Settings']['isTypingMode'] is False and key()['settings']==legacy
            host.stop();host=None;host=start('restart.log');assert key()['settings']==legacy
            host.post({'command':'profile_select','id':imported[0]});assert key()['settings']['isTypingMode'] is True
            assert host.state()['typing_status']['phase']=='disabled'
        finally:
            if host:host.stop()
            for log in logs:
                if log.exists():(a.output/log.name).write_text(log.read_text())
    (a.output/'interop.json').write_text(json.dumps({'tested_binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'native_plain_windows_macos_defaults':True,'linux_archive_defaults':True,'nested_hidden_and_opaque':True,'source_and_asset_bytes_preserved':True,'shared_action_defaults':True,'existing_saved_settings_unchanged':True,'existing_export_explicit_paste':True,'restart_preserves_modes':True,'permission_not_requested':True,'actions_executed':False,'usb_access':False},indent=2)+'\n')
    print('PASS: native/Linux missing Text modes, nested/hidden/opaque records, shared actions, existing-session preservation, explicit portable exports and restart; no actions executed.')

if __name__=='__main__':main()
