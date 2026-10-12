# Action controls audit

The installed action catalogue contains 43 visible actions plus Delay inside
Multi Actions. This audit covers all 44 definitions: 22 built-ins, five official
sample actions and 17 native Discord actions. Common title, appearance and profile
controls are separate from the action-specific settings below.

Original plugin property inspectors remain available through **Plugin settings**,
including for saved children in sequences. Advanced JSON retains unknown imported
fields; a missing primitive in an empty settings object is not treated as a schema.
Edits are merged against the settings actually displayed by the control. If a
plugin updates another field before the callback runs, that newer value is kept;
an explicit user deletion still takes effect. This prevents a stale editor
callback from deleting newly initialized plugin data.

## Built-in actions

Unless fully qualified, IDs below begin with `com.elgato.streamdeck.`.

| Action ID | Editable controls |
| --- | --- |
| system.website | URL, browser/background request, custom browser executable. |
| system.open | Application/file/folder path and native pickers. |
| system.hotkey | Key or dial left/press/right shortcuts, modifiers, key selection, clear and modifier-only chords. Imported unknown chord fields remain intact. |
| system.multimedia | Media operation; dial volume step. |
| system.keybrightness | Key presets/increase/decrease; fixed dial increments are explained. |
| system.sleep | Saved device target, with owning-device behavior explained. Other-device targeting remains unsupported. |
| soundboard.playaudio | File, playback mode, volume, fade mode/duration and explicit Linux output mapping. |
| soundboard.stopaudioplay | No per-action settings; stops Deckforge sounds. |
| page.next | No per-action settings; next top-level page. |
| page.previous | No per-action settings; previous top-level page. |
| page.goto | One-based page number. |
| page.indicator | No per-action settings; current-page display. |
| profile.openchild | Named target page from the active profile. Missing imported targets remain visible. |
| profile.backtoparent | No per-action settings; return from a folder. |
| multiactions.routine | Ordered children, child settings/inspectors, requested state and optional hold/gap timing. |
| multiactions.routine2 | Two sequences with the same child controls. |
| multiactions.random | Candidate actions with the same child controls. |
| multiactions.delay | Delay in milliseconds, up to 24 hours. |
| dial.stack | Encoder children, order, selected action, stack icon and fast-navigation options. |
| keys.adaptor | Left/press/right child actions and nested settings. |
| com.deckforge.mixer.channel | Input group, mix, key operation/level/change or dial step, including fractional values. |
| com.deckforge.mixer.mix | Mix, key operation/level/change or dial step. |

Multi Action default state removes `OverrideState`; it no longer writes the invalid
value `-1`. Custom timing is opt-in and can be reset to the plugin defaults.
Soundboard output selection writes a Linux override while retaining the original
foreign output identifier. Unsupported Coalesce behavior is disclosed rather than
presented as an operative checkbox. Launch arguments and extra media/hardware
operations are not invented by the editor.

## Official sample actions

| Action ID | Editable controls |
| --- | --- |
| com.elgato.counter.action | Increment by, normally 1–5. Imported custom increments remain preserved and editable through Advanced settings. |
| com.elgato.hello-world.newstring | Sample language or host default. Choosing host default removes the override. |
| com.elgato.layouts.built-in-layout | No authored configuration; turn the dial to choose the demonstration layout. |
| com.elgato.layouts.custom-layout | No authored configuration; turn the dial to choose the demonstration layout. |
| com.elgato.layouts.feedback | No authored configuration; turn the dial to adjust feedback. |

Runtime `count`, `titleKey` and Layouts `value` remain in stored data and Advanced
settings, but are not mistaken for ordinary configuration schemas. The sample
plugins and their original inspectors remain unchanged.

## Native Discord actions

IDs below begin with `com.elgato.discord.`. Every action also has the shared
connection and authorization controls.

| Action ID | Editable controls |
| --- | --- |
| soundboard | Server and sound ID. Sound catalogue browsing is not implemented; saved sound-name/emoji fields are compatibility metadata. |
| mute | No per-action settings. |
| deafen | No per-action settings. |
| channel.voice | Named server and channel, explicit Disconnect choice, custom IDs. |
| channel.text | Named server and text channel, custom IDs. |
| pushto.talk | No per-action settings; held gesture. |
| pushto.mute | No per-action settings; held gesture. |
| setaudiodevice | Input/output/both mode and named devices with custom IDs. |
| serverstats | Server, message count, user count and press behavior. |
| pushtotalktoggle | No per-action settings. |
| notifications | Press behavior. |
| videotoggle | No per-action settings. |
| streamtoggle | No per-action settings. |
| uservolumecontrolbutton | User, mode, mute operation, display name, icon/meter style, signed adjustment or target volume. |
| uservolumecontroldial | Most-recent speaker or specific user and percent per tick. |
| volumecontrolbutton | Input/output, mode, icon/meter style, signed adjustment or target volume. |
| volumecontrol | Input/output and percent per tick. |

The inspector now loads its schema as a local script. The previous `fetch` of a
local JSON file failed in native WebKit and left the entire settings section
blank. Packaging checks keep that script consistent with the canonical schema.

Named selections retain custom IDs and unknown fields. Channel lists carry their
source server and are cleared on server/session changes, so another server's
cached list cannot be offered as current. Changing a server leaves its channel
unselected; disconnecting requires an explicit choice. Volume adjustments show
signed percentages while preserving the original encoded settings. Input target
volume is limited to 100%, output/user volume to 200%.

Long-press notification/statistics actions and dynamic server-avatar artwork remain
unimplemented and are labeled accordingly. Permission-dependent operations retain
explicit authorization requirements; displaying a control does not grant access.

## Validation

- React interaction tests cover field editing, conditional controls, imported
  values, custom timing, source-data retention and delayed saves across devices.
- `tests/native_builtin_controls_interop.py` edits and saves actual native forms.
  It also verifies unchanged Hello World responds to language changes.
- `plugins/discord/tests/pi-forms.cjs` checks all 17 schemas, dynamic choices,
  conditional modes, percentage encoding, missing/custom IDs and retained data.
- `plugins/discord/tests/native-pi-forms.cjs` reproduces the original blank form
  and checks all 17 corrected forms in real WebKit, including settings frames.
- Discord Rust tests verify picker provenance, cache reset and obsolete-error
  cleanup. Permission failures remain visible.

These checks use isolated state, simulated devices and mock Discord lists. They
do not press user shortcuts, play audio, route microphones, or establish complete
live acceptance for every external service and device. Remaining action behavior
gaps stay recorded in [the full parity ledger](FEATURE_PARITY.md).
