# Plugin compatibility checkpoints

Deckforge keeps plugins that fail to start visible in the native plugin manager.
A bad plugin does not prevent the editor or other plugins from starting. Restart
reloads one plugin while retaining configured actions and settings. It does not
convert Windows or macOS binaries into Linux executables.

## Discord 2.4.0.123

The user supplied `discord-2.4.0.123-mac_windows.streamDeckPlugin` on 12 October
2026 as a real compatibility target. Package SHA-256:
`9d063e9b3616693093e565f762cdc65bd5638bdbb9f42e535e1c738b429302cd`.

The ZIP contains `com.elgato.discord.sdPlugin`, Windows `ESDDiscord.exe`, macOS
`ESDDiscord`, and HTML/JavaScript property inspectors. Its 32,512-byte manifest
starts with an `ELGATO` binary header rather than JSON. Manifest SHA-256:
`b12534e6177cd673e60d58652f29194a03acdd9d85b44c5d204554646b4b6583`.

Elgato's [distribution documentation](https://docs.elgato.com/streamdeck/sdk/introduction/distribution/)
describes Marketplace DRM, manifest encryption and integrity checking. The new
Rust reader authenticates and decrypts this manifest using a locally supplied,
hash-pinned Stream Deck 7.6.0.23012 executable as its reference. It verifies the
trusted signer, application identity, Ed25519 signature and AES-GCM tags. No vendor
keys or decrypted manifest files are embedded in or written by Deckforge.

The unchanged package's authenticated manifest SHA-256 is
`038c015493d11a665e7c561b993afc6f6933ad9db0ec926e1d0c6e50c33bae25`.
It declares 17 actions and 179 file-integrity records. Required-file integrity uses the recovered BLAKE3 contract and is checked on
installation and loading. See [protected packages](PROTECTED_PACKAGES.md) for
static reference import and the distinction between verification and execution.

At the user's request, Discord execution uses a new [native Rust plugin](../plugins/discord/README.md).
It preserves the original action/settings identities and supplies its own settings
panel and artwork. No Wine or original Windows/macOS backend is used. The native
replacement has host/process and mock-RPC evidence. A user-authorized live check
completed consent, token exchange, RPC authentication and a voice-settings read.
Physical action acceptance remains open. An empty Discord application redirect
list caused the reported RPC 5000 error; registering a redirect and saving the
same value in the plugin resolved that authorization failure.

## Home Assistant 3.8.4.66

The unchanged `de.perdoctus.streamdeck.homeassistant` HTML backend and property
inspector register in the native Linux helper. Its entry pages use JavaScript
modules. Loading them as `file://` documents prevented the modules from defining
`connectElgatoStreamDeckSocket`, so the helper exited with status 1.

HTML helpers now serve the authorized plugin folder through a dedicated
`deckforge-plugin` origin. Local module imports, dynamic imports and JSON fetches
work with MIME-typed responses. WebKit enables CORS requests for this custom
scheme before loading the document. This does not disable browser security or
grant cross-origin network access. Asset reads remain inside the plugin root;
external symlinks, traversal, special files and files over 32 MiB are rejected.

The helper also waits up to ten seconds for the connection callback, because a
module graph or framework mount can complete after the main-document load
notification. Registration runs once and only on the original entry document.
Plugin files and their integrity records remain unchanged.

`tests/html_module_interop.py` reproduces the old failure and checks module
imports, fetch, delayed callback readiness, bounded missing-callback failure,
and optionally the original Home Assistant backend and inspector. A full
isolated host check also loaded the protected manifest, assigned an action and
connected both runtimes with empty settings. No Home Assistant server,
credentials or physical entity controls were used. Live server authorization
and action behavior remain unverified.

The existing native HTML contract, popup, official Counter inspector and
unchanged shipped tutorial checks also pass with scoped asset loading.

## Bluetooth Buddy 1.0.0.0 and Bluetooth for Linux

