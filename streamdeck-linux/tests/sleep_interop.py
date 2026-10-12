#!/usr/bin/env python3
"""Software sleep/screensaver integration in --simulate, without USB or real locking.

Generated WebSocket probe uses the official sample's installed generic ws package.
Its independent heartbeat and feedback updates continue while displays sleep.
Idle deadline arithmetic belongs to the Rust unit tests; this suite does not wait
five minutes or substitute a shortened production timer.
"""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
import io
import json
from pathlib import Path
import tempfile
import threading
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from PIL import Image, ImageChops, ImageDraw
from system_interop import Host
from sample_interop import require

PLUGIN = 'com.deckforge.sleep-probe'
KEY = PLUGIN + '.key'
DIAL = PLUGIN + '.dial'
SLEEP = 'com.elgato.streamdeck.system.sleep'
BRIGHTNESS = 'com.elgato.streamdeck.system.keybrightness'


def probe(path, require_root):
    path.mkdir()
    proof = path / 'events.jsonl'
    (path / 'plugin.js').write_text(r'''import {createRequire} from 'node:module';
import {appendFileSync} from 'node:fs';
const WebSocket=createRequire(__ROOT__)('ws');const proof=__PROOF__;
const args={};for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);const actions=new Map();let tick=0;
const send=(event,context,payload)=>ws.send(JSON.stringify({event,context,payload}));
const record=(event,context)=>appendFileSync(proof,JSON.stringify({event,context,tick,pid:process.pid})+'\n');
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{const m=JSON.parse(data);
 if(m.event==='willAppear')actions.set(m.context,m.action);
 if(m.event==='willDisappear')actions.delete(m.context);
 if(['keyDown','keyUp','dialDown','dialUp','dialRotate','touchTap'].includes(m.event))record(m.event,m.context);
});
setInterval(()=>{if(ws.readyState!==WebSocket.OPEN)return;tick++;
 for(const [context,action]of actions){
  if(action.endsWith('.key'))send('setTitle',context,{title:`Tick ${tick}`});
  else send('setFeedback',context,{title:'Live probe',value:String(tick),indicator:tick%100});
 }
 record('heartbeat',null);
},150);
'''.replace('__ROOT__', json.dumps(str(require_root))).replace('__PROOF__', json.dumps(str(proof))))
    (path / 'manifest.json').write_text(json.dumps({
        'UUID': PLUGIN, 'Name': 'Sleep lifecycle probe', 'SDKVersion': 2,
        'CodePath': 'plugin.js', 'Nodejs': {'Version': '20'},
        'Actions': [
            {'UUID': KEY, 'Name': 'Live key', 'Controllers': ['Keypad'], 'States': [{}]},
            {'UUID': DIAL, 'Name': 'Live dial', 'Controllers': ['Encoder'],
             'States': [{}], 'Encoder': {'layout': '$B1'}},
        ]}))
    return path, proof


def rows(proof):
    if not proof.exists():
        return []
    return [json.loads(line) for line in proof.read_text().splitlines(keepends=True)
            if line.endswith('\n')]


def events(proof, context):
    return [r['event'] for r in rows(proof) if r['context'] == context]


def wait(predicate, description, timeout=5):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = predicate()
        if value:
            return value
        time.sleep(.015)
    raise AssertionError('Timed out: ' + description)


def slot(host, controller='Keypad', index=0):
    return next(s for s in host.state()['slots']
                if s['controller'] == controller and s['index'] == index)


def assign(host, action, controller='Keypad', index=0, settings=None):
    host.post({'command': 'assign', 'controller': controller, 'index': index, 'uuid': action})
    context = slot(host, controller, index)['context']
    if settings is not None:
        host.post({'command': 'settings', 'context': context, 'settings': settings})
    return context


def input_(host, event, index=0, **fields):
    host.post({'command': 'input', 'event': event, 'index': index, **fields})


def saver(host):
    return host.state()['screensaver']


def configure(host, **settings):
    host.post({'command': 'screensaver', 'after_minutes': 0, 'disable_on_lock': False, **settings})


def image(encoded, size):
    result = Image.open(io.BytesIO(base64.b64decode(encoded))).convert('RGB')
    require(result.size == size, f'Wrong preview size: {result.size}, expected {size}')
    return result


def sleep(host):
    host.post({'command': 'sleep_device'})
    require(saver(host)['active'], 'Manual sleep did not activate screensaver')


def wake(host):
    host.post({'command': 'wake_device'})
    require(not saver(host)['active'], 'Explicit wake did not restore profile')


