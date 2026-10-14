# Smart Profiles on Linux

Smart Profiles are implemented for the native editor on KDE Plasma6. The current
end-to-end acceptance uses a KDE Wayland session and simulated Stream Deck
hardware. Other desktop backends and physical-device acceptance remain open.

Open **Profile options → Smart Profiles…**, enable automatic switching, choose
an application for each profile and optionally select a default profile. Hide or
minimize the editor to activate the rules. Switching pauses while the editor or a
plugin inspector is open; Quit also holds routing during shutdown.

Installed application choices use their declared StartupWMClass when present,
otherwise their desktop ID. Recently focused applications offer observed desktop
IDs, executable paths and window classes. Advanced identity fields allow literal
matching; no shell, command arguments, wildcard expansion or foreign-name guesses
are used. Some applications report no desktop ID. Shared runtimes/interpreters
may need a desktop ID or window class instead of an executable match.

Installed applications use the shared searchable icon picker from Open Application.
Radio rows show desktop IDs to distinguish similar names, with keyboard selection,
incremental results and lazy thumbnails. Absolute and theme icons are rendered as
bounded PNGs; missing artwork uses a letter fallback. Native icon/selection/restart
acceptance: `target/application-icons/native/`. Recently observed identities remain
text choices because a process identity need not identify one installed launcher.

## Routing and persistence

Rules are stored in the private session's `smart_profiles` map, keyed by stable
device scope so serial adoption or device renaming does not discard them. Targets
must belong to that device. Bindings for inactive layouts are retained, but only
profiles compatible with the current device model can activate. Forgetting a
device removes its rules transactionally. Missing/deleted targets are ignored.

The first matching compatible profile in sidebar order wins. Exact duplicate
bindings show a warning. An unmatched application selects the configured default,
or leaves the current profile alone when no default is set. A manual selection
is retained until the observed application identity changes; PID changes alone do
not reassert a rule. Layout changes also cause reevaluation. Backend errors retain
the current profile rather than being interpreted as an unmatched application.

Profile changes preserve page/folder cursors. The replacement graph is saved
before lifecycle refresh; a save failure restores the prior graph and visible
slots. Scoped refresh balances releases/disappearance/appearance. Configuration
saves use an expected baseline and roll back on persistence failure. The settings
dialog keeps its original device owner if the selected editor device changes.
Draft dialog state survives native window hide/show in the same process. A
successful save clears the retained saved dialog, including when its old WebView
has gone away. Unsaved drafts are not promised across a full process quit.

Imported native `AppIdentifier` values remain intact. A unique imported `"*"`
marker is suggested as the initial fallback when no local rules exist. Duplicating
a profile or copying it to another
compatible device includes an independent copy of its Linux application binding.
The destination keeps its automatic-switching flag, default profile and existing
bindings. The copied profile activates manually there without changing the editor
device. Profile, navigation and binding are saved together; a failed save leaves
the previous state and held actions intact.

## Profile archive bindings

Profile export writes the current Linux binding into
`<profile>.sdProfile/Deckforge/SmartProfile.json` (under `Profiles/` for wrapped
archives). The version1 object contains only `version` and `binding`; the latter
has `kind`, `value` and `name`, using the same validation as the settings dialog.
This is a Deckforge extension, not a recovered Elgato field. Native manifests,
AppIdentifier and unrelated source files are retained. Clearing a binding removes
its metadata on the next export, including metadata retained in the source ZIP.

Explicit profile imports restore the binding under the new local profile ID.
Automatic switching, the default profile and other existing bindings retain their
destination settings; a fresh session stays disabled. Bindings retain their exact
Linux identity, so the destination application must report the same identity.
Executable paths are not translated between machines. Profile, navigation and
binding commit together before actions change. A failed save also removes its
uncommitted owned source archive. Profiles for an inactive model retain their
binding without changing the active controls.

Invalid, oversized or unsupported-version metadata rejects the import before any
profile/source is published, with an error. Metadata is limited to16KiB and rejects
unknown fields. Archives without the extension use the existing import behavior.
Plugin-bundled automatic profiles do not install Linux associations. Acceptance
of this extension in the official Windows/macOS applications remains unverified.

