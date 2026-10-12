#!/usr/bin/env python3
"""Brightness actions and device preferences through the simulated diagnostic host.

Runs the real Rust action executor and timers. No USB handles, OS input, display
brightness changes, or native windows. Preview checks cover rendered PNG changes
and feedback values, not physical luminance or exact Windows pixel equivalence.
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

from PIL import Image, ImageChops
from system_interop import Host
from sample_interop import require

BRIGHTNESS = 'com.elgato.streamdeck.system.keybrightness'
TRIGGER = 'com.elgato.streamdeck.keys.adaptor'
MULTI = 'com.elgato.streamdeck.multiactions.routine'
DELAY = 'com.elgato.streamdeck.multiactions.delay'


def control(host, event, index=0, **fields):
    host.post({'command': 'input', 'event': event, 'index': index, **fields})


def item(host, controller='Keypad', index=0):
    return next(s for s in host.state()['slots']
                if s['controller'] == controller and s['index'] == index)


def assign(host, controller='Keypad', index=0, settings=None, action=BRIGHTNESS):
    host.post({'command': 'assign', 'controller': controller, 'index': index, 'uuid': action})
    context = item(host, controller, index)['context']
    if settings is not None:
        host.post({'command': 'settings', 'context': context, 'settings': settings})
    return context


def desired(host):
    return host.state()['device_settings']['desired']


def brightness(host):
    return desired(host)['brightness']


def set_value(host, value, sleep=None):
    host.post({'command': 'device_controls',
               'settings': {'brightness': value, 'sleep_seconds': sleep}})
    require(desired(host) == {'brightness': value, 'sleep_seconds': sleep},
            'Device settings command did not retain requested values')


def stable(host, expected, duration=.65):
    end = time.monotonic() + duration
    while time.monotonic() < end:
        require(brightness(host) == expected, 'Brightness changed after release/cancellation')
        time.sleep(.025)


def wait_value(host, expected):
    return host.wait(lambda s: s['device_settings']['desired']['brightness'] == expected,
                     f'brightness {expected}')


def child(action=BRIGHTNESS, settings=None):
    return {'UUID': action, 'ActionID': str(uuid.uuid4()), 'Settings': settings or {},
            'States': [{}], 'State': 0, 'MultiActionDelays': {'Delay1': 100, 'Delay2': 100}}


def png(slot):
    image = Image.open(io.BytesIO(base64.b64decode(slot['preview']))).convert('RGB')
    require(image.size == (200, 100), 'Dial preview dimensions changed')
    return image


def modes_and_dials(host):
    require(desired(host) == {'brightness': None, 'sleep_seconds': None},
            'Fresh host must not implicitly configure hardware brightness')
    assign(host)
    control(host, 'keyDown')
    require(brightness(host) == 90, 'Missing actionIdx must increase the native default80')
    control(host, 'keyUp')
    for mode, expected in enumerate([57, 37, 100, 75, 50, 25, 0]):
        assign(host, settings={'actionIdx': mode, 'future': {'keep': True}})
        set_value(host, 47)
        control(host, 'keyDown')
        require(brightness(host) == expected, f'Key mode {mode} incorrect')
        control(host, 'keyUp')
        control(host, 'keyUp')
        require(brightness(host) == expected, 'Release changed brightness')
        require(item(host)['settings']['future'] == {'keep': True}, 'Unknown action settings lost')
    for mode in [7, -1, 99]:
        assign(host, settings={'actionIdx': mode})
        set_value(host, 47)
        control(host, 'keyDown'); control(host, 'keyUp')
        require(brightness(host) == 47, 'Unknown native integer mode must be a no-op')
    for settings in [{}, {'actionIdx': None}, {'actionIdx': '0'}]:
        assign(host, settings=settings)
        set_value(host, 95)
        control(host, 'keyDown'); control(host, 'keyUp')
        require(brightness(host) == 100, 'Default/null mode or upper clamp incorrect')
    assign(host, settings={'actionIdx': 1})
    set_value(host, 5)
    control(host, 'keyDown'); control(host, 'keyUp')
    require(brightness(host) == 0, 'Lower key clamp incorrect')

    assign(host, 'Encoder', settings={'actionIdx': 6})
    set_value(host, 47)
    before = item(host, 'Encoder')
    require(before['feedback']['value'] == '47%' and before['feedback']['indicator'] == 47,
            'Brightness feedback does not show the configured percentage')
    before_png = png(before)
    for start, ticks, expected in [(47, 1, 50), (47, -1, 45), (47, 2, 55),
                                   (47, -2, 40), (50, -1, 45), (99, 127, 100),
                                   (1, -128, 0), (47, 0, 47)]:
        set_value(host, start)
        control(host, 'dialRotate', ticks=ticks)
        require(brightness(host) == expected, f'Coarse dial {start}/{ticks} incorrect')
    set_value(host, 47)
    control(host, 'dialDown')
    require(brightness(host) == 47, 'Dial press changed brightness')
    for ticks, expected in [(1, 48), (-2, 46), (127, 100), (-128, 0)]:
        control(host, 'dialRotate', ticks=ticks)
        require(brightness(host) == expected, 'Pressed fine rotation incorrect')
    control(host, 'dialUp')
    control(host, 'touchTap')
    control(host, 'touchTap', hold=True)
    require(brightness(host) == 0, 'Press release or tap changed brightness')
    after = item(host, 'Encoder')
    require(after['layout'] == '$B1' and after['feedback']['value'] == '0%',
            'Dial percentage feedback did not update')
    require(ImageChops.difference(before_png, png(after)).getbbox() is not None,
            'Brightness percentage changes did not reach rendered pixels')


def repeats(host):
    for mode, start, first, direction in [(0, 10, 20, 1), (1, 90, 80, -1)]:
        assign(host, settings={'actionIdx': mode})
        set_value(host, start)
        began = time.monotonic()
        control(host, 'keyDown')
        require(brightness(host) == first, 'Initial key down did not apply immediately')
        stable(host, first, .30)
        wait_value(host, first + direction * 10)
        initial_elapsed = time.monotonic() - began
        require(initial_elapsed >= .45, 'Held repeat started before its 500ms fallback delay')
        repeated_at = time.monotonic()
        wait_value(host, first + direction * 20)
        require(time.monotonic() - repeated_at >= .04, 'Held repeat interval collapsed')
        control(host, 'keyUp')
        stable(host, brightness(host))


def cancellation(host):
    assign(host, settings={'actionIdx': 0})
    set_value(host, 10)
    original_page = host.state()['library']['active_page']
    control(host, 'keyDown')
    host.post({'command': 'page_create', 'name': 'Cancel brightness hold'})
    stable(host, 20)
    control(host, 'keyUp')
    host.post({'command': 'page_select', 'id': original_page})
    set_value(host, 10)
    device = host.state()['device']
    control(host, 'keyDown')
    host.post({'command': 'device_event', 'event': 'disconnected', 'reason': 'brightness hold test'})
    stable(host, 20)
    host.post({'command': 'device_event', 'event': 'connected', 'serial': device, 'firmware': 'test'})
    control(host, 'keyUp')
    stable(host, 20)

    # A Trigger center is a directly held key. Its release/cancellation must
    # clear brightness's timer just as a physical keypad release does.
    parent = assign(host, 'Encoder', action=TRIGGER)
    host.post({'command': 'trigger_save', 'context': parent,
               'raw': {'Actions': [child(), child(), child()]}})
    set_value(host, 10)
    control(host, 'dialDown')
    require(brightness(host) == 20, 'Trigger center did not reach brightness keyDown')
    host.post({'command': 'device_event', 'event': 'disconnected', 'reason': 'trigger brightness hold'})
    stable(host, 20)
    host.post({'command': 'device_event', 'event': 'connected', 'serial': device, 'firmware': 'test'})
    control(host, 'dialUp')
    set_value(host, 10)
    control(host, 'dialRotate', ticks=2)
    wait_value(host, 30)
    stable(host, 30, .8)

    # Cancel queued Trigger pulses while their virtual keys are held.
    set_value(host, 10)
    control(host, 'dialRotate', ticks=10)
    host.post({'command': 'page_create', 'name': 'Cancel virtual brightness'})
    set_value(host, 47)
    stable(host, 47)
    host.post({'command': 'page_select', 'id': original_page})

    parent = assign(host, action=MULTI)
    host.post({'command': 'multi_save', 'context': parent, 'raw': {
        'Actions': [{'Actions': [child(), child(DELAY, {'delay': 3000}),
                                  child(settings={'actionIdx': 6})]}, {'Actions': []}]}})
    set_value(host, 10)
    control(host, 'keyDown'); control(host, 'keyUp')
    wait_value(host, 20)
    host.post({'command': 'page_create', 'name': 'Cancel brightness sequence'})
    require(not host.state()['multi_runs'], 'Page change retained the Multi run')
    set_value(host, 47)
    stable(host, 47)
    host.post({'command': 'page_select', 'id': original_page})


def null_import(host, root):
    profile = root / 'null.streamDeckProfile'
    with zipfile.ZipFile(profile, 'w') as archive:
        archive.writestr('manifest.json', json.dumps({'Name': 'Brightness null settings',
            'Controllers': [{'Type': 'Keypad', 'Actions': {'0,0': {
                'UUID': BRIGHTNESS, 'Settings': None, 'States': [], 'Future': 'preserved'}}}]}))
    host.post({'command': 'import_profile', 'path': str(profile)})
    require(item(host)['settings'] == {}, 'Imported null must present default configuration')
    set_value(host, 47)
    control(host, 'keyDown'); control(host, 'keyUp')
    require(brightness(host) == 57, 'Imported null brightness action did not execute default mode')
    exported = root / 'exported.streamDeckProfile'
    host.post({'command': 'export_profile', 'path': str(exported)})
    with zipfile.ZipFile(exported) as archive:
        actions = [action for name in archive.namelist() if name.endswith('manifest.json')
                   for controller in json.loads(archive.read(name)).get('Controllers', [])
                   for action in (controller.get('Actions') or {}).values()]
    require(any(a.get('UUID') == BRIGHTNESS and a.get('Settings') is None
                and a.get('Future') == 'preserved' for a in actions), 'Null import lost original raw fields')


def preferences(host, state):
    original = host.state()['device']
    set_value(host, 47, 123)
    saved = (state / 'session.json').read_bytes()
    before = host.state()['device_settings']
    for settings in [{'brightness': 101, 'sleep_seconds': 999},
                     {'brightness': -1, 'sleep_seconds': 999},
                     {'brightness': 3.5}, {'brightness': '47'},
                     {'brightness': 30, 'sleep_seconds': -1},
                     {'brightness': 30, 'sleep_seconds': 2147483648},
                     {'brightness': 30, 'unknown_control': True}]:
        host.post({'command': 'device_controls', 'settings': settings}, expected=400)
        require(host.state()['device_settings'] == before, 'Invalid control settings partially applied')
        require((state / 'session.json').read_bytes() == saved, 'Invalid settings modified persistence')
    require(before['applied']['brightness'] is None,
            'Simulation claimed a physical brightness setter was applied')
    other = 'brightness-other-device'
    host.post({'command': 'device_event', 'event': 'connected', 'serial': other, 'firmware': 'test'})
    require(desired(host) == {'brightness': None, 'sleep_seconds': None},
            'New device inherited another serial\'s preferences')
    set_value(host, 25, 456)
    host.post({'command': 'device_event', 'event': 'connected', 'serial': original, 'firmware': 'test'})
    require(desired(host) == {'brightness': 47, 'sleep_seconds': 123},
            'Returning original device lost preferences')
    return original, other


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/deckforge'))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='deckforge-brightness-') as directory:
        root = Path(directory)
        state = root / 'state'
        host = Host(binary, [], state, root / 'host.log')
        try:
            host.ready()
            modes_and_dials(host)
            repeats(host)
            cancellation(host)
            null_import(host, root)
            original, other = preferences(host, state)
        except Exception:
            print(host.log.read_text()[-8000:])
            raise
        finally:
            host.stop()
            require(host.process.returncode == 0, 'Host did not stop cleanly')
        restarted = Host(binary, [], state, root / 'restart.log')
        try:
            restarted.ready()
            require(restarted.state()['device'] == original, 'Restart changed the remembered device')
            require(desired(restarted) == {'brightness': 47, 'sleep_seconds': 123},
                    'Restart lost configured brightness/idle timer')
            restarted.post({'command': 'device_event', 'event': 'connected', 'serial': other, 'firmware': 'test'})
            require(desired(restarted) == {'brightness': 25, 'sleep_seconds': 456},
                    'Restart lost the second device\'s settings')
        finally:
            restarted.stop()
            require(restarted.process.returncode == 0, 'Restarted host did not stop cleanly')
    print('PASS: brightness modes/default/null import, held repeat/release, coarse/fine dial and PNG feedback, '
          'page/disconnect/Trigger/Multi cancellation, atomic invalid settings, per-device preferences and restart.')
    print('Simulation only: no USB handles or physical brightness changes; PNG checks are not Windows pixel parity.')


if __name__ == '__main__':
    main()
