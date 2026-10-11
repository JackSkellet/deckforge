# Deckforge

An independent Linux Stream Deck+ desktop application written from scratch in
Rust. The native editor uses egui/eframe and calls the Rust host directly in the
same process. No browser is required. A separate headless diagnostic mode retains
the earlier local web editor. No existing Stream Deck host, driver implementation,
or application template is used.
Generic USB, image, font, HTTP, WebSocket and archive libraries are dependencies.

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
Advanced JSON editing remains available. Draft changes survive periodic refresh
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

Import is read-only: original archives stay intact. All profile pages and raw
action data remain available to the parser, but the host currently activates
only the selected current page. Unknown actions are reported and remain inactive.
Importing a built-in Windows action does not implement that action on Linux.

## Implemented and checked

| Area | Current evidence |
| --- | --- |
| USB | Real `0fd9:0084` Stream Deck+ opened; firmware `2.0.3.4` read |
| Hardware input | Live key transitions, dial press/release, signed rotation observed |
| Displays | Key JPEGs and touchscreen JPEGs written; user confirmed displays and controls work |
| Plugin runtime | Unchanged official Node Counter and Layouts feedback plugins register and respond |
| Plugin protocol | `willAppear`, `willDisappear`, key/dial events, settings/global settings, current-state title/image/state and basic feedback |
| Feedback | `$B1` title/value/indicator subset; not a general layout engine |
| Profiles | Six official sample archives and two Windows-bundled Plus profiles; current/default pages, encoded UUID folders, settings, states and images |
| Persistence | Real sample settings survive host restart; imported background images tested for round-trip persistence |
| Native desktop editor | Installed optimized Rust window; action assignment, input testing, live previews, typed settings, JSON, title/removal, native file pickers and status. User confirmed native setting edits affect physical presses. |

Touch and swipe packet decoding have tests. A touch routes `touchTap` to the
corresponding dial. The official feedback sample does not visibly react to touch;
this run does not claim full gesture behavior was physically verified.

## Remaining compatibility work

The full objective stays open. [FEATURE_PARITY.md](docs/FEATURE_PARITY.md) tracks
60 feature requirements against the shipped Windows 7.6 release. No sample-only
completion criterion replaces the desktop parity goal.

1. HTML plugin runtimes and plugin property inspectors. The Windows installer
   contains an actual Elgato tutorial HTML plugin that provides a concrete target.
2. General custom/built-in feedback layouts, separate hardware/editor targets,
   inactive-state image/title updates, alerts and richer image formats such as SVG.
3. Profile page/folder switching, exporting profiles, multi-actions, built-in
   hotkeys/audio/application actions, and automatic profile selection.
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
