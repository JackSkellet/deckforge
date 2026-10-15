# Clipboard-free Text actions

Text now offers **Paste from clipboard** and **Type without clipboard**. Typing
uses the public `org.freedesktop.portal.RemoteDesktop` keyboard API. It requests
keyboard access only, with no screen capture, pointer access, clipboard access,
input recording or permission-store writes in production.

Choose Type without clipboard, save the action, and click **Enable desktop typing**.
The desktop may ask for permission. Setup never sends the action's text. After
enabling it, focus the destination app and press the Stream Deck key. Permission
lasts for this Deckforge session; it is not restored automatically after restart.
Imported explicit `isTypingMode: true` actions use this path. Loading/importing or
saving an action does not request permission or type anything.

An active typing action exposes **Stop typing** in the header, including after
switching away from that action. Setup can be cancelled, and access can be disabled.
Stopping closes this app's portal session. Typing resumes only after explicit
Enable. The editor's Test press is disabled for typing actions and containers that
contain them, because their text/Enter would be sent back to the editor's controls.
Physical input and authenticated diagnostic input remain supported.

## Paste shortcut editor

In Paste from clipboard mode, expand **Paste shortcut** to edit the key, modifiers,
sides and physical/logical mapping. For a terminal, keep V and enable Ctrl and
Shift. **Reset paste shortcut** restores standard Ctrl+V. An empty custom shortcut
also falls back to Ctrl+V. Switching to Type without clipboard retains the custom
paste record but does not execute it.

Fresh Text actions now use logical mapping for standard paste, as do new Clipboard
actions. Older saved/imported actions without the mapping field retain physical
behavior. Custom chords keep their own mapping; new standard-paste defaults do
not override existing custom shortcuts. Malformed non-object custom data is retained
until explicitly reset or replaced; the editor identifies the standard fallback.
The native editor verifies editing, reset, mode changes, Save and restart. Real
Dvorak GTK/Konsole tests include a legacy Ctrl+Shift+V record for terminal paste.

## Behavior

The complete string is validated before sending input. CRLF and lone CR become
one Return each; LF, Tab and optional final Enter have keyboard semantics. Other
control characters, including NUL, are rejected before any text is sent. Valid
Unicode uses standard Unicode keysyms, including characters absent from the active
keyboard layout. Text is limited to the existing 1 MiB bound.

The existing System action queue serializes typing with other keyboard actions.
Typing refuses to start while Deckforge owns held shortcut keys. Overflowing a
queue with another text job returns an error without permanently stopping the
keyboard worker; lost key-release requests retain the existing release-all policy.
5 ms pacing between characters limits downstream input backlog. Stop prevents
future submissions; events already delivered to another process cannot be recalled.
The session is also cancelled early during host shutdown.

The clipboard is never used by this mode. `Hotkey` settings are retained but ignored
while typing, matching the observed original branch. Paste mode keeps its existing
clipboard/custom-shortcut behavior. No keystroke text or permission token is logged
or stored outside the action's ordinary saved settings. Runtime permission status
and its connection are not serialized into profiles or the session file.

`system_typing.rs` owns a separate cancellable portal task so permission dialogs do
not block the host or keyboard releases. Each task has a generation; late setup or
typing completions cannot overwrite a newer cancelled state. Request subscriptions
are limited to this connection's namespace. Session Closed signals handle external
revocation. Key releases are attempted even when a press has an uncertain outcome;
failed sessions close instead of silently falling back to clipboard pasting.

## Original 7.6 evidence

Static analysis used the supplied executable SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
`KA_Text` settings loader `0x140394bf0` and serializer `0x140395100` establish
`pastedText`, `isSendingEnter`, `isTypingMode` and `Hotkey`. Execution at
`0x1403956b0` selects a clipboard task for false and the OS typing manager for true;
the latter normalizes CRLF/CR to LF and forwards the Enter flag. Windows manager
`0x14059d700` builds Unicode INPUT records and calls SendInput.

The loader supplies `!text.isEmpty()` as the missing-mode default. The JSON value
helper `0x1401b9ed0`, called through `0x1401b8380`, returns that default when the key
is missing. This is static evidence, not execution of the Windows app. Private
records and Evidence IDs are under `../evidence/text-typing/`.

Missing modes now have explicit import/export rules:

| Boundary | Missing `isTypingMode` |
| --- | --- |
| New native or untagged import | Type for nonempty `pastedText`; Paste for empty/missing text. |
| New import with package `OSType: Linux` | Paste, preserving earlier Linux archive behavior. |
| Existing saved Deckforge session | Retains its saved data and previous Paste behavior. |
| Export of an existing missing-mode action | Writes explicit Paste in the output only, including raw-only nested actions on inactive pages. |
| Explicit mode, including invalid values | Preserved; invalid values remain errors until corrected. |

The import rule applies to profiles and shared-action files, folders and recognized
nested action containers. Unknown plugin payloads, unsupported raw controls and
malformed settings are preserved. Original input files and cached archive member
bytes are not rewritten. Exported explicit choices prevent later imports, including
native apps, from reinterpreting a saved Linux action's effective mode.

