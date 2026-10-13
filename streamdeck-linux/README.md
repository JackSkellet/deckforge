# Deckforge

**Made with AI.** Elgato wasn't making a Linux port, and I didn't want an expensive paperweight on my table.

An independent Linux Stream Deck desktop application written from scratch in
Rust, with a React/TypeScript editor embedded in a native Wry/WebKit window. A
small in-process bridge calls the same Rust host. Plugin HTML runs in separate
native WebKit windows. A separate headless diagnostic mode retains
the earlier local web editor. No existing Stream Deck host, driver implementation,
or application template is used.
Generic USB, image, font, HTTP, WebSocket, archive and native browser libraries
are dependencies. Linux HTML support uses system GTK3 and WebKitGTK4.1 (2.40+) libraries.

This is a working early compatibility build, **not a complete port of Elgato's
application**. Its first targets are the unchanged official Counter and Layouts
feedback sample plugins. Elgato's Windows executable is never run. The supported
protected Marketplace format can import the original macOS 7.6.0.23012 installer
to obtain a local static verification reference. No vendor executable is run.

## Device models

Model-specific transports and layouts cover Mini, the 2019/MK.2/Scissor 15-key
family, the original 2017 device, XL, Plus, Plus XL, Neo and Studio. Only Plus has physical validation so far;
the additional transports have protocol and software tests. Pedal has an
input-only transport recovered from the original executable. Studio supports
32 key displays, two screenless dials and brightness; LED/ring output remains
unsupported. See [Device models](docs/DEVICES.md) for supported USB
identities, selection commands and remaining limits.

## Multiple devices

Choose a deck in the header to edit it while other decks keep running. Plugins
remain shared; pages, assignments, controls and reconnect state belong to each
device. Copy profiles through **Profile options → Copy to device…**, or use the
action clipboard between compatible controls. See [Multiple devices](docs/MULTI_DEVICE.md)
for ownership, persistence and validation boundaries.

## Native Discord plugin

[Discord for Deckforge](plugins/discord/README.md) is a from-scratch Rust plugin
with the original 17 Discord action identities. It runs as a Linux executable,
with its own settings panel and artwork. No Wine is used. Notifications, server
statistics and volume graphics have mock integration coverage. Live authorization and connection were confirmed by the user; feature-specific
Discord control acceptance remains incomplete. Native distributions include the
plugin without credentials or tokens. Install it from **Plugins → Included with
Deckforge**, then connect your own account in its settings.

## Appearance editor

Select a control and expand **Appearance** for custom icons and per-state title
styling. Rust renders the draft preview; Save applies it. User overrides retain
priority over live plugin updates, and Reset restores the underlying appearance.
See [Action appearance](docs/APPEARANCE.md) for font substitution, export behavior
and the remaining rendering limits.

## Audio mixer

Open **Audio mixer** to configure independent monitor and stream mixes, input
groups, application assignments and output devices. Routing is disabled by default;
changes require **Apply routes**. **Mixer channel** and **Mixer output mix** actions
provide key/dial controls with configurable steps and live control feedback.
See [Audio mixer](docs/AUDIO_MIXER.md) for setup, recovery behavior and the remaining
Wave Link compatibility work. Live meters and the advanced audio patchbay are
implemented; effects and Wave-device DSP remain deferred.

## Run the native application

Open **Deckforge** from the Linux application menu, or run:

```sh
~/.local/bin/deckforge
```

The installed optimized executable is `~/.local/bin/deckforge`. Its desktop entry
is `~/.local/share/applications/deckforge.desktop`. Default application data lives
in `$XDG_DATA_HOME/deckforge`, or `~/.local/share/deckforge` when unset.
Local development sessions and external reference fixtures are not included in
the repository.

