# Deckforge

An independent Linux Stream Deck+ desktop application written from scratch in
Rust. The native editor uses egui/eframe and calls the Rust host directly in the
same process. Plugin HTML runs in isolated native WebKit windows. A separate headless diagnostic mode retains
the earlier local web editor. No existing Stream Deck host, driver implementation,
or application template is used.
Generic USB, image, font, HTTP, WebSocket, archive and native browser libraries
are dependencies. Linux HTML support uses system GTK3 and WebKitGTK4.1 libraries.

This is a working early compatibility build, **not a complete port of Elgato's
application**. Its first targets are the unchanged official Counter and Layouts
feedback sample plugins. Elgato's Windows executable is an analysis input, not a
runtime dependency.

## Run the native application

Open **Deckforge** from the Linux application menu, or run:

```sh
~/.local/bin/deckforge
```

The installed optimized executable is `~/.local/bin/deckforge`. Its desktop entry
is `~/.local/share/applications/deckforge.desktop`. Default application data lives
in `$XDG_DATA_HOME/deckforge`, or `~/.local/share/deckforge` when unset. Your earlier
workspace session was copied into this directory with its sample plugins;
`state/live/session.json` remains intact as the original.

Select a key or dial in the native window, choose an action, and assign it. Edit
simple settings with text, numeric and checkbox controls, then Save changes.
Use **Plugin settings…** to open an action's original HTML property inspector
when its manifest provides one. Advanced JSON editing remains available. Draft changes survive periodic refresh
and switching between controls. The native window also has plugin/profile file
pickers, title editing, removal and input testing. Closing the window currently
stops this host and its plugin processes; tray/background mode remains unfinished.

Build and install an updated native executable:

```sh
cd '/home/jack/Documents/ChatGPT/REA Workspace/streamdeck-linux'
cargo build --release --locked
python scripts/install-desktop.py
```

The per-user installer changes only its executable, icon and launcher; it keeps
replaced files as `.previous`. It does not install system packages, a startup
service or udev rules. It leaves application settings unchanged. The connected
device already grants this user access. No device reset or firmware write is used.

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
and press/tap to mute. Volume state feedback is not implemented yet.

The host needs write access to `/dev/uinput` for keyboard/media actions. It does
not change permissions automatically. Qt shortcut identities are translated to
Linux physical key codes; arbitrary keyboard-layout equivalence and Coalesce
behavior remain unverified. The kernel device was created/removed in testing,
but event readback could not run with current event-node permissions. No desktop
shortcut or real audio change is claimed as verified by the automated tests.

The latest action result appears in the native window. Network work runs outside
the editor and input worker. `--simulate` prevents USB access only: configured
System actions and plugins can still perform their normal OS/network operations.
Close/Open Application activation, text entry, hotkey switch and other desktop
actions remain on the parity ledger.

## Soundboard

Assign **Play audio** to a key, choose a file, and select Play/Stop, Overlap,
Restart, or Loop/Stop. Volume and one-to-five-second fades are configurable in
native controls. **Refresh outputs** lists real PipeWire/PulseAudio devices;
choose one or the desktop default. An unavailable explicit output produces an
error instead of silently using another device.

**Stop audio** and the native **Stop soundboard** button stop only Deckforge's
soundboard voices. Page changes preserve ongoing playback; closing the app stops
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

## Commands

```sh
./target/debug/deckforge devices
./target/debug/deckforge observe --seconds 30
./target/debug/deckforge profile path/to/export.streamDeckProfile
./target/debug/deckforge install path/to/plugin.streamDeckPlugin --destination state/plugins
./target/debug/deckforge serve --plugin state/plugins/example.sdPlugin --profile path/to/export.streamDeckProfile
./target/debug/deckforge serve --simulate --plugin path/to/example.sdPlugin --state state/simulation --port 0
```

`--serial` selects a specific Stream Deck+. `--simulate` never opens USB.
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
| Native desktop editor | Installed optimized Rust window; action assignment, input testing, live previews, typed settings, JSON, title/removal, native file pickers and status. User confirmed native setting edits affect physical presses. |

Touch and swipe packet decoding have tests. A touch routes `touchTap` to the
corresponding dial. The official feedback sample does not visibly react to touch;
this run does not claim full gesture behavior was physically verified.

## Remaining compatibility work

The full objective stays open. [FEATURE_PARITY.md](docs/FEATURE_PARITY.md) tracks
60 feature requirements against the shipped Windows 7.6 release. No sample-only
completion criterion replaces the desktop parity goal.

1. Broader HTML/property-inspector workflows, embedded resources, local file
   controls and real-world plugin compatibility beyond the tested samples.
2. Separate hardware/editor targets, inactive-state image/title updates, alerts,
   animation, and native-reference visual validation of layout font/bar details.
3. Windows acceptance of exported profiles, folder auto-exit, multi-actions,
   text/hotkey-switch/audio actions, application activation, and automatic profile selection.
4. Native Windows/macOS plugin binaries and platform-specific dependencies.
   These do not become Linux programs simply by reproducing the WebSocket API.
5. USB reconnect recovery, more device models, plugin supervision, tray/autostart,
   and broader long-running tests. Restart the host after a USB disconnect.

Unsupported messages/options are logged or rejected explicitly. `showOk` and
`showAlert` are currently logged without displaying overlays; trigger-description
updates are accepted but not shown in the editor. `application.version` in plugin
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