On 14 October 2026 the installed original backend registered with Deckforge,
but its log showed `fk-bt-helper.exe EACCES`. The shipped helper is a Windows
PE executable. The JavaScript backend starts that executable unconditionally,
so a connected plugin socket does not establish Bluetooth functionality.
REA static evidence `ev_32c8c0d9f103de4e633cfee6b7c089797a0bdfafa859678cb5baf8162bd0940d`
has partial graph coverage; the startup log and helper file type independently
establish this failure.

At the user's request, [Bluetooth for Linux](../plugins/bluetooth-compat/README.md)
replaces the package with independent Node/Python code, a native settings panel,
and original artwork. The four original action identities and their device/label
settings are retained. BlueZ supplies paired devices, connection state, radio
control and battery percentages when available. No Wine or vendor backend is
used. The guarded installer retains the complete original package for rollback.

Thirteen helper/installer tests and a real simulated-host integration test cover
commands, failures, retries, device removal, stored settings, and owned child
cleanup. All four settings panels pass actual native WebKit interaction and
layout checks. Live read-only BlueZ checks found three paired devices and one
connection; no device exposed a battery percentage. A real host running against
BlueZ rendered the battery dial with an explicit unavailable value. These checks
do not prove physical connect/disconnect, radio switching or battery reporting
from another device. The vendor's custom color editor and animations are not
part of this replacement. The installed replacement registered on the live
Stream Deck Plus host after restart, with profiles and settings preserved.

## Spotify 2.5.0.86 and Spotify for Linux

The original JavaScript crashes during startup because its data-directory
lookup only defines Windows and macOS. Its player-controller router also rejects
Linux. REA static evidence
`ev_0948c85a509259f419e8c0898a25a3d353a3b9c1421f1a84bd3dbf92ea3e4a17`
has partial graph coverage; the observed startup exception confirms the first
failure.

[Spotify for Linux](../plugins/spotify-linux/README.md) is an explicitly labelled
local derivative of the user's package. Its installer authenticates the original
manifest and required files, retains the original directory, and adds an MPRIS
controller. All 15 original action declarations and property inspectors remain.
The repository distributes only the adaptation, not the vendor bundle.
Deckforge's protected-package checks are unchanged.

A real private D-Bus fixture verifies local playback, volume, shuffle, repeat,
seek, metadata and URI controls. The original Playback action reaches that
fixture through actual key events in the real USB-simulated host. Live read-only
checks found Spotify running, stopped, without a current track. Actual music
playback, account sign-in and cloud library/queue/Connect features remain
unverified. The latter retain the original Spotify account/API requirements. The installed
derivative also registered successfully on the live host after restart.

## ChatGPT & Codex 1.0.0.8

The original backend registered but rejected Linux in its desktop Unix-socket
endpoint and Codex CLI discovery. The installed [Linux adaptation](../plugins/codex-linux/README.md)
preserves socket ownership/permission checks and authenticates the original
package before staging a local derivative. It adds Unix-socket selection, verified
CLI discovery and xdg-open protocol launching.

The shipped transport completed a real initialize handshake. Activity, Usage and
Health actions rendered in an isolated host and a complete 217-chat catalogue
arrived from the running desktop app. History negotiation fell back successfully
from protocol version 2 to version 1. No prompt/approval/task mutations were sent.
Voice/dictation state was reported unavailable and remains a separate platform
boundary. REA JavaScript analysis was partial, with evidence
`ev_3db7c51aae66f615022b84d3863c21cf781042a95ca3ad4c6df65619fd3fd589`.

## Remaining runtime work

- More protected-package/reference versions beyond the verified 7.6 contract.
- Windows/macOS executable plugins and OS-specific native dependencies.
- Broader runtime/version selection and updates. Portable generic scripts and unambiguous Windows/macOS script entries now work, including `.cjs`; Node paths are searched deterministically.
- Automatic Marketplace update discovery. [Account sign-in and browser install handoff](MARKETPLACE.md) are implemented using the website session; saved-session reuse after restart and the real Twitch Creator Tools download/install/registration have been verified on this host. Managed package replacement/rollback, persistent Enable/Disable and explicit removal/reinstall are implemented.