Select a key or dial, search the **Action library** by action or plugin name,
and use its **+** button to assign. You can also drag an action onto a compatible
control. Categories and action icons come from plugin manifests; **All**, **Keys**
and **Dials** filter the library. Incompatible assignments are disabled.
Edit simple settings with text, numeric and checkbox controls,
then Save changes.
The current 44 action definitions have been audited for editable controls. Counter
increments, sample language, shortcut choices, folder targets, audio outputs and
sequence timing have explicit editors. Discord's property inspector includes
named server/channel/user/device choices and mode-specific controls. See the
[complete action control inventory](docs/ACTION_CONTROLS.md) for per-action
coverage and remaining backend limitations.
Right-click a control or use **Edit** to copy, cut, paste or remove it. Dragging
between occupied controls swaps them; Ctrl-drag copies. See
[Action editing](docs/ACTION_EDITING.md) for shortcuts, folder copies and draft
protection. [Protected packages](docs/PROTECTED_PACKAGES.md) describes the optional
static reference needed by some Marketplace downloads. Use **Plugins → Import
Elgato installer** to extract it from the supported `.pkg` without entering an
executable path or logging into Elgato.
Use **Plugin settings…** to open an action's original HTML property inspector
when its manifest provides one. Advanced JSON editing remains available. Draft changes survive periodic refresh
and switching between controls. **Plugins** contains installation and runtime
status, Enable, Disable, Restart and Remove, plus managed package updates and
previous-version restoration. Disabling a plugin stops its backend and
inspectors while preserving assignments and settings, and remains in effect
after restarting Deckforge. Remove retains profile assignments and settings for
reinstall, privately archives managed plugin folders and leaves external folders
untouched. **Update from package** reviews the incoming version before replacing
files; **Restore version** returns to a retained predecessor while keeping your
current settings. See [plugin lifecycle](docs/PLUGIN_LIFECYCLE.md).
**Preferences → Marketplace** signs in through Elgato's own page and remembers
the session for browser install links. Downloads use the existing package
verification and installation path; replacements require update review.
See [Marketplace](docs/MARKETPLACE.md) for session behavior and validation limits.
The left sidebar manages profiles and imports. Title editing, removal and input
testing stay with the selected control. Closing or hiding the editor
keeps the device and plugins running. Open Deckforge again from the launcher or
tray to restore the editor, including unsaved drafts. **Quit** in the editor or
tray stops the host and its plugin processes.

The plugin manager shows each plugin's connection status, version, folder and log
path. A plugin that cannot start stays listed with its error; the rest of the app
keeps running. **Restart** reloads that plugin from its folder and restores its
visible action contexts with their saved settings. It does not automatically retry
crashing plugins. Missing or malformed plugin folders stay in the session so they
can be repaired and retried. Windows-only executables still require a compatible
runtime or Linux port; restarting does not make them supported.

Plugins that monitor application launches can use **Monitored applications** to
map their Windows or macOS identifiers to Linux executable names or paths.
Mappings are explicit and saved per plugin. See
[application monitoring](docs/PLUGIN_APPLICATIONS.md) for matching and event
behavior. [System-resume events](docs/PLUGIN_SYSTEM_EVENTS.md) replay visible
actions and notify plugins after logind reports sleep completion.

**Preferences → Startup & about → Plugin language** selects localized plugin and
action labels and the language supplied to property inspectors. It applies after
a full app restart; saved profiles and user titles are preserved. See
[plugin localization](docs/PLUGIN_LOCALIZATION.md).

The tray uses Linux StatusNotifierItem. If your desktop has no tray, reopening
the launcher still restores the editor through a private local socket. This
works by closing and recreating the actual native window, including on Wayland.
Compositor focus policy may prevent it from taking keyboard focus automatically.

```sh
deckforge desktop --background
deckforge window show
deckforge window quit
```

These commands use the default application state; add `--state /path/to/state`
for a separate session. Opening the same state again activates its existing
process. Startup flags such as `--plugin` and `--profile` apply only when starting
a new host. In **Preferences**, enable **Start this session when I log in** to start it in
the background at your next desktop login. It defaults to off. The setting uses
`$XDG_CONFIG_HOME/autostart/deckforge.desktop`, falling back to
`~/.config/autostart/deckforge.desktop`. Disabling writes `Hidden=true` so a
system-wide entry with the same name cannot re-enable startup. Custom entries
are preserved. The executable, state directory and simulation mode are retained.

The same setting is available as `deckforge startup enable`, `disable`, or
`status`, with optional `--state` and `--simulate`. A state directory must exist
before enabling startup. These commands do not log you out or start a second host.
**Preferences** also shows the application version and source repository.

Build and install an updated native executable:

```sh
cd streamdeck-linux
npm --prefix frontend ci
npm --prefix frontend run build
cargo build --release --locked
python scripts/install-desktop.py
```

To build a distributable archive containing the native app and Discord plugin:

```sh
python3 scripts/build-distribution.py
```

The archive is written to `target/release/deckforge-linux-<architecture>.tar.gz`.
Extract it and run its `deckforge` executable, or run its
`scripts/install-desktop.py` installer. The bundled plugin is offered in Plugins;
existing installations and private credentials are not overwritten. No user state,
Marketplace cookies, Discord credentials or connection tokens enter the archive.
Use `--skip-build` only when the host and plugin release binaries are already
built from the desired sources. See [Discord setup](plugins/discord/README.md).

