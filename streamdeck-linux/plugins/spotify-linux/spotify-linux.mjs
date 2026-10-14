// Linux controller for the vendor Spotify plugin. Uses the existing session bus.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const run = promisify(execFile);
const path = '/org/mpris/MediaPlayer2';
const player = 'org.mpris.MediaPlayer2.Player';
const properties = 'org.freedesktop.DBus.Properties';
const missingPlayer = error => /ServiceUnknown|NameHasNoOwner|not provided by any .service|does not exist|not activatable/.test(error.stderr || error.message);

export class SpotifyLinuxController {
  constructor({ execute = run, service = 'org.mpris.MediaPlayer2.spotify' } = {}) {
    this.execute = execute;
    this.service = service;
    this.client = null; // The plugin's existing polling loop reads getState().
  }
  async initialize() { return this; }
  async bus(...args) {
    const { stdout } = await this.execute('busctl', ['--user', '--json=short', '--timeout=3', '--', ...args], { timeout: 4000, env: { ...process.env, LC_ALL: 'C' }, maxBuffer: 1024 * 1024 });
    return stdout.trim() ? JSON.parse(stdout).data : [];
  }
  async call(method, signature, ...args) {
    await this.bus('call', this.service, path, player, method, ...(signature ? [signature, ...args.map(String)] : []));
    return true;
  }
  async set(name, type, value) {
    await this.bus('set-property', this.service, path, player, name, type, String(value));
    return true;
  }
  async get(name) {
    const value = await this.bus('get-property', this.service, path, player, name);
    return value;
  }
  async getState() {
    let raw;
    try { [raw] = await this.bus('call', this.service, path, properties, 'GetAll', 's', player); }
    catch (error) { if (!missingPlayer(error)) throw error; }
    const data = Object.fromEntries(Object.entries(raw || {}).map(([key, variant]) => [key, variant.data]));
    const meta = Object.fromEntries(Object.entries(data.Metadata || {}).map(([key, variant]) => [key, variant.data]));
    const state = String(data.PlaybackStatus || 'Stopped').toLowerCase();
    const positionSeconds = Number(data.Position || 0) / 1e6;
    const durationSeconds = Number(meta['mpris:length'] || 0) / 1e6;
    const uri = meta['xesam:url'] || '';
    return {
      isRunning: !!raw,
      spotifyHasTrack: !!meta['xesam:title'],
      player: {
        state, isPlaying: state === 'playing', volume: Math.round(Number(data.Volume || 0) * 100),
        positionSeconds, durationSeconds, autoRepeatMode: data.LoopStatus === 'Playlist' ? 'List' : (data.LoopStatus || 'None'),
        isShuffleActive: !!data.Shuffle, isShuffleEnabled: data.CanControl !== false, isRepeatEnabled: data.CanControl !== false,
      },
      currentTrack: {
        id: uri || meta['mpris:trackid'] || '', uri, title: meta['xesam:title'] || '',
        artist: (meta['xesam:artist'] || []).join(', '), album: meta['xesam:album'] || '',
        artworkUrl: meta['mpris:artUrl'] || '', positionSeconds, durationSeconds, duration: durationSeconds,
      },
    };
  }
  play() { return this.call('Play'); }
  pause() { return this.call('Pause'); }
  playPause() { return this.call('PlayPause'); }
  stop() { return this.call('Stop'); }
  nextTrack() { return this.call('Next'); }
  prevTrack() { return this.call('Previous'); }
  previousTrack() { return this.prevTrack(); }
  getVolume() { return this.get('Volume').then(value => Math.round(value * 100)); }
  setVolume(value) { return this.set('Volume', 'd', Math.max(0, Math.min(100, Number(value))) / 100); }
  async changeVolume(delta = 5) { return this.setVolume(await this.getVolume() + Number(delta)); }
  volumeUp() { return this.changeVolume(5); }
  volumeDown() { return this.changeVolume(-5); }
  setShuffling(value) { return this.set('Shuffle', 'b', !!value); }
  setRepeating(value) { return this.set('LoopStatus', 's', value === 'List' ? 'Playlist' : value); }
  skipBySeconds(seconds = 10) { return this.call('Seek', 'x', Math.round(Number(seconds) * 1e6)); }
  skipForward() { return this.skipBySeconds(10); }
  skipBackward() { return this.skipBySeconds(-10); }
  playSpotifyUri(uri) {
    if (!/^spotify:|^https:\/\/open\.spotify\.com\//.test(uri)) throw new Error('Expected a Spotify URI');
    return this.call('OpenUri', 's', uri);
  }
  playTrack(uri) { return this.playSpotifyUri(uri); }
  async isSpotifyRunning() { return (await this.getState()).isRunning; }
  async launchSpotify() { await this.execute('xdg-open', ['spotify:'], { timeout: 5000 }); return true; }
  async quit() { await this.bus('call', this.service, path, 'org.mpris.MediaPlayer2', 'Quit'); return true; }
  async getPlaybackState() { return (await this.getState()).player.state; }
  async getCurrentTrack() { return (await this.getState()).currentTrack; }
  async getCurrentArtwork() { return (await this.getCurrentTrack()).artworkUrl; }
  async getCurrentTrackName() { return (await this.getCurrentTrack()).title; }
  async getCurrentTrackArtist() { return (await this.getCurrentTrack()).artist; }
  async getCurrentTrackAlbum() { return (await this.getCurrentTrack()).album; }
  getPlayerVolume() { return this.getVolume(); }
  async getAutoRepeatMode() { return (await this.getState()).player.autoRepeatMode; }
  async isShuffleActive() { return (await this.getState()).player.isShuffleActive; }
  isEventsEnabled() { return false; }
}
