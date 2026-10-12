#!/usr/bin/env python3
"""Action copy/move lifecycle and portable asset checks through the actual host.

Uses an unchanged official Counter sample plus an inert local WebSocket probe.
No USB, keyboard injection, external requests or user profiles are touched.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import time
import zipfile

from sample_interop import COUNTER, COUNTER_LOCK_SHA256
from system_interop import Host

PROBE = 'com.deckforge.transfer-probe.action'
FOLDER = 'com.elgato.streamdeck.profile.openchild'


def at(host, index, controller='Keypad'):
    return next((s for s in host.state()['slots'] if s['index'] == index and s['controller'] == controller), None)


def press(host, index):
    for event in ('keyDown', 'keyUp'):
        host.post({'command': 'input', 'event': event, 'index': index})


def assign(host, index, uuid, settings=None):
    host.post({'command': 'assign', 'controller': 'Keypad', 'index': index, 'uuid': uuid})
    action = at(host, index)
    if settings is not None:
        host.post({'command': 'settings', 'context': action['context'], 'settings': settings})
    return action['context']


def make_probe(root, counter):
    root.mkdir()
    source = r'''import {createRequire} from 'node:module';
import {appendFileSync} from 'node:fs';
const WebSocket=createRequire(__ROOT__)('ws'), proof=__PROOF__;
const args={};for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{const m=JSON.parse(data);appendFileSync(proof,JSON.stringify(m)+'\n');
 if(m.event==='keyDown')setTimeout(()=>ws.send(JSON.stringify({event:'setTitle',context:m.context,payload:{title:'STALE REPLY'}})),250);
});'''
    proof = root/'events.jsonl'
    (root/'plugin.mjs').write_text(source.replace('__ROOT__', json.dumps(str(counter.parent/'package.json'))).replace('__PROOF__', json.dumps(str(proof))))
    (root/'manifest.json').write_text(json.dumps({'UUID': 'com.deckforge.transfer-probe', 'Name': 'Transfer probe',
        'CodePath': 'plugin.mjs', 'Nodejs': {'Version': '20'}, 'SDKVersion': 2,
        'Actions': [{'UUID': PROBE, 'Name': 'Transfer probe', 'Controllers': ['Keypad'], 'States': [{'Title': 'Probe'}]}]}))
    return root, proof


def events(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines(keepends=True) if line.endswith('\n')]


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project/'target/release/deckforge')
    parser.add_argument('--samples', type=Path, default=project.parent/'evidence/official-samples')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    counter = args.samples.resolve()/'counter/com.elgato.counter.sdPlugin'
    assert hashlib.sha256((counter.parent/'package-lock.json').read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'boundary': 'Real host, official Counter and inert local probe; simulated input; no USB or user state.'}
    with tempfile.TemporaryDirectory(prefix='deckforge-transfer-') as directory:
        root = Path(directory)
        probe, proof = make_probe(root/'probe.sdPlugin', counter)
        state = root/'state'
        host = Host(binary, [counter, probe], state, root/'host.log').ready()
        try:
            host.wait(lambda s: len(s['plugins']) == 2 and all(p['connected'] for p in s['plugins']), 'plugins registered')
            original_page = host.state()['library']['active_page']
            original_profile = host.state()['library']['active_profile']
            source = assign(host, 0, COUNTER, {'count': 3, 'incrementBy': 2, 'extra': {'retain': [1, 2]}})
            host.wait(lambda s: at(host, 0)['settings'].get('count') == 3, 'Counter source settings')
            host.post({'command': 'slot_copy', 'context': source})
            host.post({'command': 'page_create', 'name': 'Copied page'})
            copied_page = host.state()['library']['active_page']
            host.post({'command': 'slot_paste', 'controller': 'Keypad', 'index': 0, 'expected_target': None})
            copied = at(host, 0)
            assert copied['context'] != source and copied['settings']['extra'] == {'retain': [1, 2]}
            press(host, 0)
            host.wait(lambda s: at(host, 0)['settings']['count'] == 5, 'copied Counter independent input')
            host.post({'command': 'page_select', 'id': original_page})
            assert at(host, 0)['settings']['count'] == 3
            host.post({'command': 'slot_paste', 'controller': 'Encoder', 'index': 0, 'expected_target': None}, expected=400)
            assert at(host, 0, 'Encoder') is None
            occupied = assign(host, 1, PROBE, {'marker': 'destination'})
            host.post({'command': 'slot_transfer', 'context': source, 'controller': 'Keypad', 'index': 1, 'copy': False, 'expected_target': 'stale'}, expected=400)
            assert at(host, 0)['context'] == source and at(host, 1)['context'] == occupied
            host.post({'command': 'open_inspector', 'context': source})
            host.wait(lambda s: any(i['context'] == source and i['connected'] for i in s['inspectors']), 'source property inspector')
            host.post({'command': 'slot_transfer', 'context': source, 'controller': 'Keypad', 'index': 1, 'copy': False, 'expected_target': occupied})
            assert at(host, 0)['uuid'] == PROBE and at(host, 1)['uuid'] == COUNTER
            assert all(i['context'] != source for i in host.state()['inspectors']), 'Moved action retained its old inspector'
            # Moving an action held down releases the old context before it leaves.
            held = at(host, 0)['context']
            host.post({'command': 'input', 'event': 'keyDown', 'index': 0})
            host.post({'command': 'slot_transfer', 'context': held, 'controller': 'Keypad', 'index': 2, 'copy': False, 'expected_target': None})
            moved = at(host, 2)['context']
            host.wait(lambda s: any(e.get('event') == 'willAppear' and e.get('context') == moved for e in events(proof)), 'moved lifecycle')
            observed = [e['event'] for e in events(proof) if e.get('context') == held]
            assert 'keyUp' in observed and 'willDisappear' in observed
            assert observed.index('keyUp') < observed.index('willDisappear')
            new_appear = next(e for e in events(proof) if e.get('event') == 'willAppear' and e.get('context') == moved)
            assert new_appear['payload']['coordinates'] == {'column': 2, 'row': 0}
            time.sleep(.35)
            assert at(host, 2)['title'] != 'STALE REPLY', 'Late reply targeted the moved action'
            # A cut leaves its source until a compatible destination accepts it.
            cut = at(host, 1)['context']
            host.post({'command': 'slot_cut', 'context': cut})
            assert at(host, 1)['context'] == cut and host.state()['clipboard']['mode'] == 'cut'
            host.post({'command': 'page_select', 'id': copied_page})
            host.post({'command': 'slot_paste', 'controller': 'Keypad', 'index': 3, 'expected_target': None})
            assert at(host, 3)['uuid'] == COUNTER and host.state()['clipboard'] is None
            host.post({'command': 'page_select', 'id': original_page})
            assert at(host, 1) is None
            parent = assign(host, 6, 'com.elgato.streamdeck.multiactions.routine')
            host.post({'command': 'multi_save', 'context': parent, 'raw': {'FutureContainer': 81, 'Actions': [{'Actions': [
                {'UUID': PROBE, 'ActionID': 'original-child', 'Settings': {'marker': 'source-child'},
                 'States': [{}], 'FutureChild': {'keep': True}}
            ]}, {'Actions': []}]}})
            original_child = at(host, 6)['multi_children'][0][0]
            host.post({'command': 'slot_copy', 'context': parent})
            host.post({'command': 'slot_paste', 'controller': 'Keypad', 'index': 7, 'expected_target': None})
            copied_parent = at(host, 7)
            copied_child = copied_parent['multi_children'][0][0]
            assert copied_parent['raw']['FutureContainer'] == 81 and copied_child['raw']['FutureChild'] == {'keep': True}
            assert copied_child['context'] != original_child['context']
            assert copied_child['raw']['ActionID'] != original_child['raw']['ActionID']
            host.post({'command': 'settings', 'context': copied_child['context'], 'settings': {'marker': 'copy-child'}})
            assert at(host, 6)['multi_children'][0][0]['settings']['marker'] == 'source-child'
            assert at(host, 7)['multi_children'][0][0]['settings']['marker'] == 'copy-child'
            # Folder copies get independent destination page trees across profiles.
            host.post({'command': 'folder_create', 'index': 4, 'name': 'Original folder'})
            folder = at(host, 4)
            press(host, 4)
            child_page = host.state()['library']['active_page']
            assign(host, 1, COUNTER, {'count': 11, 'incrementBy': 4})
            host.post({'command': 'folder_back'})
            host.post({'command': 'slot_copy', 'context': folder['context']})
            host.post({'command': 'profile_create', 'name': 'Destination'})
            destination = host.state()['library']['active_profile']
            host.post({'command': 'slot_paste', 'controller': 'Keypad', 'index': 0, 'expected_target': None})
            assert at(host, 0)['settings']['ProfileUUID'] != child_page
            press(host, 0)
            press(host, 1)
            host.wait(lambda s: at(host, 1)['settings']['count'] == 15, 'copied folder child works')
            host.post({'command': 'profile_select', 'id': original_profile})
            host.post({'command': 'page_select', 'id': child_page})
            assert at(host, 1)['settings']['count'] == 11
            # Two raw state images must survive a cross-profile copy, source-profile
            # deletion, export and process restart without rasterizing their bytes.
            images = [f'<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144"><rect width="144" height="144" fill="{color}"/></svg>'.encode() for color in ('red', 'blue')]
            archive = root/'states.streamDeckProfile'
            raw_action = {'UUID': 'unknown.transfer.action', 'Name': 'Portable states', 'ActionID': 'original-action', 'State': 0,
                          'Settings': {'future': ['preserve', 9]}, 'FutureAction': {'nested': True},
                          'States': [{'Image': 'Images/red.svg', 'Title': 'Red'}, {'Image': 'Images/blue.svg', 'Title': 'Blue'}]}
            with zipfile.ZipFile(archive, 'w') as z:
                z.writestr('Fixture.sdProfile/manifest.json', json.dumps({'Name': 'Imported assets', 'Controllers': [{'Type': 'Keypad', 'Actions': {'0,0': raw_action}}]}))
                for name, data in zip(('red', 'blue'), images):
                    z.writestr('Fixture.sdProfile/Images/'+name+'.svg', data)
            host.post({'command': 'import_profile', 'path': str(archive)})
            asset_profile = host.state()['library']['active_profile']
            host.post({'command': 'slot_copy', 'context': at(host, 0)['context']})
            host.post({'command': 'profile_select', 'id': destination})
            host.post({'command': 'folder_back'})
            host.post({'command': 'slot_paste', 'controller': 'Keypad', 'index': 5, 'expected_target': None})
            host.post({'command': 'profile_delete', 'id': asset_profile})
            output = root/'portable.streamDeckProfile'
            host.post({'command': 'export_profile', 'path': str(output)})
            with zipfile.ZipFile(output) as z:
                contents = [z.read(n) for n in z.namelist()]
                assert all(image in contents for image in images), 'State image bytes lost during cross-profile transfer'
                assert all(b'asset_source' not in z.read(n) for n in z.namelist() if n.endswith('manifest.json'))
            assert at(host, 5)['raw']['FutureAction'] == {'nested': True}
            host.stop()
            host = Host(binary, [], state, root/'restart.log').ready()
            assert host.state()['clipboard'] is None, 'Clipboard unexpectedly persisted private action content'
            assert at(host, 5)['settings'] == raw_action['Settings'] and at(host, 5)['preview']
            host.post({'command': 'export_profile', 'path': str(root/'after-restart.streamDeckProfile')})
            with zipfile.ZipFile(root/'after-restart.streamDeckProfile') as z:
                assert all(image in [z.read(n) for n in z.namelist()] for image in images)
            report.update(counter_independence=True, cross_page_cut=True, swap=True, held_release_and_stale_reply=True, inspector_closed_on_move=True, nested_copy_independence=True,
                          folder_independence=True, original_state_assets_portable=True, restart=True)
        finally:
            host.stop()
    if args.report:
        args.report.write_text(json.dumps(report, indent=2)+'\n')
    print('PASS: action copy/cut/swap, plugin lifecycle, independent folders, state-image portability and restart; simulation only.')


if __name__ == '__main__':
    main()