Node.js 20.19+ is needed to build the editor. The generated bundle is embedded in
the Rust executable, with no server needed at runtime. Rebuild the frontend before
compiling Rust after editor changes. `desktop --legacy-editor` temporarily retains
the earlier egui editor as a migration fallback.

The per-user installer updates its executable, icon and launcher, keeping replaced
files as `.previous`. It also registers `streamdeck://` plugin links when no other
default handler is configured; existing handlers are preserved. See
[Plugin browser links and callbacks](docs/PLUGIN_URLS.md). It does not install system packages, a startup
service or udev rules. It leaves application settings unchanged. Your Linux seat
or udev configuration must grant access to the device. No device reset or firmware
write is used.

For development with the workspace's original sample session:

```sh
cargo build --locked
./run-samples.sh
```

Do not run two hosts on the same physical deck. `--simulate` opens no USB device.
The existing headless `serve` command still exposes the diagnostic browser editor
at its printed loopback address; it is not the native application's interface.
Native mode exposes only the plugin WebSocket endpoint, not the
browser editor or its HTTP control routes.

## Built-in Linux actions

Choose **Website**, **Open application, file or folder**, **Hotkey**, or **Media
control** from the native action picker. Each has dedicated settings controls.
Website supports a default/selected browser or a bounded background HTTP request.
Open uses a Linux executable or the desktop file association; imported Windows
paths stay editable and produce an explicit error until mapped to a Linux target.
Profile strings are never evaluated as shell commands.

Hotkeys use a persistent Linux uinput keyboard. Key down/up owns and releases the
shortcut; page changes and shutdown release departing holds. Dial shortcuts map
to counterclockwise, clockwise, and press/tap independently. Media keys include
transport and volume/mute; the System volume dial maps rotation to volume steps
and press/tap to mute. System Volume uses native PulseAudio/PipeWire control:
the dial and editor show the current percentage and mute state, including changes
made elsewhere. **Volume step per tick** sets 0.1–100 percentage points per tick,
defaulting to 5. Channel balance is preserved, and device changes are detected.
Changing volume does not clear mute; press or tap to toggle it.

The host needs write access to `/dev/uinput` for hotkeys and transport media keys.
The System Volume dial uses the audio server directly. It does
not change permissions automatically. Qt shortcut identities are translated to
Linux physical key codes; arbitrary keyboard-layout equivalence and Coalesce
behavior remain unverified. The kernel device was created/removed in testing,
but event readback could not run with current event-node permissions. The user confirmed physical volume and mute controls work. Other desktop
shortcuts/transport effects remain unverified. Live percentage/step behavior is
tested against an owned virtual output, with a read-only comparison to the real
default output.

The latest action result appears in the native window. Network work runs outside
the editor and input worker. `--simulate` prevents USB access only: configured
System actions and plugins can still perform their normal OS/network operations.
Close/Open Application activation, text entry, hotkey switch and other desktop
actions remain on the parity ledger.

## Device brightness

**Preferences → Device brightness** applies a value from 0–100%. The value is
saved for the selected device and restored after reconnecting. Existing sessions
without an override keep their hardware brightness until you apply a setting or
use a brightness action.

Assign **Device brightness** to a key for increase/decrease by 10 points or fixed
100%, 75%, 50%, 25% and 0% values. The original action UUID and `actionIdx` settings
are supported, including shipped null settings. A held increase/decrease key
repeats after 500 ms, then every 100 ms. These are the original app's fallback
timings; its Windows keyboard-repeat preference integration is not reproduced on
Linux yet. Releases, page changes, cancellation and disconnects stop affected
held actions.

On a dial, turn to the next/previous 5% boundary; hold while turning for 1% steps.
Press or tap alone does nothing. The touch strip shows the requested percentage.
When no brightness has been chosen, the first relative action starts from the
original app's default of 80%. Elgato documents a brightness setter but no getter,
so this percentage is a requested value, not a physical luminance measurement.
The original Plus mapping is retained: logical 0–100% maps to raw HID 4–100,
so logical 50% sends raw 52. Raw zero belongs to the separate hardware sleep path.

## Sleep and screensavers

In **Preferences → Screensaver**, choose a PNG, JPEG or GIF, set **Sleep After**
to Never, 5, 15, 30, 60 or 120 minutes, and use **Preview on device**. Deckforge
uses its own logo by default. Images fit the full 800×480 display with black
borders. Files are copied into the session cache; moving the original file does
not break the saved screensaver. Image decoding runs outside the host lock and
native UI thread, and a newer configuration wins over an older import still loading.