def make_images(root):
    static = root / 'original.png'
    picture = Image.new('RGB', (800, 480), (190, 35, 70))
    ImageDraw.Draw(picture).rectangle((0, 380, 799, 479), fill=(20, 110, 220))
    picture.save(static)
    animated = root / 'original.gif'
    Image.new('RGB', (800, 480), 'red').save(animated, save_all=True,
        append_images=[Image.new('RGB', (800, 480), 'blue')], duration=[400, 600], loop=0)
    return static, animated


def static_and_live_feedback(host, proof, static):
    key = assign(host, KEY)
    dial = assign(host, DIAL, 'Encoder')
    host.wait(lambda s: next(x for x in s['slots'] if x['context'] == key)['title'].startswith('Tick '),
              'live plugin feedback')
    configure(host, image=str(static))
    cached = Path(saver(host)['preferences']['image'])
    require(cached != static and cached.is_file() and cached.read_bytes() == static.read_bytes(),
            'Screensaver image was not copied into private storage')
    static.unlink()
    initial_title = slot(host)['title']
    initial_heartbeats = len(rows(proof))
    sleep(host)
    first = saver(host)
    generation = first['generation']
    full = image(first['preview'], (800, 480))
    require(full.getpixel((13, 12)) == (190, 35, 70), 'Full screensaver pixels differ')
    require(image(slot(host)['preview'], (120, 120)).getpixel((0, 0)) == (190, 35, 70),
            'Key screensaver crop does not match full LCD')
    require(image(slot(host, 'Encoder')['preview'], (200, 100)).getpixel((0, 0)) == (20, 110, 220),
            'Dial screensaver crop does not match touch strip')
    host.post({'command': 'sleep_device'})
    require(saver(host)['generation'] == generation, 'Repeated sleep restarted current generation')
    wait(lambda: len(rows(proof)) >= initial_heartbeats + 3, 'plugin heartbeats while asleep')
    host.wait(lambda _: slot(host)['title'] != initial_title, 'live title updated behind screensaver')
    require(saver(host)['preview'] == first['preview'], 'Plugin feedback replaced screensaver')
    require(len({r['pid'] for r in rows(proof)}) == 1, 'Sleep restarted plugin process')
    wake(host)
    restored = slot(host)
    require(restored['title'] != initial_title, 'Wake restored stale title instead of current plugin feedback')
    require(image(restored['preview'], (120, 120)).getpixel((0, 0)) != (190, 35, 70),
            'Wake retained screensaver pixels')
    sleep(host)
    require(saver(host)['generation'] == generation + 1, 'New sleep did not advance generation')
    wake(host)
    return key, dial, cached


def consumed_inputs(host, proof, key, dial):
    for gestures, context in [(['keyDown', 'keyUp'], key), (['dialDown', 'dialUp'], dial),
                               (['dialRotate'], dial), (['touchTap'], dial)]:
        before = events(proof, context)
        sleep(host)
        for event in gestures:
            input_(host, event, **({'ticks': 1} if event == 'dialRotate' else {}))
        require(not saver(host)['active'], f'{gestures} did not wake display')
        time.sleep(.18)
        require(events(proof, context) == before, f'Waking gesture leaked to plugin: {gestures}')
        for event in gestures:
            input_(host, event, **({'ticks': 1} if event == 'dialRotate' else {}))
        wait(lambda: len(events(proof, context)) == len(before) + len(gestures), 'following normal input')
        require(events(proof, context)[len(before):] == gestures, 'Following input event sequence changed')


def page_and_hold(host, proof, key):
    original = host.state()['library']['active_page']
    sleep(host)
    host.post({'command': 'page_create', 'name': 'Edited while asleep'})
    new = host.state()['library']['active_page']
    assign(host, KEY)
    require(saver(host)['active'], 'Editing page woke sleeping display')
    require(image(slot(host)['preview'], (120, 120)).getpixel((0, 0)) == (190, 35, 70),
            'New page assignment replaced screensaver')
    wake(host)
    require(host.state()['library']['active_page'] == new, 'Wake restored an obsolete page')
    host.post({'command': 'page_select', 'id': original})
    sleep(host)
    input_(host, 'swipe', start=[700, 50], end=[100, 50])
    require(not saver(host)['active'] and host.state()['library']['active_page'] == original,
            'First swipe navigated instead of only waking')
    input_(host, 'swipe', start=[700, 50], end=[100, 50])
    require(host.state()['library']['active_page'] == new, 'Following swipe did not navigate')
    host.post({'command': 'page_select', 'id': original})
    before = events(proof, key)
    input_(host, 'keyDown')
    wait(lambda: len(events(proof, key)) > len(before), 'held plugin key')
    sleep(host)
    wait(lambda: events(proof, key)[len(before):] == ['keyDown', 'keyUp'], 'sleep releases held key')
    wake(host)
    input_(host, 'keyUp')
    time.sleep(.18)
    require(events(proof, key)[len(before):] == ['keyDown', 'keyUp'], 'Late release fired after sleep cancellation')
    assign(host, BRIGHTNESS, index=2)
    host.post({'command': 'device_controls', 'settings': {'brightness': 20}})
    input_(host, 'keyDown', index=2)
    sleep(host)
    time.sleep(.7)
    require(host.state()['device_settings']['desired']['brightness'] == 30,
            'Held brightness repeat continued during sleep')
    wake(host)
    input_(host, 'keyUp', index=2)
    return original, new


