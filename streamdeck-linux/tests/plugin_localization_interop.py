#!/usr/bin/env python3
"""Localized host labels, unchanged SDK dictionaries, and native PI language.

Uses only temporary state and simulated devices; vendor sample files stay intact.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import zipfile

from multi_device_interop import rows, wait
from sample_interop import COUNTER_LOCK_SHA256, require
from system_interop import Host

ID = 'com.deckforge.locale-probe'
ACTION = ID+'.action'
MULTI = 'com.elgato.streamdeck.multiactions.routine'

def dial_slot(snapshot):
    return next(s for s in snapshot['slots'] if s['uuid']==ACTION and s['controller']=='Encoder')

def parent_slot(snapshot):
    return next(s for s in snapshot['slots'] if s['uuid']==MULTI)


def create_probe(root, sdk):
    plugin = root/(ID+'.sdPlugin'); plugin.mkdir()
    events = plugin/'events.jsonl'
    (plugin/'plugin.mjs').write_text(r'''import streamDeck from __SDK__;
import {appendFileSync} from 'node:fs';
const write=value=>appendFileSync(__EVENTS__,JSON.stringify(value)+'\n');
streamDeck.actions.onWillAppear(async ev=>{await ev.action.setTitle('Dynamic English');});
streamDeck.ui.onSendToPlugin(ev=>write(ev.payload));
await streamDeck.connect();
write({event:'registered',language:streamDeck.info.application.language,hello:streamDeck.i18n.translate('hello'),fallback:streamDeck.i18n.translate('fallback')});
'''.replace('__SDK__',json.dumps(sdk.as_uri())).replace('__EVENTS__',json.dumps(str(events))))
    (plugin/'pi.html').write_text('''<!doctype html><meta charset="utf-8"><title>Localization test</title><p id="label"></p><script>
window.connectElgatoStreamDeckSocket=(port,uuid,event,info,actionInfo)=>{
 const language=JSON.parse(info).application.language,ws=new WebSocket(`ws://127.0.0.1:${port}`);
 ws.onopen=()=>{ws.send(JSON.stringify({event,uuid}));document.getElementById('label').textContent=language;
 ws.send(JSON.stringify({event:'sendToPlugin',payload:{event:'pi',language}}));};
};</script>''')
    manifest = {'UUID':ID,'Name':'Base plugin','Description':'Base description','Category':'Base category',
                'Version':'1.0.0','SDKVersion':2,'CodePath':'plugin.mjs','Nodejs':{'Version':'20'},
                'Actions':[{'UUID':ACTION,'Name':'Base action','Tooltip':'Base tooltip','Controllers':['Keypad','Encoder'],
                            'States':[{'Name':'Base off','Title':'Static English'},{'Name':'Base on'}],
                            'Encoder':{'layout':'$B1','TriggerDescription':{'Rotate':'Base turn','Push':'Base push'}},
                            'PropertyInspectorPath':'pi.html'}]}
    (plugin/'manifest.json').write_text(json.dumps(manifest))
    (plugin/'de.json').write_text(json.dumps({'Name':'Deutsches Plugin','Description':'Deutsche Beschreibung',
        'Category':'Must not change category',ACTION:{'Name':'Deutsche Aktion','Tooltip':'Deutscher Hinweis',
        'States':[{'Name':'Aus','Title':'Must not change title'},{'Name':'An'}],
        'Encoder':{'TriggerDescription':{'Rotate':'Drehen','Push':''}}},'Localization':{'hello':'Hallo'}}))
    (plugin/'en.json').write_text(json.dumps({'Name':'English override','Localization':{'hello':'Hello','fallback':'English fallback'}}))
    return plugin, events


def contents(path):
    with zipfile.ZipFile(path) as archive:
        return {name:archive.read(name) for name in archive.namelist()}


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=project/'target/debug/deckforge')
    parser.add_argument('--report',type=Path,required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    sample = project.parent/'evidence/official-samples/counter'
    require(hashlib.sha256((sample/'package-lock.json').read_bytes()).hexdigest()==COUNTER_LOCK_SHA256,'SDK fixture changed')
    sdk = sample/'node_modules/@elgato/streamdeck/dist/plugin/index.js'
    counter = sample/'com.elgato.counter.sdPlugin'
    with tempfile.TemporaryDirectory(prefix='deckforge-plugin-language-') as directory:
        root = Path(directory); state = root/'state'; state.mkdir()
        (state/'session.json').write_text(json.dumps({'slots':[],'plugin_language':'de'}))
        plugin, events = create_probe(root,sdk)
        source_hashes = {p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in plugin.glob('*.json')}
        host = Host(binary,[counter,plugin],state,root/'host.log')
        try:
            host.ready(); host.wait(lambda s:len(s['plugins'])==2 and all(p['connected'] for p in s['plugins']),'plugins connected')
            registered = wait(host,lambda:next((r for r in rows(events) if r.get('event')=='registered'),None),'SDK language')
            require(registered=={'event':'registered','language':'de','hello':'Hallo','fallback':'English fallback'},'SDK custom dictionary language/fallback wrong')
            snapshot = host.state()
            require(next(p for p in snapshot['plugins'] if p['id']=='com.elgato.counter')['name']=='Zähler','Unchanged Counter label not translated')
            action = next(a for a in snapshot['actions'] if a['uuid']==ACTION)
            require(action['name']=='Deutsche Aktion' and action['category']=='Base category','Wrong localized action/category')
            require(action['states'][0]['Name']=='Aus' and action['states'][0]['Title']=='Static English','State name/title boundary broken')
            host.post({'command':'assign','controller':'Encoder','index':0,'uuid':ACTION})
            host.wait(lambda s:dial_slot(s)['feedback'].get('title')=='Dynamic English','unchanged dynamic title')
            slot = dial_slot(host.state()); context = slot['context']
            require(slot['title']=='Static English','Static title was translated')
            require(slot['name']=='Deutsche Aktion' and slot['state_names']==['Aus','An'],'Slot presentation not translated')
            require(slot['trigger_description']=={'rotate':'Drehen','push':'Base push'},'Gesture translation/empty fallback wrong')
            host.post({'command':'settings','context':context,'settings':{'userText':'Keep this English','unknown':{'future':True}}})
            host.post({'command':'open_inspector','context':context})
            wait(host,lambda:any(r.get('event')=='pi' and r.get('language')=='de' for r in rows(events)),'native PI registration language')
            host.post({'command':'close_inspector','context':context})
            # Match the editor's original-declaration defaults for nested actions.
            child = {'UUID':ACTION,'ActionID':'localized-child','Name':action['source_name'],
                     'States':action['source_states'],'State':0,'Settings':{'keep':'original'}}
            host.post({'command':'assign','controller':'Keypad','index':0,'uuid':MULTI})
            parent_context=parent_slot(host.state())['context']
            host.post({'command':'multi_save','context':parent_context,'raw':{'Actions':[{'Actions':[child]},{'Actions':[]}]}})
            host.wait(lambda s:parent_slot(s)['multi_children'][0][0]['title']=='Dynamic English','nested dynamic title')
            parent=parent_slot(host.state())
            require(parent['multi_children'][0][0]['name']=='Deutsche Aktion','Nested default label not localized')
            require(parent['raw']['Actions'][0]['Actions'][0]['Name']=='Base action','Localized nested label leaked into saved raw data')
            require(parent['raw']['Actions'][0]['Actions'][0]['States'][0]['Name']=='Base off','Localized nested state leaked into raw data')
            export_before=root/'before.streamDeckProfile'
            host.post({'command':'export_profile','path':str(export_before)})
            host.post({'command':'plugin_language','value':'fr','baseline':'de'})
            require(host.state()['plugin_language']['active']=='de' and host.state()['plugin_language']['restart_required'],'Language changed before restart')
            host.post({'command':'restart_plugin','id':ID})
            wait(host,lambda:len([r for r in rows(events) if r.get('event')=='registered'])==2,'plugin restart')
            require([r for r in rows(events) if r.get('event')=='registered'][-1]['language']=='de','Plugin restart mixed active languages')
            host.stop()
            host=Host(binary,[],state,root/'restart.log');host.ready()
            host.wait(lambda s:all(p['connected'] for p in s['plugins']),'restored plugins')
            wait(host,lambda:len([r for r in rows(events) if r.get('event')=='registered'])==3,'full restart registration')
            require([r for r in rows(events) if r.get('event')=='registered'][-1]=={'event':'registered','language':'fr','hello':'Hello','fallback':'English fallback'},'Missing language did not retain requested language with SDK English fallback')
            snapshot=host.state(); action=next(a for a in snapshot['actions'] if a['uuid']==ACTION)
            require(action['plugin_name']=='Base plugin' and action['name']=='Base action','Host incorrectly used English file fallback')
            require(dial_slot(snapshot)['context']==context and dial_slot(snapshot)['settings']['userText']=='Keep this English','Restart changed saved action data')
            host.wait(lambda s:dial_slot(s)['feedback'].get('title')=='Dynamic English','restored dynamic title')
            host.wait(lambda s:parent_slot(s)['multi_children'][0][0]['title']=='Dynamic English','restored nested title')
            require(parent_slot(host.state())['multi_children'][0][0]['name']=='Base action','Nested label stayed in the previous language')
            export_after=root/'after.streamDeckProfile'
            host.post({'command':'export_profile','path':str(export_after)})
            require(contents(export_before)==contents(export_after),'Changing language rewrote exported profile contents')
            require(source_hashes=={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in plugin.glob('*.json')},'Source translations or manifest changed')
            report={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'sdk_lock_sha256':COUNTER_LOCK_SHA256,
                    'official_counter_labels':True,'unchanged_sdk_custom_strings_and_fallback':True,'native_pi_language':True,
                    'states_and_gesture_labels':True,'full_restart_language_boundary':True,'no_manifest_english_fallback':True,
                    'profile_contents_and_source_files_preserved':True,'boundary':'Simulated devices, temporary state, unchanged official Counter and SDK2.0.4; native PI registration. No installed app or USB changed.'}
        finally:
            host.stop()
            require(host.process.returncode==0,'Host shutdown failed')
    args.report.parent.mkdir(parents=True,exist_ok=True)
    args.report.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: localized host labels, unchanged SDK custom strings, native PI language and original profile/source preservation.')


if __name__=='__main__': main()