The original **Sleep** action runs on key release. An empty device target means
all currently supported devices; an explicit target must match the connected
serial. The current host owns one Plus. Physical wake consumes the waking report
and queued reports, then restores the latest cached key and touch-strip images.
Plugins remain running and their display updates are retained while asleep.
Opening/focusing the native editor or using **Wake device** also wakes the deck.

The software timer follows device input and display/feature activity. It does
not use computer-wide keyboard/mouse idle. Custom images and GIFs use the software
screensaver path; the device worker temporarily suspends its firmware timer and
restores the captured or explicitly configured value in USB order. The device's
raw timer completed a separate 0→60→0-second hardware test with restoration readback.

Desktop lock detection uses read-only KDE or logind state. While locked, the
software screensaver displays its first frame and suppresses physical input;
unlock restores the profile. This is a Linux policy, not a claim of identical
Windows lock handling. Unknown lock status does not invent an unlock. KDE Wayland
cannot provide idle time through this D-Bus API; lock detection still works.
A locked shutdown retains the static image while restoring the firmware timer.

PNG/JPEG/GIF files are limited to 32 MiB input and 4096 pixels per dimension.
Decoder allocations and retained RGB frames each have a 128 MiB budget. GIF
frames preserve their decoded delays and disposal, loop continuously, and use
10 ms for zero-delay frames. Interactive image crop/zoom, native finite-loop
parity, standby-image installation, and the original no-custom-image firmware
fade-to-zero path remain open.

A full-LCD transfer and all nine profile-frame restoration transfers completed
on the connected Plus without transport errors. This is HID acceptance evidence;
physical appearance, physical wake gestures and real desktop lock/unlock still
need direct acceptance. See [protocol notes](docs/SCREEN_SAVER_PROTOCOL.md).

## Soundboard

Assign **Play audio** to a key, choose a file, and select Play/Stop, Overlap,
Restart, or Loop/Stop. Volume and one-to-five-second fades are configurable in
native controls. **Refresh outputs** lists real PipeWire/PulseAudio devices;
choose one or the desktop default. An unavailable explicit output produces an
error instead of silently using another device.

**Stop audio** and the native **Stop soundboard** button stop only Deckforge's
soundboard voices. Page changes and closing the editor preserve ongoing playback; quitting Deckforge stops
it. The Rust mixer streams mono/stereo audio at 48 kHz, using generic Rodio/Symphonia
codec libraries and a nonblocking native PulseAudio connection. It does not invoke
an external player. Mixing is limited to 32 voices, eight per action.

Wrapped profiles with `Resources/manifest.json` now resolve embedded media even
when the saved path points to another operating system. Playback leaves that path
intact. Exporting a newly selected audio file embeds a fresh resource; unchanged
imported media keeps its original bytes. The existing profile limits apply,
including 32 MiB per embedded asset. Original Windows output IDs are preserved;
`linuxOutput` records your explicit local mapping.

Generated WAV playback and an unchanged bundled MP3 are tested through a temporary
virtual audio sink, with captured PCM checks for volume and fades. This proves the
Linux output path; audible speaker/headphone and physical key acceptance remain
separate. Exact Windows fade curves/timing, multichannel downmix and hardware
playback/progress graphics remain open. Export acceptance in Windows is unverified.

## Multi Actions

Assign **Multi Action**, **Multi Action Switch**, or **Random Action** to a key.
The native editor adds, removes and reorders steps, edits each step's settings,
and opens compatible plugin inspectors. **Save sequence** applies the draft.
A switch has two sequences and advances after successful completion. Random
runs one selected step and permits repeated choices. Pressing a running sequence
again cancels it.

A step's virtual key hold and following gap each support 1–100 ms; explicit
Delay steps use milliseconds. Instance timing overrides plugin-manifest timing,
then the 100 ms defaults. Long waits do not block the editor. Leaving a page or
quitting cancels its running sequences and releases dispatched controls. Page-hide
cancellation is a Linux behavior pending an exact Windows page-transition trace.
Keyboard preparation finishes before timed steps begin.

Each child has a stable context, isolated settings and real `isInMultiAction`
plugin events. Desired state uses the original `OverrideState` field. Plugins
that declare `SupportedInMultiActions: false`, including the official Counter,
are excluded. The unchanged Hello World sample is tested inside sequences and is installed
alongside Counter and Layouts in the native action library.
Multi Actions inside other Multi Actions are rejected; unknown imported steps
stay preserved and block execution with a useful error.

