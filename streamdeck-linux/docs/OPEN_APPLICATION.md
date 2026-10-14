# Open Application on Linux

The original `com.elgato.streamdeck.system.openapp` action now appears alongside
Open. Choose an installed desktop application or an executable, optionally add
arguments, and choose what happens when the key is held.

A release before 500 ms opens the application. Holding for 500 ms requests close,
force close, or nothing; releasing afterwards does not reopen it. Page departure,
disconnect and cancelled nested actions discard pending holds. Settings remain in
the original `exec`, `source`, `args`, `bring_to_front`, and `long_press` fields.
Foreign executable paths remain editable and require a Linux replacement.

The installed-app picker now shares an icon list with Smart Profiles. Search
matches names, desktop IDs, launcher paths and executables. Radio rows support
keyboard selection;40 results are shown at a time with Show more for the rest.
GTK resolves the installed application's absolute or theme icon on the UI thread;
the native worker uses the existing bounded thumbnail decoder to return PNG.
Icons load when visible and share a four-request limit with action icons. Missing
or unreadable artwork uses a letter fallback. Native SVG and hicolor theme lookup
are verified. Sidebar icons and legacy XPM decoding remain outside this picker implementation.
Automatic key artwork is described below.

Installed applications use GIO desktop launchers, retaining their working
directory and launch flags. Additional arguments are parsed into literal words,
then escaped for the desktop entry parser; no shell is invoked by Deckforge.
Launchers that explicitly name a shell still run that selected launcher as usual.

On KDE, Bring running application to front uses a temporary KWin script matching
the selected executable's pinned current-user processes and stable window UUIDs.
On local X11, EWMH activation uses XRes-verified process ownership and restores
minimized windows. When supported, it requests the window's workspace before
activation; sticky windows do not change workspaces. Window managers retain
control over accepting focus requests. Normal Close first
requests window closure, allowing the application to handle unsaved work. Only a successful inventory with no matching windows permits a SIGTERM
fallback; refused close requests and desktop errors do not. Force close uses the existing
pidfd-backed SIGKILL path. Launchers sharing an interpreter are distinguished by
their fixed launch arguments. For manually entered executables, the configured
arguments also identify running processes. Extra desktop-launcher arguments remain
launch-only. Deckforge refuses to close itself.

On other Wayland desktops, D-Bus-activatable applications can use their desktop activation
contract. Other running applications report that bringing to front is unavailable;
disabling that option permits launching another instance. Universal compositor
activation and launcher wrappers whose runtime executable changes remain open. Window-close requests do not prove that an
application accepted the close.

Desktop requests run on a separate bounded worker so compositor timeouts do not
delay keyboard releases. Launched applications outlive Deckforge.

## Reference and verification

The supplied Windows executable SHA-256 is
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
REA/Ghidra statically recovered the original settings loader at `0x1402cf9c0`,
500 ms press timer at `0x1402d0b20`, short-release dispatch at `0x1402d0b40`,
hold handler at `0x1402d0880`, and `none` / `quit` / `force_kill` enum mapping at
`0x1402d1d60`. No Windows application was executed.
Evidence is retained in `../evidence/open-application/` relative to the repository.

Rust tests exercise literal arguments through a real GIO launcher, working
directories, hold cancellation and process isolation. Native WebKit tests exercise
application search, selection, arguments, original enum values, Save and restart.
The native picker fixture also verifies absolute/theme icon rendering and Smart
Profile selection/restart in private state. Reports/captures:
`target/application-icons/native/`. Frontend tests cover radio keyboard selection,
search/paging, missing artwork, late replies and shared request throttling.
The process suite exercises a uniquely copied test executable and, with `--kwin`,
a disposable Qt window. It verifies actual activation without another launch,
normal window close, force close, held release suppression and page cancellation.
No user applications are used as test targets.

Sources: [GIO launch](https://docs.gtk.org/gio/method.AppInfo.launch.html),
[desktop entry arguments](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html),
[KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/).

Local X11 activation/close acceptance uses owned Xvfb/Openbox and Qt windows;
see [Close Application](CLOSE_APPLICATION.md) for requirements and test coverage.

## Automatic artwork and running feedback

Open Application keys now use the selected desktop application's icon and show a
small green dot while a matching application is observed running. External starts
and exits update the key too. Several matching processes keep the indicator on
until the last one exits. Custom/imported key images retain precedence and receive
no automatic overlay. Reset image restores the automatic artwork; Appearance's
unsaved preview follows the same rule. The editor, nested key previews and physical
frame output share this rendering path. Runtime observations never enter profiles.

The existing current-user plugin process watcher is reused, with 500ms sampling.
A fixed executable/argument prefix identifies each target, including manual
interpreter commands. Only candidate processes have their argument prefix read;
opened process directories, start times and executable checks reject reused PIDs.
Unreadable matching identities give an unknown result. Unrelated unreadable
processes do not suppress all indicators. This is sampled feedback, not an audit
log; very brief processes may be missed. Up to512 visible Open Application controls
are observed across devices. Navigation/settings changes reject old results.

Desktop launcher icons support absolute files and freedesktop hicolor/pixmaps
lookup without requiring a GUI thread. Manual executables use an icon only when
one visible installed launcher matches unambiguously; otherwise the existing key
background remains and the running dot still works. Images use the existing
bounded thumbnail decoder. Icons/configuration are cached for the visible action's
settings; restart after replacing its desktop entry/icon.
Following arbitrary desktop theme changes live remains open.

Launcher executable parsing now handles quoted paths containing spaces. GIO's
get_executable shortcut splits such paths at the first space; Deckforge parses the
complete Exec value instead. Escaped percent signs in fixed arguments are literal,
and field codes end the fixed process-matching prefix. No shell is introduced.

Static reference: KA_OpenApp image callback0x1402d0f50 selects cached idle/running
images from process-manager matches.0x1402cf280 builds the application's icon and
its running-overlay variant;0x1402d1130 refreshes images and subscribes to process
changes. Evidence: ../evidence/open-application/running-artwork-native.json. The
Windows app was not executed; Deckforge's green dot is its own artwork.

Tests: application_artwork_interop.py verifies absolute/hicolor icons, external
launch/exit, multiple instances, argument isolation, target replacement, restart
and unchanged saved state. Native_application_artwork_interop.py checks rendered
pixels, automatic/custom Appearance previews and restart. Focused Rust checks
cover hardware frames, custom precedence, stale settings and process identity.
The existing launcher test also executes a copied Python binary from a quoted path
containing spaces. Proofs/screenshots: target/application-artwork/. Physical device
acceptance remains with the user.

Icon lookup uses the documented [freedesktop-icons lookup](https://docs.rs/freedesktop-icons/0.4.0/freedesktop_icons/struct.LookupBuilder.html).
