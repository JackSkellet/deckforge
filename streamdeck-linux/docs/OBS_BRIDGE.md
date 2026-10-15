# Plugin setup and native Linux OBS

Open **Plugins > Configure** beside a plugin. Choose a settings page if the
plugin supplies more than one, then **Open plugin settings**. This uses the
plugin's own property inspector and works before assigning a key.

Shared settings, including authentication saved with the Stream Deck global
settings API, persist normally. The setup action exists only while its window is
open. Its action-specific settings are temporary. Configure an assigned key to
save selections such as its OBS scene. Setup never inserts a key or sends a key
press, and render/feedback messages for the temporary context never reach hardware.
Plugins can also save their own private data as usual. A plugin without a property
inspector has no setup page to open. This does not make Windows/macOS executables
Linux-compatible.

## OBS

The Elgato OBS Studio plugin uses the `streamdeck-obs` protocol and an OBS-side
module. OBS's standard WebSocket server on port 4455 is a different protocol.
The Plugins row's Connected label means the plugin has registered with Deckforge;
it does not indicate an OBS connection. Deckforge maps this plugin's declared
`obs64.exe` and `com.obsproject.obs-studio` subscriptions to the native `obs`
executable, so it receives the launch event needed to start connecting. Explicit
user mappings, including an empty mapping, take precedence. For a custom OBS
executable, edit the plugin's **Monitored applications** mappings.

For native OBS, open **Plugins > OBS Studio > Configure > Install OBS bridge**.
Fully quit and reopen OBS when convenient, then use **Check OBS connection**.
Installation never starts, stops, restarts or changes scenes in OBS. The check
performs a read-only version exchange with the bridge over loopback.
The bridge itself provides scene, source, audio, recording, streaming and other
OBS operations to the existing Elgato plugin. Only connection, scene enumeration,
scene switching and restoration have been validated in the isolated integration
test; this is not a claim of testing every OBS action or hardware control.

The native module installs per user under
`$XDG_CONFIG_HOME/obs-studio/plugins/StreamDeckPlugin`, defaulting to
`~/.config/obs-studio/plugins/StreamDeckPlugin`. Pre-existing unmanaged or modified
installations are preserved. Reinstalling the same managed build is harmless.
A different installed build is left untouched and reported for manual removal.
Remove only this directory to uninstall this bridge while OBS is closed.

Flatpak OBS needs a bridge built for its runtime. The native installer does not
modify it. The [Linux bridge project](https://github.com/LinuxGamesTV/streamdeck-obs-linux-plugin2)
provides a separate Flatpak route; it has not been validated here.

## Building and distribution

Requirements: Git, CMake, Ninja, a C++17 compiler, native OBS development headers
and CMake packages, and Qt6 Core/Widgets development files.

```sh
python scripts/build-obs-bridge.py
python scripts/install-desktop.py --binary target/release/deckforge --without-bundled-plugins
```

The first command builds `target/release/obs-bridge`. The desktop installer stages
that package under the user's Deckforge data directory, making **Install OBS
bridge** available. For a command-line install/check, use `deckforge obs-bridge
--install` or `deckforge obs-bridge`. Neither starts OBS.

To include it in a distribution, pass `--with-obs-bridge` to
`scripts/build-distribution.py`. It is optional because OBS/Qt ABI compatibility
is distribution-dependent; rebuild against the destination's native OBS/Qt when
needed. Compilation here used OBS 32.2.1 and Qt 6.12 on x86_64 Linux.

The module is built from [Elgato's GPL-3.0-or-later source](https://github.com/elgatosf/streamdeck-obs-plugin2)
at commit `fcddcde567bdeafd235829d4fe8519894b158e29`, version 5.5.4.
The Linux CMake wrapper in `scripts/obs-bridge` replaces missing upstream build
files and adjusts the optional frontend-library probe to `libobs-frontend-api.so`.
All other source and protocol handlers are upstream. The upstream checkout is
kept clean; the adjusted translation unit is generated in the build directory.
The bridge package carries its licenses, pinned build metadata, checksum, and
complete corresponding source including submodules and the Linux build wrapper.
The main Deckforge host remains separately licensed under MIT.

## Validation

`tests/plugin_setup_interop.py` exercises real native HTML plugin/inspector
helpers, bidirectional messages, global persistence and temporary settings with
no assigned keys. `tests/obs_bridge_interop.py` uses private OBS configuration,
Xvfb and an isolated network namespace. It runs the installed official OBS
plugin's unchanged JavaScript protocol classes with browser-compatible Node
WebSocket events and ordinary Node timers. It also checks Deckforge's bridge
status request. The optional `--reference-app PATH` argument also runs the installed protected
OBS plugin and opens its original Scene inspector through the real Plugins menu,
using only an empty temporary Deckforge session. The reference is read through
the normal authenticated manifest reader. This does not validate account login
for unrelated plugins.

Run the OBS test with `python tests/obs_bridge_interop.py`; it requires Linux user
and network namespaces, `ip`, native OBS, Node with `--experimental-websocket`,
the built bridge and the installed OBS plugin's source files. Its Xvfb executable
comes from `target/x11-layouts/tools/usr/bin/Xvfb`, as used by the native drag tests.
No connection to the user's running OBS instance is possible inside that network
namespace. The test changes and restores only its two private empty scenes.