Profiles retain nested settings and assets, including newly embedded audio.
Measured plugin events verify ordering, hold/gap timing, cancellation, switch and
random behavior. Physical shortcut timing, Windows export acceptance, and full
native workflow/hardware acceptance remain separate checks. A sequence dispatches
asynchronous actions in order; it cannot infer when an external application or
network operation has finished.

Sequence rows include live child images and temporary plugin warning/success
badges. These stay attached to each child when draft steps are reordered. See
[Multi Action feedback](docs/MULTI_ACTION_FEEDBACK.md) for parent progress, completion checks and their global preference.

## Dial Stacks

Assign **Dial Stack** to a dial, then add and order compatible dial actions in the
native editor. Press/release cycles through them. Rotation and touch operate the
selected action; a normal turn while pressed adjusts it without cycling. The
selected child keeps its own settings, feedback, layout and property inspector.
Opening an inactive child's plugin settings explicitly activates it first.

**Fast navigation** consumes held turns and commits the pending selection on
release, with the recovered 300 ms rotation suppression afterward. Linux honors
its saved setting; one inspected Windows loader resets it, and other restore
paths remain unverified. The optional stack mark uses the original overlay
rectangle with independently drawn artwork.

Only the selected child receives active plugin lifecycle events. Late inactive
updates are retained without replacing the current display. Native selection,
page changes, export/reimport and restart preserve child identities/settings.
Original Wave Link stacks are preserved with unavailable actions identified;
implementing the container does not make the Windows Wave Link plugin work.
The unchanged Layouts plugin is tested with multiple independent stack instances
and actual PNG comparisons. Physical dial-stack acceptance remains pending.

## Action Triggers

Assign **Action Trigger** to a dial to run key actions from counterclockwise turns,
physical presses, and clockwise turns. The editor keeps these three positions
fixed; empty slots stay empty. Each child has independent settings, images and a
property inspector. The touch strip shows three icons, each tappable within its
own rectangle. A physical press holds the middle action until release.

Rotation produces 60 ms key presses with 15 ms between repeats. An initial burst
is capped at three presses; more turns can queue three behind the current press.
Turning the other direction replaces pending work. Page changes, stack selection,
and edits cancel departing work with balanced releases. Overlapping physical and
touch presses share a held child until both release, a Linux ownership policy.

Action Trigger accepts ordinary key plugins even when they opt out of Multi
Actions; the unchanged Counter sample is tested. Automatic multi-state toggles
honor `DisableAutomaticStates`. Multi Actions can be assigned to these slots and
opened through **Edit sequence…**. Add Action Trigger inside a Dial Stack through
**Edit Action Trigger…**; stack pushes still cycle the stack, so touch activates
the middle key action there. **Go to page** uses the original one-based page number.

All eleven recovered original Action Trigger records round-trip in tests.
Simulation verifies plugin events and rendered images; physical acceptance of
these new gestures and Windows acceptance of exported profiles remain pending.

## Plugin resources

Plugins and native property inspectors can use `getResources` and `setResources`.
Bindings remain separate from settings, and action events include the resource
map. Updates replace the complete map; empty path values remove those bindings.
Notifications go to the other side: plugin changes reach its inspector, and
inspector changes reach its plugin. Request replies retain their IDs.

Profile export bundles referenced files. Import restores readable local paths
while preserving filenames and shared-file references. Different source files
with the same name remain separate. Nested actions, page/profile duplication,
explicit clears and restart preserve their resource bindings. File data is read
at export time; a missing file prevents export without overwriting an existing
archive. Each file is limited to 32 MiB and the complete profile to 128 MiB.

The resource conformance test calls unchanged official SDK 2.0.4 methods from a
generated plugin and runs native WebKit inspectors. There is no complete official
resource sample in the inspected sample repository. Windows import acceptance
and broader third-party file-picker workflows remain unverified.

## Plugin feedback and dial hints

`showOk` adds a green check to keys and `showAlert` adds a yellow warning triangle.
Both last three seconds; another request replaces the indicator and restarts its
timer. The underlying image/title can keep changing and remains after the effect
ends. Action Trigger children show their own miniature indicators.

Dial warnings follow the original app's separate behavior: a red tint over the
whole dial cell at 75% opacity for 165 ms, fading out over the next 165 ms. An
active Dial Stack child warns on its parent cell; inactive children cannot flash
it. `showOk` is a key-only SDK operation. Multi Action child-to-physical-parent
indicator routing remains unverified.

