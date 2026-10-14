# Close application on Linux

New Close actions request ordinary window closure on KDE Wayland and local X11 desktops. The application can
accept, refuse, or show its own save prompt. A successful request does not mean the
application has exited. Normal windows and dialogs are included.

Choose the application executable and, optionally, a literal argument prefix.
Quote arguments containing spaces; shell expansion is never performed. This lets
two scripts sharing Python or Node remain separate targets. Matching is restricted
to the current user and exact executable. Deckforge refuses to close itself.

The Close behavior choices are:

- **Close windows only:** request closure; do not signal a headless process.
- **Send SIGTERM:** ask matching processes to terminate, without compositor access.
- **Close windows; stop background processes:** request closure when windows exist;
  use SIGTERM only after a successful desktop inventory finds none.

Force close explicitly uses SIGKILL. Its checkbox preserves the inactive behavior
setting. Signals do not guarantee an unsaved-document prompt. Desktop connection
errors never fall back to signals. When GUI windows are present, automatic mode
also leaves matching background helpers alone.

New assignments and newly imported native profiles/shared actions default to
Close windows only. Existing saved records with no Linux method keep automatic
behavior. Linux-tagged imports and exports materialize that legacy automatic
default. Explicit values and unknown settings are preserved. No session migration
is performed. As with Text defaults, an old unmarked archive cannot reliably be
identified as Linux-originated.

Window closure requires KDE KWin scripting on Wayland, or a local X11 server with
XRes 1.2 and an EWMH window manager. Explicit signals work without those services.
Launcher wrappers, Flatpak/AppImage launch indirection, other Wayland compositors, and
application-specific save-dialog acceptance remain outside this verified subset.
Foreign executable paths need a Linux replacement. The optional argument prefix
must match the running process, not merely a wrapper's original launch command.

Process handles are pinned before inspection. The close path captures window UUIDs
and PIDs, rechecks the process executable/arguments, then requests closure only for
those captured windows. On X11, server-reported XRes client PIDs identify owners;
the editable `_NET_WM_PID` label is never trusted. Only normal/dialog windows
advertising `WM_DELETE_WINDOW` receive close requests. Matching utility windows
and windows without that protocol still prevent automatic signal fallback.
X11 I/O has a two-second deadline, with no background task sending late requests.
An observed window disappearing does not cause a signal
fallback. Requests run on the application worker so desktop timeouts do not block
keyboard releases. Open Application's held Close uses the same automatic path.

## Evidence and checks

Static analysis of the supplied Stream Deck 7.6 Windows executable
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`
identified `KA_Close` settings loader `0x14038dfa0`, serializer `0x14038e150`,
and execute `0x14038dc20`. At `0x14038dd96`, assembly inverts `forceQuit` before
calling process manager `0x1407b0020`. Normal Close supplies one type-0 operation
with a 1000 ms wait; Force supplies type 3 with 100 ms. Backend `0x1407bdb00`
uses EnumWindows/WM_CLOSE for type 0 and TerminateProcess for type 3. The backend's
empty-options fallback is not used by this action. No matching process is a no-op.
Native evidence is retained in `../evidence/system-close/native-analysis.json`.
The original Windows application was not executed.

`tests/close_application_interop.py` uses a private session bus and KWin compositor,
owned Qt normal/dialog windows, and uniquely copied executables. It verifies close
veto and acceptance, background-helper protection, argument isolation, held Close,
explicit signals, backend failure, legacy headless behavior and no-match success.
It also runs Open Application activation/hold/page-cancellation acceptance inside
that private desktop. It never targets user windows or USB devices.

`tests/native_close_application_interop.py` checks all three form modes, argument
scope, Force's dormant settings, opaque settings and restart without executing an
action. Rust tests check pinned identities across exec, process isolation, literal
arguments, defaults and original profile export preservation. Reports and native
screenshots are in `target/system-close/`.

API reference: [KWin scripting](https://develop.kde.org/docs/plasma/kwin/api/).

`tests/x11_window_interop.py` uses private Xvfb/Openbox and owned Qt clients. It
checks minimized and cross-workspace activation, normal/dialog close veto and
acceptance, held Open close, forged PID isolation, utility/missing-protocol
protection, missing inventory/WM errors and explicit signals. It also exercises
a local server without an authority file; authentication remains the server's
decision. Reports: `target/x11-windows/`. KDE regression and native form/restart
checks pass separately. No physical device or user window was a test target.

References: [EWMH](https://specifications.freedesktop.org/wm/latest-single/) and
[ICCCM](https://x.org/releases/X11R7.6/doc/xorg-docs/specs/ICCCM/icccm.pdf).
