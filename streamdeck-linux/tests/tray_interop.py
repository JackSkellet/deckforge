#!/usr/bin/env python3
"""Native SNI registration/menu lifecycle for one test-owned simulated desktop.

Requires a graphical session with an active StatusNotifierWatcher and busctl.
Only the spawned process receives D-Bus Activate/menu events. No OS mouse,
keyboard, hardware access, or desktop settings changes occur. Window-open and generation status prove renderer lifecycle, not compositor
focus or pixel-perfect tray painting.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time


def wait(predicate, description, timeout=12):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.04)
    raise AssertionError('Timed out: ' + description)


def bus(*args):
    result = subprocess.run(['busctl', '--user', '--json=short', '--', *map(str, args)],
                            capture_output=True, text=True, timeout=4)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)['data'] if result.stdout.strip() else None


def property_(service, path, interface, name):
    return bus('get-property', service, path, interface, name)


def unwrap(value):
    if isinstance(value, dict):
        if set(value) == {'type', 'data'}:
            return unwrap(value['data'])
        return {key: unwrap(item) for key, item in value.items()}
    if isinstance(value, list):
        return [unwrap(item) for item in value]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/deckforge'))
    args = parser.parse_args(); binary = args.binary.resolve(strict=True)
    watcher = 'org.kde.StatusNotifierWatcher'
    assert property_(watcher, '/StatusNotifierWatcher', watcher, 'IsStatusNotifierHostRegistered'), \
        'An active desktop StatusNotifierHost is required'
    with tempfile.TemporaryDirectory(prefix='deckforge-tray-') as directory:
        root = Path(directory); state = root / 'state'; log = root / 'desktop.log'
        def command(name, required=True):
            result = subprocess.run([str(binary), 'window', name, '--state', str(state)],
                                    capture_output=True, text=True, timeout=4)
            if not required:
                return result
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)
        with log.open('w') as output:
            process = subprocess.Popen([str(binary), 'desktop', '--simulate', '--background',
                                        '--state', str(state)], stdout=output, stderr=subprocess.STDOUT)
            service = f'org.kde.StatusNotifierItem-{process.pid}-1'
            def registered():
                return property_(watcher, '/StatusNotifierWatcher', watcher, 'RegisteredStatusNotifierItems')
            try:
                wait(lambda: 'Native background host ready' in log.read_text(), 'background host startup')
                wait(lambda: any(service in item for item in registered()), 'test-owned tray registration')
                wait(lambda: command('status')['tray_online'], 'reported tray availability')
                initial = command('status')
                assert initial['pid'] == process.pid and initial['visible'] is False, initial
                assert initial['window_open'] is False and initial['editor_generation'] == 0, initial
                interface = 'org.kde.StatusNotifierItem'; path = '/StatusNotifierItem'
                assert property_(service, path, interface, 'Id') == 'deckforge'
                assert property_(service, path, interface, 'Title') == 'Deckforge'
                icons = property_(service, path, interface, 'IconPixmap')
                assert len(icons) == 1 and icons[0][:2] == [64, 64], repr(icons)[:100]
                pixels = icons[0][2]
                assert len(pixels) == 16384 and pixels[:4] == [0, 0, 0, 0], repr(pixels)[:80]
                assert any(pixels[i:i + 4] == [255, 108, 218, 187] for i in range(0, len(pixels), 4)), \
                    'Tray icon lost ARGB color channels'
                menu_path = property_(service, path, interface, 'Menu')
                assert menu_path == '/MenuBar'
                layout = unwrap(bus('call', service, menu_path, 'com.canonical.dbusmenu',
                                    'GetLayout', 'iias', 0, -1, 0))
                def entries(node):
                    identifier, properties, children = node
                    yield identifier, properties
                    for child in children:
                        yield from entries(child)
                labels = {properties['label']: identifier for identifier, properties in entries(layout[1])
                          if 'label' in properties}
                assert set(labels) == {'Open Deckforge', 'Quit'}, labels
                # Activate is a protocol call to this fixture's item only.
                bus('call', service, path, interface, 'Activate', 'ii', 0, 0)
                wait(lambda: command('status')['window_open'], 'tray Activate creates native editor')
                opened = command('status')
                assert opened['pid'] == process.pid and opened['visible'], opened
                assert opened['editor_generation'] == 1, opened
                command('hide')
                wait(lambda: not command('status')['window_open'], 'hide destroys native editor')
                hidden = command('status')
                assert not hidden['visible'] and hidden['editor_generation'] == 1, hidden
                assert process.poll() is None, 'Hiding editor stopped host'
                def menu_click(label):
                    bus('call', service, menu_path, 'com.canonical.dbusmenu', 'Event',
                        'isvu', labels[label], 'clicked', 'i', 0, 0)
                menu_click('Open Deckforge')
                wait(lambda: command('status')['window_open'], 'menu Open recreates native editor')
                reopened = command('status')
                assert reopened['pid'] == process.pid and reopened['visible'], reopened
                assert reopened['editor_generation'] == 2, reopened
                menu_click('Quit')
                assert process.wait(timeout=10) == 0, log.read_text()
                wait(lambda: not any(service in item for item in registered()), 'tray service removal')
                assert command('status', required=False).returncode != 0
                assert (state / 'session.json').is_file(), 'Quit did not preserve session'
            except Exception as error:
                raise AssertionError(str(error) + '\n' + log.read_text()[-6000:]) from error
            finally:
                if process.poll() is None:
                    process.kill(); process.wait(timeout=5)
    print('PASS: owned native SNI registration, Id/Title, 64x64 ARGB icon, menu layout, background without editor, Activate creates editor, Hide destroys it, Open recreates editor in same PID, menu Quit, clean deregistration and saved state.')
    print('No USB or OS mouse/keyboard injection. Compositor focus and visual icon painting are not asserted.')


if __name__ == '__main__':
    main()
