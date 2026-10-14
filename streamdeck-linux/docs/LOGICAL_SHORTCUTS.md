# Logical shortcut mapping on X11 and KDE Wayland

Hotkey, Hotkey Switch, dial shortcuts and custom Text paste shortcuts now offer
**Key mapping → Follow desktop layout**. Clipboard actions expose the equivalent
**Shortcut key mapping** choice. Physical key positions remain available. Existing
records without a mode retain physical behavior; loading or saving them never
silently changes the mapping. The explicit linuxKeyMode field survives export,
import and restart, including unknown future values that remain runtime errors.

New Clipboard and standard Text-paste actions default to logical mapping. Existing
sessions/imports and general Hotkey defaults retain their previous choice. This
avoids changing unknown native virtual-key/OEM semantics by assuming every stored
Qt code means a desired character. Text exposes its paste key/modifiers/mapping in
an expandable editor; custom records retain their own mode.

Logical mapping resolves the selected letter, digit or punctuation against the
active X11 or KDE Wayland layout when the key is actually pressed. Required Shift or
AltGr levels are added when available. Explicit modifiers and modifier sides are
preserved. Keypad and nonprinting keys retain their physical identity. A selected
symbol absent from the map produces an error rather than a guessed key.

The implementation reuses the existing uinput keyboard and hold ownership. It
requests no additional desktop permission. On Wayland, a read-only connection
creates no surface and KDE supplies the active layout group. On X11, authenticated
XCB/XKB queries read the core keyboard map and effective group; KDE is not needed.
libxkbcommon-x11 is loaded only for X11 and missing support is reported explicitly.
When WAYLAND_DISPLAY is present, Wayland remains authoritative; a failed Wayland
query does not silently use a potentially different Xwayland map.

Releases use captured physical codes, even after a layout switch. Shared modifier
counts and reverse release order remain in the existing keyboard owner. Dial queues
resolve each stroke at execution, not when a turn was queued. Physical modes and
nonprinting/keypad-only chords do not need a layout lookup.

One metadata-only thread services a bounded lookup queue. The input worker uses a500ms lookup deadline, checks cancellation every5ms, and
checks it again before using a
result. Stop cancels a pending lookup promptly. The reader never presses keys;
results arriving after timeout/cancellation are discarded. A stalled native display
call can occupy the one reader until it returns, but cannot create additional
threads or emit delayed input. Further queued work is bounded and errors explicitly.
Existing keyboard-error cleanup releases owned keys. Process exit owns reader
teardown; no detached helper process or external service is installed.

Wayland connection, keymap I/O and KDE queries retain their150ms individual limits.
Keymap text is limited to1MiB and read with private mmap. Native XKB operations are
confined to the metadata thread. Observed resolution was approximately3–4ms on
Wayland and under2ms for logical X11 cases on this host; these are measurements,
not latency guarantees. Unavailable/ambiguous seats, unsupported maps, missing
services and unmappable symbols are errors, with no guessed physical fallback.

The supported subset is the core keyboard on X11 and a single Wayland seat on KDE.
Other Wayland compositors, unusual per-device maps and externally held physical
hardware modifiers need further acceptance. Caps/Num Lock, repeat and a second
virtual keyboard pass controlled KDE acceptance; see [Keyboard state](KEYBOARD_STATE.md).
Windows/macOS imports without this Linux field retain the legacy physical mode;
choose the desired mapping explicitly. Full keyboard-layout parity is not claimed.

## Implementation checks

Production code lives in keyboard_layout.rs and system_keyboard.rs. Typed Chord
metadata carries the mapping choice through System plans and Dials. The parser
keeps portable Qt identity and the original side/keypad/OEM flags. Full chords are
resolved before any press. Literal key identities and unsupported modes are never
silently coerced. No mapping query is performed on release.

Tests/logical_hotkeys_interop.py uses the actual Rust decoder/resolver/keyboard
writer and actual System worker, relaying its captured packets only to an owned
private KWin/Qt client. US, German, Norwegian and French pass logical Y, Ctrl sides,
Shift, AltGr, slash/backslash/number rows, held release across a live layout switch,
shared Ctrl ownership, and queued dial strokes resolving under changed layouts.
An unavailable Wayland socket emits no fallback key packets. Timings/source hashes
and event traces are in target/logical-hotkeys/proof.json.

The actual System-worker focused clipboard suite also passes under US Dvorak:
Copy/Cut/Paste, Unicode Text/Enter, real Konsole terminal paste and custom Text
paste shortcut. This demonstrates behavior beyond layouts sharing QWERTY clipboard
positions. Report: target/logical-hotkeys/clipboard-dvorak/proof.json.
Native WebKit Save/restart and actual-host export/import/restart checks cover the
mapping choice, side modifiers, unknown settings and legacy missing modes without
executing actions. Reports: target/logical-hotkeys/native/ and settings.json.