Historical untagged Linux exports can be indistinguishable from native files.
Without an explicit mode or Linux marker, native defaults apply; review the mode
when importing such an archive. Future exports are unambiguous because the choice
is explicit. No existing sessions are migrated or silently switched to typing.
New locally assigned actions still explicitly default to Paste.

An omitted text field means empty text, so Enter-only actions work. Explicit null
or non-string text remains invalid. The editor identifies non-boolean saved modes
and only replaces them when the user selects a valid mode.

`src/action_defaults.rs` implements the bounded import/export transformation. Six focused
Rust tests cover rules, nested/hidden/opaque data, old-runtime export preservation,
shared actions, source bytes and failed-import rollback. The 15 export tests include
original installer profiles and byte-exact media/resources; the only newly permitted
metadata additions are explicit effective modes on interpreted Text actions. Raw
controls outside the modeled layout retain identical parsed data. The 28 transfer
and 15 System-action tests pass. Real host tests cover native/plain/Windows/macOS/Linux
archives, shared files, preserved existing sessions, portable exports and restart.
Native WebKit verifies mode display, malformed-mode preservation and explicit
permission controls. Reports are under `target/text-defaults/`.

## Verification and limits

The actual release host passed `tests/text_typing_interop.py` against a focused GTK
TextView and real Konsole in a private KWin 6.7.5 desktop. Tests cover Latin, Greek,
CJK, emoji, combining marks, tabs, newline normalization and optional Enter;
clipboard preservation; no typing before permission; setup sending no text; FIFO
ordering; invalid-text preflight; active cancellation; cancellation of a real
permission dialog; external session revocation and recovery; shutdown; and restart.
The private permission store's process is verified to use the fixture's data folder
before test-only permission configuration. No real desktop permissions are changed.

Native WebKit tests verify mode selection, Save/restart, preserved unknown and
custom-shortcut fields, explicit permission controls, and the editor self-typing
guard. Screenshots were inspected. The existing command/clipboard/environment OS
suite also passes on the same release. Evidence is under `target/text-typing/`.

The prior EIS probe's missing text device did not establish that typing was
impossible. The public portal's NotifyKeyboardKeysym path works on this installed
KDE backend; upstream [RemoteDesktop implementation](https://github.com/KDE/xdg-desktop-portal-kde/blob/master/src/remotedesktop.cpp)
and [keyboard forwarding implementation](https://github.com/KDE/xdg-desktop-portal-kde/blob/master/src/waylandintegration.cpp)
explain the alternative route. The product uses the public permission flow, not the
private test relay or direct KWin internal API.

Other portal backends, X11 sessions, unusual layouts/modifier states, games and IMEs
need their own acceptance. Physical Stream Deck validation remains with the user.
The isolated tests open no uinput device and perform no USB I/O. This is a verified
typing subset, not a claim that full desktop feature parity is complete.

## Paste scheduling

Paste keeps its existing 150ms settling interval before optional Enter and the
next clipboard-changing action. The System worker now waits by deadline rather
than sleeping: releases of already-held hotkeys and dial strokes, dial cancellation
and command Stop remain responsive. New keyboard presses and later clipboard
writes remain FIFO behind the paste. A bounded32-job deferred queue prevents
unbounded memory growth; overflow is reported. A rejected release removes an
unstarted matching press so it cannot later become a stuck key.

Captured command output can also write the clipboard, so completion polling waits
until the current paste settles. Process Stop remains available. Existing capture
size/time limits remain. Shutdown suppresses pending Enter and releases owned keys.
Empty Text with Send Enter now writes an empty clipboard and invokes its paste
shortcut before Enter. Empty Text without Send Enter is still rejected. Typing mode
continues to leave clipboard contents untouched.

Reference evidence: KA_Text validity0x140395cc0 accepts empty text only with Enter;
its dispatch0x1403956b0 creates text MIME data even then. ESDPasteTask0x140252860
uses timer stages for clipboard, shortcut and Enter. ESDPasteManager0x140252390
serializes tasks and includes an optional clipboard restoration path, with a false
configuration fallback in the manager construction. The subsequent [restoration audit](CLIPBOARD_RESTORE.md) identifies the global
configuration keys and full-MIME clone contract. Actual vendor runtime settings were
not observed; Deckforge retains its documented clipboard replacement policy. Evidence: ../evidence/text-typing/paste-scheduling-analysis.json.
The Linux settling duration is retained behavior, not a claim of exact vendor timing.

The private focused GTK/Konsole test now checks key-up priority during a paste,
queued press/release order, FIFO clipboard writes and two Text actions, empty-text
Enter and shutdown suppressing Enter. Dial tests cover releasing an active stroke
while new strokes wait. Reports: target/paste-scheduling/acceptance/. These use the
real worker with a test packet writer relayed to a private compositor, not physical
USB/uinput acceptance or the user's clipboard.