def invalid_requests(host, root):
    broken = root / 'broken.gif'; broken.write_bytes(b'not an image')
    old = saver(host)['preferences']
    saved = (root / 'state/session.json').read_bytes()
    for fields in [{'after_minutes': 1}, {'after_minutes': -1}, {'after_minutes': 5.5},
                   {'disable_on_lock': 'false'}, {'image': str(broken)},
                   {'image': str(root / 'missing.png')}, {'image': 42}, {'unknown': True}]:
        host.post({'command': 'screensaver', 'after_minutes': 0, 'disable_on_lock': False,
                   **fields}, expected=400)
        require(saver(host)['preferences'] == old, 'Invalid screensaver request replaced settings')
        require((root / 'state/session.json').read_bytes() == saved, 'Invalid request rewrote session')
    for minutes in [0, 5, 15, 30, 60, 120]:
        configure(host, after_minutes=minutes)
        require(saver(host)['preferences']['after_minutes'] == minutes, 'Supported timeout rejected')
    configure(host)


def newer_settings_win_during_image_preparation(host, root):
    # Forty 1024px frames keep preparation observable without approaching input
    # or retained-frame limits. No artificial delay in the production loader.
    base = Image.linear_gradient('L').resize((1024, 1024)).convert('P')
    base.putpalette([channel for i in range(256) for channel in (i, (i * 3) % 256, 255 - i)])
    frames = [ImageChops.offset(base, index * 31, index * 17) for index in range(40)]
    path = root / 'concurrent.gif'
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=100, loop=0, optimize=False)
    previous = saver(host)['preferences']
    started = threading.Event()

    def slow_configuration():
        request = Request(host.base + '/api/control', json.dumps({
            'command': 'screensaver', 'after_minutes': 5, 'disable_on_lock': False,
            'image': str(path)}).encode(), headers={
                'Content-Type': 'application/json', 'X-Deckforge-Token': host.token}, method='POST')
        started.set()
        try:
            with urlopen(request, timeout=30) as response:
                return response.status, response.read().decode()
        except HTTPError as error:
            return error.code, error.read().decode()

    with ThreadPoolExecutor(max_workers=1) as executor:
        pending = executor.submit(slow_configuration)
        require(started.wait(2), 'Concurrent configuration never started')
        time.sleep(.03)
        require(not pending.done(), 'Fixture loaded before overlapping settings could be tested')
        configure(host, after_minutes=15)
        status, body = pending.result(timeout=30)
    require(status == 400 and 'settings changed' in body,
            f'Stale image preparation was not rejected: {status} {body}')
    require(saver(host)['preferences'] == {**previous, 'after_minutes': 15},
            'Slow image load overwrote the newer settings')
    configure(host)


def animated_and_lock(host, proof, animated, key):
    configure(host, image=str(animated))
    cached = Path(saver(host)['preferences']['image'])
    animated.unlink()
    sleep(host)
    require(saver(host)['frames'] == 2, 'GIF frames were flattened or dropped')
    def color():
        return image(saver(host)['preview'], (800, 480)).getpixel((400, 240))
    require(color() == (255, 0, 0), 'Animation did not start at first frame')
    began = time.monotonic()
    wait(lambda: color() == (0, 0, 255), 'second GIF frame')
    require(time.monotonic() - began >= .20, 'GIF first frame delay was ignored')
    wait(lambda: color() == (255, 0, 0), 'GIF loops to first frame')
    wait(lambda: color() == (0, 0, 255), 'GIF advances again')
    host.post({'command': 'session_state', 'locked': True})
    require(saver(host)['active'] and saver(host)['reason'] == 'lock', 'Lock did not engage screensaver')
    require(color() == (255, 0, 0), 'Lock did not freeze first frame')
    before = events(proof, key)
    dial = slot(host, 'Encoder')['context']
    before_dial = events(proof, dial)
    end = time.monotonic() + 1.1
    while time.monotonic() < end:
        require(color() == (255, 0, 0), 'Locked GIF continued animating')
        time.sleep(.08)
    input_(host, 'keyDown'); input_(host, 'keyUp')
    input_(host, 'dialDown'); input_(host, 'dialRotate', ticks=3)
    input_(host, 'dialUp'); input_(host, 'touchTap')
    host.post({'command': 'wake_device'})
    require(saver(host)['active'], 'Locked device allowed input/manual wake')
    host.post({'command': 'session_state', 'locked': None})
    require(saver(host)['active'], 'Unknown lock state acted as unlock')
    time.sleep(.18)
    require(events(proof, key) == before, 'Locked input reached plugin')
    require(events(proof, dial) == before_dial, 'Locked dial or touch input reached plugin')
    host.post({'command': 'session_state', 'locked': False})
    require(not saver(host)['active'], 'Known unlock failed to wake')
    input_(host, 'keyDown'); input_(host, 'keyUp')
    wait(lambda: len(events(proof, key)) == len(before) + 2, 'unlocked input works')
    configure(host, disable_on_lock=True)
    host.post({'command': 'session_state', 'locked': True})
    require(not saver(host)['active'], 'Disabled lock screensaver still activated')
    host.post({'command': 'session_state', 'locked': False})
    configure(host)
    return cached