`tests/x11_layout_interop.py` uses a disposable Xvfb and owned Qt/GTK/Konsole clients.
It checks four active groups, punctuation and digits, physical opt-out, held release
across group changes, live Dvorak keymap replacement, and actual clipboard/terminal
operations. The XTest relay and fixtures verify server PID/UID using Unix peer
credentials before sending input. Xvfb is extracted under target from the configured
package repository; no system install is changed. A stalled, owned X11 proxy proves
Stop, a bounded lookup timeout, no emitted keys, discarded late results and recovery.
Reports: target/x11-layouts/. The production backend reads XKB metadata only; XTest
is exclusively the fixture transport. Reference: [libxkbcommon X11 API](https://xkbcommon.org/doc/current/group__x11.html).

## Verified backend behavior

Two independent probes ran in private KWin sessions with owned Qt clients and
US/German/Norwegian layouts. The observed versions were KWin6.7.5-3.1,
xdg-desktop-portal-kde6.7.5-1.1, xdg-desktop-portal1.22.1-2.1, libei1.6.0,
and libxkbcommon1.13.2. Tests use private config/data/runtime/session buses, verify
KWin's PID, and seed permission only in their verified disposable permission store.
They never grant permission on the user's bus, open real uinput or use USB.

`tests/portal_shortcut_probe.py` uses the existing Rust Portal implementation in
an ignored, explicitly gated test. Physical modifier events plus logical keysyms
correctly deliver Y, Ctrl+Y, Shift+Y, Ctrl+Shift+Z, Right Ctrl+Y and AltGr behavior.
However, switching layouts between keysym press/release leaves the original
physical key pressed until the session is closed. The fixture records client events
both before and after session close, distinguishing a working release from cleanup.
All three starting layouts reproduce this. Evidence: target/portal-shortcuts/;
assessment.json records the observed failure separately from ordinary chord success.

This is consistent with the [KWin fake-input implementation](https://raw.githubusercontent.com/KDE/kwin/Plasma/6.7/src/backends/fakeinput/fakeinputbackend.cpp),
which resolves a keysym against the current keymap for each event. This source is
corroborating evidence; the isolated client trace establishes the observed result.
Do not implement held shortcuts by sending the same keysym on release. Closing the
whole keyboard session would release unrelated held shortcuts and lose permission.

`tests/eis_keymap_probe.py` connects through the public RemoteDesktop portal and
ConnectToEIS, then uses the returned device keymap to resolve Y to a physical code.
It sends Ctrl/Shift modifiers physically and retains the resolved code for release.
Actual Qt scans/symbols/modifiers confirm logical Y, Ctrl+Y, Right Ctrl+Shift+Y,
release after a live layout change, Y in the second layout, and switching back.
Releases are observed while the session remains open. Evidence: target/eis-layouts/.
This is a test prototype, not production integration or a general keysym resolver.

The tested KWin EIS sender did not provide initial keyboard state before any
changes; later modifier/group changes did produce EIS events. An earlier conclusion
that state events were entirely absent was too broad. The EIS prototype falls back
to KDE's read-only org.kde.KeyboardLayouts.getLayout until EIS state arrives. The
Wayland-reader variant deliberately uses KDE for every group lookup, proving that
it does not depend on EIS state delivery. Initial/final sources are recorded in the
report. Other compositors need a verified state source; missing state must remain
an error rather than a guessed layout.

## Integration constraints

Keep System worker ownership, dial timing, FIFO ordering and cancellation intact.
Resolve a logical chord at press time, retain the concrete device/keycode identity
for its full lifetime, and release that captured identity. Device/keymap replacement
must cancel affected holds; a release must never be re-resolved under a new layout.
Shared modifiers need reference counting on the actual emitted identities.

Preserve existing saved physical shortcuts. A deliberate logical/physical choice
must remain explicit through Save, import/export and restart. Native/default
behavior needs a separately verified boundary rule, not a silent rewrite of old
profiles. Keypad/OEM identities, Backtab and side-specific flags must retain their
meaning. The initial prototype resolves only base-level Y; shifted symbols, number
rows, missing symbols, Caps/Num Lock and external physical modifiers need acceptance.

The [public portal contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)
prohibits mixing Notify methods with EIS after ConnectToEIS on the same session.
Existing Unicode typing uses NotifyKeyboardKeysym and KWin's tested EIS connection
has no Text capability. Do not switch the existing typing session to EIS and break
Unicode input. A separate public Wayland keymap reader is now verified without a window or focus.
`tests/wayland_keymap_probe.c` binds only the seat/keyboard, reads the bounded XKB
map, and fails if it receives enter/key events. Running the logical resolver with
that map passes the same three-layout and held-release tests; proof is under
`target/wayland-layouts/`. This permits reuse of the existing physical-event writer
and its hold ownership, without another permission/session. Those prototype tests relay physical events through EIS. The production integration
described above now resolves behind the keyboard module and retains the existing
uinput writer.

Acceptance gates for this work include: held-key release after layout
change, overlapping actions and shared modifiers, dial cancellation/page departure,
backend revocation/disconnect, permission cancellation, typing interoperation and
ordinary clipboard/terminal shortcuts. User-owned physical-device acceptance is
separate. No claim of full keyboard-layout parity is justified yet.