## Focus observer and ownership

`src/focus_monitor.rs` subscribes to KWin's `windowActivated` and metadata signals.
It reports initial focus and the compositor's minimized state for the editor's
own windows. This complements the toolkit state on Wayland. An atomic window
state guard pauses routing immediately on show/restore without taking the host
mutex. The host coalesces observations on its250ms monitoring loop.

Reports contain PID, desktop ID, window class and a readable current-user
`/proc/PID/exe` path. Titles and contents are never collected. Invalid/non-UTF8
executable paths are unavailable; Linux's deleted-image annotation is stripped
only when the mapped inode has no links. A kernel-backed test verifies that an
actual filename ending in ` (deleted)` is not mistakenly truncated. At most32
recent application records are retained in memory, not session history.

The observer starts only for enabled device rules or an explicit editor preview.
Each state uses a deterministic private script name derived from its canonical
path while the host owns its session lock. Startup unloads only that owned name
before installing a new observer. The generated script is a mode600 managed file
inside the state directory. Normal shutdown unloads the script and removes the
file; a new owner recovers the old script/file after a crash. Observer retirement
is nonblocking in the host; finished workers are reaped, and shutdown joins them.

Session-bus calls and initial reports have two-second bounds. A KWin owner change
causes reconnect. Error snapshots may contain last-known focus; routing always
checks the error before using it. Physical KWin restart acceptance is not claimed.

## Original contracts and differences

The supplied Stream Deck7.6 Windows AppExecutable has SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
`0x1403d75b0` reads optional AppIdentifier; `0x1403dc3a0` serializes it.
`0x14029a670` recognizes `"*"` as the default-profile marker and distinguishes the
first matching profile in duplicate warnings. Its underlying matching-list order
is not established; sidebar order is an explicit Linux rule, not proven identical
Windows ordering. String `0x14165f390` describes switching while the app window is
closed. Evidence: `../evidence/smart-profiles/rea-bundle.json`.

The official [Smart Profiles guide](https://help.elgato.com/hc/en-us/articles/360053419071-Elgato-Stream-Deck-Smart-Profiles)
and [newer guide](https://www.elgato.com/ca/en/explorer/products/stream-deck/smart-profiles-stream-deck/)
describe suspending switches while editing and resuming when minimized. The focus
observer uses the documented [KWin API](https://develop.kde.org/docs/plasma/kwin/api/).

## Verification

Unit tests cover device/model ownership, exact rules, duplicate order, fallback,
visible/minimized guards, manual override, cursor retention, inactive-layout
binding preservation, imported metadata, stale configuration and persistence
failure. Frontend tests cover installed/observed identity choices and captured
ownership. Window-state and device-forget regressions are included.

`tests/smart_profiles_interop.py` configures rules through real WebKit, focuses
only an owned GTK fixture and verifies visible pause, actual KWin minimize/hide
routing, manual override, default fallback and restart. It deliberately kills an
owned simulated background host, restarts the same private state and verifies
owned-script recovery plus final script/file cleanup. No USB or real user profile
is changed. Reports/capture: `target/smart-profiles/native/`.

That test exposed a hidden-editor focus bug: GTK surface destruction was queued
after the event loop stopped, leaving KWin focused on the supposedly closed
editor. Flushing GDK after destruction fixed the real fallback failure. The
separate native file-open/draft test checks this lifecycle interaction too.

The read-only `focus-probe --seconds N [--events]` diagnostic remains available.
`tests/native_smart_copy_interop.py` verifies the real duplicate and device-copy
dialogs, independent bindings, destination preferences and restart. Copied cropped
artwork survives removal of its original source and exports with exact image
bytes. It also exports, deletes the original session and image, imports into a
fresh native session via desktop file opening, restarts and re-exports. Reports/
captures: `target/smart-archive/native/`. Unit tests cover plain/wrapped archives,
cleared bindings, invalid metadata, rollback with held actions, inactive layouts
and copying to a previously unconfigured destination.

Non-KDE backends, sidebar application icons, official-app archive acceptance,
original duplicate-priority fidelity, legacy/diagnostic editor behavior and
Windows/macOS acceptance remain incomplete. Full PR-08 and overall parity are
not complete.
