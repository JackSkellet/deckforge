#!/usr/bin/env python3
"""Exercise the Linux controller against a real private D-Bus MPRIS service."""
import os
from pathlib import Path
import subprocess
import sys
import threading
import tempfile
import time
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

PLAYER = 'org.mpris.MediaPlayer2.Player'
PROPS = 'org.freedesktop.DBus.Properties'

class Player(dbus.service.Object):
    def __init__(self, bus):
        self.name = dbus.service.BusName('org.mpris.MediaPlayer2.spotify', bus)
        super().__init__(bus, '/org/mpris/MediaPlayer2')
        self.values = {
            'PlaybackStatus': dbus.String('Paused'), 'Volume': dbus.Double(.5),
            'LoopStatus': dbus.String('None'), 'Shuffle': dbus.Boolean(False),
            'Position': dbus.Int64(12_000_000), 'CanControl': dbus.Boolean(True),
            'Metadata': dbus.Dictionary({'xesam:title': dbus.String('Fixture track'),
                'xesam:artist': dbus.Array(['Fixture artist'], signature='s'),
                'mpris:length': dbus.Int64(120_000_000),
                'xesam:url': dbus.String('spotify:track:fixture')}, signature='sv'),
        }
    @dbus.service.method(PROPS, in_signature='s', out_signature='a{sv}')
    def GetAll(self, interface):
        assert interface == PLAYER
        return self.values
    @dbus.service.method(PROPS, in_signature='ss', out_signature='v')
    def Get(self, interface, name):
        assert interface == PLAYER
        return self.values[name]
    @dbus.service.method(PROPS, in_signature='ssv')
    def Set(self, interface, name, value):
        assert interface == PLAYER
        self.values[name] = value
    @dbus.service.method(PLAYER)
    def PlayPause(self):
        self.values['PlaybackStatus'] = dbus.String('Playing' if self.values['PlaybackStatus'] == 'Paused' else 'Paused')
    @dbus.service.method(PLAYER, in_signature='x')
    def Seek(self, delta):
        self.values['Position'] = dbus.Int64(self.values['Position'] + delta)
    @dbus.service.method(PLAYER, in_signature='s')
    def OpenUri(self, uri):
        self.values['Metadata']['xesam:url'] = dbus.String(uri)


def main():
    if '--private-bus' not in sys.argv:
        return subprocess.call(['dbus-run-session', '--', sys.executable, __file__, '--private-bus', *sys.argv[1:]])
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    service = Player(bus)
    loop = GLib.MainLoop()
    worker = threading.Thread(target=loop.run, daemon=True)
    worker.start()
    try:
        result = subprocess.run(['node', '--input-type=module', '-e', r'''
import assert from 'node:assert/strict';
import {SpotifyLinuxController} from './plugins/spotify-linux/spotify-linux.mjs';
const player = new SpotifyLinuxController();
let state = await player.getState();
assert.equal(state.isRunning, true);
assert.equal(state.player.state, 'paused');
assert.equal(state.currentTrack.title, 'Fixture track');
assert.equal(state.currentTrack.artist, 'Fixture artist');
assert.equal(state.currentTrack.durationSeconds, 120);
assert.equal(state.player.positionSeconds, 12);
await player.playPause();
assert.equal(await player.getPlaybackState(), 'playing');
await player.setVolume(150);
assert.equal(await player.getVolume(), 100);
await player.changeVolume(-25);
assert.equal(await player.getVolume(), 75);
await player.setShuffling(true);
assert.equal(await player.isShuffleActive(), true);
await player.setRepeating('List');
assert.equal(await player.getAutoRepeatMode(), 'List');
await player.skipBySeconds(-2);
assert.equal((await player.getState()).player.positionSeconds, 10);
await player.playSpotifyUri('spotify:track:second');
assert.equal((await player.getCurrentTrack()).uri, 'spotify:track:second');
assert.throws(() => player.playSpotifyUri('file:///tmp/not-music'));
const absent = new SpotifyLinuxController({service:'org.mpris.MediaPlayer2.absent'});
assert.equal((await absent.getState()).isRunning, false);
console.log('PASS: real private D-Bus playback, metadata, volume, shuffle, repeat, seek, URI, absent-player handling');
'''], cwd=Path(__file__).resolve().parents[1])
        if result.returncode:
            return result.returncode
        if '--plugin' in sys.argv:
            from system_interop import Host
            plugin = Path(sys.argv[sys.argv.index('--plugin') + 1]).resolve()
            project = Path(__file__).resolve().parents[1]
            with tempfile.TemporaryDirectory(prefix='spotify-host-') as directory:
                root = Path(directory)
                host = Host(project / 'target/release/deckforge', [plugin], root / 'state', root / 'host.log')
                try:
                    host.ready()
                    host.wait(lambda state: any(p['id'] == 'com.elgato.spotify' and p['connected'] for p in state['plugins']), 'Spotify registered')
                    host.post({'command': 'assign', 'controller': 'Keypad', 'index': 0, 'uuid': 'com.elgato.spotify.player-control'})
                    context = next(s['context'] for s in host.state()['slots'] if s['index'] == 0)
                    host.post({'command': 'settings', 'context': context, 'settings': {'multimediaCommand': 'togglePlayPause'}})
                    time.sleep(.5)
                    host.post({'command': 'input', 'event': 'keyDown', 'index': 0})
                    host.post({'command': 'input', 'event': 'keyUp', 'index': 0})
                    deadline = time.monotonic() + 5
                    while service.values['PlaybackStatus'] != 'Paused' and time.monotonic() < deadline:
                        time.sleep(.05)
                    assert service.values['PlaybackStatus'] == 'Paused', 'Original action failed to reach Linux MPRIS'
                    print('PASS: original Spotify action through real host registration/key events reaches Linux MPRIS')
                finally:
                    host.stop()
        return 0
    finally:
        loop.quit()
        worker.join(timeout=2)

if __name__ == '__main__':
    raise SystemExit(main())