def builtin(host):
    for target in ['', host.state()['device']]:
        assign(host, SLEEP, index=1, settings={'DeviceUUID': target})
        input_(host, 'keyDown', index=1)
        require(not saver(host)['active'], 'Sleep action ran before keyUp')
        input_(host, 'keyUp', index=1)
        require(saver(host)['active'] and saver(host)['reason'] == 'action', 'Sleep action did not execute')
        wake(host)
    assign(host, SLEEP, index=1, settings={'DeviceUUID': 'missing-device'})
    input_(host, 'keyDown', index=1)
    host.post({'command': 'input', 'event': 'keyUp', 'index': 1}, expected=400)
    require(not saver(host)['active'], 'Missing target slept current device')
    device = host.state()['device']
    assign(host, SLEEP, index=1, settings={'DeviceUUID': device})
    host.post({'command': 'device_event', 'event': 'disconnected', 'reason': 'sleep target test'})
    input_(host, 'keyDown', index=1)
    host.post({'command': 'input', 'event': 'keyUp', 'index': 1}, expected=400)
    require(not saver(host)['active'], 'Offline current target accepted Sleep action')
    host.post({'command': 'device_event', 'event': 'connected', 'serial': device, 'firmware': 'test'})


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/release/deckforge')
    args = parser.parse_args()
    require_root = project.parent / 'evidence/official-samples/counter/package.json'
    with tempfile.TemporaryDirectory(prefix='deckforge-sleep-') as directory:
        root = Path(directory)
        static, animated = make_images(root)
        plugin, proof = probe(root / 'probe.sdPlugin', require_root)
        host = Host(args.binary.resolve(), [plugin], root / 'state', root / 'host.log')
        try:
            host.ready()
            host.wait(lambda s: all(p['connected'] for p in s['plugins']), 'probe connected')
            key, dial, _ = static_and_live_feedback(host, proof, static)
            consumed_inputs(host, proof, key, dial)
            page_and_hold(host, proof, key)
            invalid_requests(host, root)
            newer_settings_win_during_image_preparation(host, root)
            cached = animated_and_lock(host, proof, animated, key)
            builtin(host)
        except Exception:
            print(host.log.read_text()[-10000:])
            raise
        finally:
            host.stop()
            require(host.process.returncode == 0, 'Host failed to shut down')
        restarted = Host(args.binary.resolve(), [plugin], root / 'state', root / 'restart.log')
        try:
            restarted.ready()
            require(Path(saver(restarted)['preferences']['image']) == cached and cached.is_file(),
                    'Restart lost cached image after original was deleted')
            sleep(restarted)
            require(saver(restarted)['frames'] == 2 and saver(restarted)['error'] is None,
                    'Restart did not load cached animation')
            wake(restarted)
            for cleared in [None, '']:
                configure(restarted, image=cleared)
                sleep(restarted)
                require(saver(restarted)['preferences']['image'] is None and saver(restarted)['frames'] == 1,
                        'Reset did not restore static default logo')
                image(saver(restarted)['preview'], (800, 480))
                wake(restarted)
        finally:
            restarted.stop()
            require(restarted.process.returncode == 0, 'Restarted host failed to shut down')
    print('PASS: static/GIF screensavers, cached persistence, validation atomicity, consumed wake gestures, '
          'live plugin feedback, page/hold lifecycle, generation stability, lock freeze/unlock and Sleep action targets.')
    print('Simulation only: no USB, physical display changes, real session lock or five-minute idle wait.')


if __name__ == '__main__':
    main()