Plugins can update **Turn**, **Press**, **Tap**, and **Hold** hints using
`setTriggerDescription`. A sparse map hides omitted hints; an empty map restores
manifest defaults. These are runtime hints, separate from saved action settings.
Native redraw notifications update animations without faster idle polling, and
unchanged USB images are not retransmitted. Shutdown restores normal frames.

The unchanged official Image Resizer sample exercises both warning feedback and
successful resizing of a temporary image. It runs in its original sample checkout
with its Linux Sharp dependencies; this does not establish compatibility with
Windows-only native plugin binaries. Badge artwork is independently drawn;
pixel-perfect vendor artwork and new physical feedback acceptance remain open.

## Offline editing and USB recovery

The native editor opens even when the deck is absent or hidraw access fails. It
keeps profiles, visible actions and plugin processes available, displays the
connection error, and checks for the selected device every second. The last
selected serial and firmware are remembered. `--serial YOUR_SERIAL` selects a
different target; the host never switches to an arbitrary other connected deck.

A lost connection releases held buttons/shortcuts and clears pending physical
dial gestures. Already-started Multi Actions and timed Action Trigger presses
continue in the offline editor. Reconnecting the same device sends the SDK
connection event and restores the latest images without recreating visible action
instances. A newly identified device is announced before its action events.

Generated SDK tests verify lifecycle and state retention. Fake transport tests
cover read/write failures and replay of all nine current display images. A gated
probe also closed and reopened the actual Plus HID handle successfully, with zero
image writes during that probe. Physical cable removal, USB re-enumeration and
system suspend/resume remain separate acceptance checks.

## Commands

```sh
./target/debug/deckforge devices
./target/debug/deckforge observe --seconds 30
./target/debug/deckforge profile path/to/export.streamDeckProfile
./target/debug/deckforge install path/to/plugin.streamDeckPlugin --destination state/plugins
./target/debug/deckforge serve --plugin state/plugins/example.sdPlugin --profile path/to/export.streamDeckProfile
./target/debug/deckforge serve --simulate --plugin path/to/example.sdPlugin --state state/simulation --port 0
```

`--serial` selects a specific supported Stream Deck. `--model` selects its family or an offline layout. `--simulate` never opens USB.
`--seconds N` bounds a host session. `--port 0` chooses an available local port.
The CLI plugin installer only extracts a validated package; `serve --plugin` launches
its code under the current user. Node versions must match the plugin manifest;
the host checks `node`, `/usr/bin/node`, installed nvm runtimes, and a matching
`DECKFORGE_NODE20` or `DECKFORGE_NODE24` override. A plugin folder added through the
native editor is loaded in place and its path is retained across launches. `DECKFORGE_FONT` can select a TTF font.

Import keeps the original archive intact and adds the full profile to the library.
Use the profile/page selectors and Manage controls to create, rename, duplicate,
reorder or delete entries. Folder actions and Back navigate imported folders;
left/right touch-strip swipes change top-level pages. Pin an action to share it
across pages and folders; local assignments override the same pinned position.
The Pinned actions view lets you edit pins hidden by local overrides.

Export writes a `.streamDeckProfile` ZIP, preserving original assets and unknown
fields. Existing export files receive a backup before replacement. Round trips
through Deckforge are tested; acceptance by the Windows app is still unverified.
Unknown actions are retained and reported, but remain inactive.
Importing a built-in Windows action does not implement that action on Linux.

## Implemented and checked

| Area | Current evidence |
| --- | --- |
| USB | Real `0fd9:0084` Stream Deck+ opened; firmware `2.0.3.4` read |
| Hardware input | Live key transitions, dial press/release, signed rotation observed |
| Displays | Key JPEGs and touchscreen JPEGs written; user confirmed displays and controls work |
| Plugin runtime | Node samples work; native HTML helper passes generated protocol fixtures and registers the unchanged Windows-bundled tutorial |
| Plugin protocol | `willAppear`, `willDisappear`, key/dial events, settings/global settings, current-state title/image/state and basic feedback |
| Feedback | All six published built-ins and four official custom layouts exercised through the unchanged Layouts plugin; text, bars, gradients, images, SVG, clipping and incremental properties |
| Profiles | Full library, pages/folders/pins, original-format export/reimport; two shipped Plus trees retain all source fields/assets. Unchanged Lights Out bundled profile runs and changes SVG-backed images. |
| Persistence | Real sample settings survive host restart; imported background images tested for round-trip persistence |
| Native desktop editor | Installed React/TypeScript editor inside a native WebKit window, backed by Rust. Action assignment, live previews, typed settings, structured sequences, JSON, native file pickers and persistent drafts. Prior physical acceptance used the earlier egui editor. |

