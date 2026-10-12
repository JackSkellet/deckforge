"""Private PipeWire/Pulse fixture with no device discovery or real session sockets.

Socket isolation: https://docs.pipewire.org/page_module_protocol_native.html
Only factories and a dummy driver are loaded. No ALSA/Bluetooth/device monitor.
"""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import time


@contextmanager
def private_audio(root: Path):
    runtime = root / 'runtime'
    config = root / 'pipewire-config'
    runtime.mkdir(mode=0o700)
    config.mkdir()
    pulse_socket = runtime / 'pulse-native'
    (config / 'test-core.conf').write_text('''
context.properties = { core.daemon = true core.name = deckforge-test support.dbus = false }
context.spa-libs = { audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }
context.modules = [
 { name = libpipewire-module-protocol-native }
 { name = libpipewire-module-metadata }
 { name = libpipewire-module-spa-node-factory }
 { name = libpipewire-module-client-node }
 { name = libpipewire-module-access args = { access.force = unrestricted } }
 { name = libpipewire-module-adapter }
 { name = libpipewire-module-link-factory }
 { name = libpipewire-module-session-manager }
]
context.objects = [
 { factory = metadata args = { metadata.name = default } }
 { factory = spa-node-factory args = { factory.name = support.node.driver node.name = Test-Dummy-Driver priority.driver = 20000 } }
]
''')
    (config / 'test-pulse.conf').write_text('''
context.properties = { support.dbus = false }
context.spa-libs = { audio.convert.* = audioconvert/libspa-audioconvert support.* = support/libspa-support }
context.modules = [
 { name = libpipewire-module-protocol-native }
 { name = libpipewire-module-metadata }
 { name = libpipewire-module-client-node }
 { name = libpipewire-module-adapter }
 { name = libpipewire-module-protocol-pulse args = { server.address = [ "unix:''' + str(pulse_socket) + '''" ] } }
]
''')
    wp_config = root / 'wireplumber-config'
    wp_config.mkdir()
    (wp_config / 'wireplumber.conf').write_text(Path('/usr/share/wireplumber/wireplumber.conf').read_text())
    env = dict(os.environ)
    env.update(XDG_RUNTIME_DIR=str(runtime), PIPEWIRE_RUNTIME_DIR=str(runtime),
               PIPEWIRE_CONFIG_DIR=str(config), PIPEWIRE_REMOTE='deckforge-test',
               PULSE_RUNTIME_PATH=str(runtime), PULSE_SERVER='unix:' + str(pulse_socket),
               WIREPLUMBER_CONFIG_DIR=str(wp_config), XDG_STATE_HOME=str(root / 'state'),
               DBUS_SESSION_BUS_ADDRESS='unix:path=' + str(runtime / 'no-session-bus'),
               DBUS_SYSTEM_BUS_ADDRESS='unix:path=' + str(runtime / 'no-system-bus'))
    children = []
    handles = []
    try:
        for command in [['pipewire', '-c', 'test-core.conf'], ['pipewire-pulse', '-c', 'test-pulse.conf'], ['wireplumber', '--profile', 'policy']]:
            log = (root / (command[0] + '.log')).open('w')
            handles.append(log)
            children.append(subprocess.Popen(command, env=env, stdout=log, stderr=log))
            if command[0] == 'pipewire':
                deadline = time.monotonic() + 4
                while not (runtime / 'deckforge-test').exists():
                    assert children[-1].poll() is None, (root / 'pipewire.log').read_text()
                    assert time.monotonic() < deadline, 'Private core socket timeout'
                    time.sleep(.03)
        deadline = time.monotonic() + 5
        while True:
            assert all(child.poll() is None for child in children), 'Private server stopped'
            attempt = subprocess.run(['pactl', '--format=json', 'info'], env=env, capture_output=True, text=True, timeout=1)
            if attempt.returncode == 0:
                break
            assert time.monotonic() < deadline, attempt.stderr
            time.sleep(.04)
        # This must be a fresh server with no discovered hardware endpoints.
        for kind in ['sinks', 'sources']:
            items = json.loads(subprocess.check_output(['pactl', '--format=json', 'list', kind], env=env, text=True))
            assert items == [], f'Unexpected devices on private server: {kind}'
        yield env
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=3)
        for handle in handles:
            handle.close()