Touch and swipe packet decoding have tests. A touch routes `touchTap` to the
corresponding dial. The official feedback sample does not visibly react to touch;
this run does not claim full gesture behavior was physically verified.

## Remaining compatibility work

The full objective stays open. [FEATURE_PARITY.md](docs/FEATURE_PARITY.md) tracks
60 feature requirements against the shipped Windows 7.6 release. No sample-only
completion criterion replaces the desktop parity goal.

1. Broader HTML/property-inspector workflows, local file controls and real-world
   plugin compatibility beyond the tested samples and SDK fixtures.
2. User-authored image animation and native-reference visual validation of layout
   font/bar details. Separate hardware/editor targets and inactive-state updates
   are implemented; static WebP now joins SVG, PNG and JPEG in the shared decoder.
3. Windows acceptance of exported profiles, folder auto-exit, physical acceptance
   of Action Trigger, text/hotkey-switch actions, application activation, and automatic profile selection.
4. Native Windows/macOS plugin binaries and platform-specific dependencies.
   These do not become Linux programs simply by reproducing the WebSocket API.
5. Physical unplug/re-enumeration and suspend/resume acceptance, more device models,
   plugin supervision, tray/autostart, and broader long-running tests.

Unsupported messages/options are logged or rejected explicitly. Plugin success and
warning feedback is visible, and dial trigger descriptions appear in the native editor. `application.version` in plugin
registration advertises the 7.6 protocol target, not complete 7.6 compatibility;
`application.platform` is honestly `linux`, which some plugins will not accept.

## Validation

```sh
cargo test --locked
cargo test --locked -- --include-ignored
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
python tests/sample_interop.py
python tests/layout_interop.py
python tests/webview_interop.py
python tests/native_lifecycle.py
python tests/profile_interop.py
python tests/bundled_profiles_interop.py
python tests/system_interop.py
python tests/soundboard_interop.py --binary target/release/deckforge
python tests/multi_interop.py --binary target/release/deckforge
python tests/stack_interop.py --binary target/release/deckforge
python tests/volume_interop.py --binary target/release/deckforge
python tests/trigger_interop.py --binary target/release/deckforge
python tests/resources_interop.py --binary target/release/deckforge
python tests/feedback_interop.py --binary target/release/deckforge
python tests/device_interop.py --binary target/release/deckforge
# After building the unchanged official Image Resizer fixture:
python tests/feedback_interop.py --binary target/release/deckforge --official-image-resizer
```

The optional installer test needs the local Windows artifacts. The sample
interop test uses real official plugin code and isolated temporary state, then
checks counter events, dial feedback, rejected requests and restart persistence.
It never accesses USB. Official sample tests skip when their external fixtures
are absent; they are not replacements for unit tests.

## Sources and provenance

- [Elgato general HID protocol](https://docs.elgato.com/streamdeck/hid/general/)
- [Stream Deck+ HID protocol](https://docs.elgato.com/streamdeck/hid/stream-deck-plus/)
- [Elgato plugin WebSocket contract](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/)
- [Plugin manifest](https://docs.elgato.com/streamdeck/sdk/references/manifest/)
- [Official samples](https://github.com/elgatosf/streamdeck-plugin-samples), pinned
  to `8cc82bad8716c000bacc131285ffcf6805dafff9`. Counter source built unchanged;
  Layouts uses its official prebuilt archive. Sample code is a compatibility
  fixture, not copied into this application's implementation.
- [Windows 7.6 release](https://help.elgato.com/hc/en-us/articles/51156609278737-Elgato-Stream-Deck-7-6-Release-Notes),
  MSI `Stream_Deck_7.6.0.23012.msi`. REA/Ghidra outputs and reproducible artifact
  observations are in `../evidence/elgato-7.6/`, especially `findings.md`.

The cloned REA repository supplies the investigation workflow. Its checkout
remains unchanged. The version-pinned published REA 6.4.0 CLI performed the local
PE-resource and Ghidra inspections because the checkout's dependency lockfile
uses a remote mirror rejected by this machine's npm policy. No policy was changed.
Static evidence is kept separate from Linux runtime and user hardware evidence.

Application source is MIT licensed. External official samples and Windows
artifacts retain their respective licensing and are kept outside the application.

## Native verification, 2026-10-11

The optimized installed executable was rendered on this Wayland desktop with
its own screenshot event, then closed normally. The native capture showed both
real official plugins connected in simulation; child shutdown was verified.
A system-only PATH capture also verified discovery of the sample's required
Node 20 runtime without shell startup scripts.

The live native app then opened the physical Stream Deck+, retained the user's
saved settings, and ran both plugins. The user explicitly confirmed that editing
incrementBy in the native window changed the physical key's behavior.
27 tests, strict Clippy, formatting, and unchanged-sample interoperability passed.
Artifacts are in `../evidence/native/`. This proves the named native workflow,
not full desktop parity or every control/OS integration.

## HTML and layout verification

Native inspector file inputs now return the selected file's full Linux path,
including unchanged `sdpi-file` controls. Multiple selection, File reading,
cancel/reset and special-character filenames have native WebKit test coverage.
See [file controls and remaining limits](docs/PLUGIN_FILE_CONTROLS.md).

The native WebKit helper runs HTML backends hidden and opens property inspectors
as native windows. Inspector identities are bound to one action. Settings/global
settings and plugin/inspector messages route in both directions; malformed
contexts cannot write another action's settings. Closing/replacing an action
cleans up its inspector. Failed unregistered helpers are reaped and surfaced in
the activity log. SIGINT/SIGTERM now close the native editor as well as its host.

The unchanged Counter inspector registers directly. A separate, explicitly
instrumented fixture loads its original HTML unchanged in a same-origin srcdoc
frame, edits the real sdpi control and proves the Counter consumes the setting.
This is not a manual-click claim. The shipped tutorial's registration is tested;
its complete onboarding workflow is still open.

All six built-ins (`$X1`, `$A0`, `$A1`, `$B1`, `$B2`, `$C1`) and four custom layouts
run through the unchanged official Layouts plugin. PNG content, SVG icons,
gradients, independent bars and persisted feedback are checked. The official
`$C1` sample references an absent `imgs/actions/layout/layout.svg`; that item is
blank with a diagnostic, while its indicators continue working. Original sample
files were not repaired or replaced to make the test pass.

The renderer follows documented geometry and inspected official example output.
Exact native antialiasing, all font weights and every bar edge are not proven
pixel-identical to the Windows renderer. Full desktop parity remains active in
the feature checklist.

Verification update, 2026-10-12: 50 unit/integration-boundary tests and strict
Clippy pass. The optimized binary also passes all four process-level suites.
The HTML suite now proves generated popup child-document execution and preserved
window.opener communication; exact tutorial page visuals are still not established.
Native SIGTERM/SIGINT shutdown and session reuse are tested in actual windows.
Evidence for this update is retained in `../evidence/html-layout/`.

### Native background lifecycle checks

`python tests/window_interop.py --binary target/release/deckforge` uses isolated
simulation state and a generated plugin to verify background operation, window
recreation, same-process relaunch, plugin continuity, explicit quit and signal
cleanup. `python tests/tray_interop.py --binary target/release/deckforge` checks
an available session-bus watcher and invokes only the test-owned tray item's
Activate/Open/Quit methods. Neither test injects desktop input or opens USB.
The tray test needs a running StatusNotifier watcher and `busctl`. The window
test needs the separately obtained official Counter sample dependencies.
Native title-bar input and compositor focus still need direct desktop acceptance.

`python tests/startup_interop.py --binary target/release/deckforge` validates the
startup entry and launches it through GIO with an isolated configuration and
paths containing spaces, quotes and other reserved characters. It checks the
exact state/simulation arguments, background startup, quit and disable behavior.
The test requires `gio` and `desktop-file-validate`; it does not change the real
user's startup preference or perform a desktop logout/login. Actual next-login
acceptance remains a separate check.

`python tests/brightness_interop.py --binary target/release/deckforge` covers the
real simulated host's key modes, held-repeat timing, dial adjustment, rendered
feedback, nested action cancellation, per-device settings and restart. The
explicitly gated hardware sleep tests require exclusive device ownership and a
selected serial; ordinary test runs do not write settings to a physical device.
The protocol implementation follows [Elgato's HID reference](https://docs.elgato.com/streamdeck/hid/general/).

`python tests/sleep_interop.py --binary target/release/deckforge` verifies static
and animated frames, copied-asset persistence, wake input consumption, plugin
continuity, lock simulation, cancellation and stale-import rejection. Timer
boundaries use deterministic Rust tests; this suite does not wait five real
minutes or lock the desktop. `DECKFORGE_TRACE_HID=1` logs accepted image-transfer
metadata for an explicitly run host; it never logs image bytes.
