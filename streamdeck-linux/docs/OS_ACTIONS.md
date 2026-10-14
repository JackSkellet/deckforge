# Linux OS actions

The 14 October 2026 priority is OS integration. Device validation belongs to the
user. Background/screensaver polish and Windows/macOS-only plugin compatibility
are deferred. These actions use Deckforge UUIDs, not invented vendor contracts.

## Clipboard

`com.deckforge.system.clipboard` appears under **System → Clipboard**.

- Copy saved text writes exact Unicode and whitespace to the desktop clipboard.
  Empty text replaces the previous content. This mode needs no virtual keyboard.
- Copy selection sends Ctrl+C, Cut selection sends Ctrl+X, and Paste clipboard
  sends Ctrl+V to the focused application. Copy and Paste offer Ctrl+Shift for
  terminal applications. Terminal Cut has no standard shortcut and is rejected.

The existing Text action can paste through the clipboard or type through the
desktop keyboard portal, optionally followed by Enter. See [TEXT_TYPING.md](TEXT_TYPING.md)
for typing permission, cancellation, native evidence and acceptance. The paste path and Clipboard action share the persistent arboard clipboard owner, with Wayland
data-control and X11 support. A failed write clears the handle for the next retry.
Clipboard retention after Deckforge exits depends on the desktop clipboard manager.
Shortcut modes require the existing uinput keyboard setup and application focus.
Text paste uses an interruptible settling deadline, preserving clipboard FIFO while
held-key releases remain responsive; see [paste scheduling](TEXT_TYPING.md#paste-scheduling).
Desktop image/file formats can be pasted by the receiving app through its normal
shortcut; this action itself writes text only.

This is the OS clipboard. Deckforge's action Copy/Paste remains an independent
profile-editing clipboard. An OS MIME extension for action graphs would be a Linux addition; the original
action clipboard is in-process, not an established OS MIME contract. See
[Nested clipboard](NESTED_CLIPBOARD.md).

## Run command

`com.deckforge.system.command` appears under **System → Run command**.

Program mode resolves an executable name or absolute path. Arguments use quoted
word splitting, without environment expansion, globbing, pipes or substitution.
Shell mode explicitly runs the supplied script with `/bin/sh -c`, supporting
those shell operations. Working directory must be an existing absolute directory;
an empty field uses the home folder. Neither mode executes during editing/import.
Execution follows the existing owned key-release path and works in sequences.

Background commands have no input stream. New actions default to **Show result**;
existing saved actions without an output setting retain **Discard output**.
Discarded-output commands continue after Deckforge quits. All commands run
asynchronously and share the existing 32-child bound. Launch failures and
unsuccessful process exit statuses appear in system status.

Show result captures stdout and stderr separately. **Copy successful output to
clipboard** also copies complete successful UTF-8 stdout, preserving whitespace
and a trailing newline. Nonzero exits, timeouts, cancellation, NUL/invalid UTF-8,
oversized stdout and inherited pipes left open do not write the clipboard.
Successful empty output replaces clipboard text with an empty string. Explicit
Copy output uses the same completed-result checks and rejects expired run IDs.

Captured commands have a timeout of 1–3600 seconds, default 30, and a **Stop
command** button. Stop/timeout kill the command's owned process group. Captured
commands are stopped when the host quits, before waiting for HTTP workers.
Programs that detach into separate sessions are outside that process group.
One captured run per action and eight in total are allowed. Starting another run
of the same action while its captured command is active reports an error.

Nonblocking pipes are drained by the existing System worker, without additional
capture threads. The last 16 captured results remain in memory, with up to 1 MiB
stdout and 64 KiB stderr each. Excess data is drained and discarded, not allowed
to block the process. Status snapshots contain metadata only; Show output fetches
up to 16 KiB from each stream on demand. The preview is escaped text, never HTML.
Output bytes are not written to profile/session files or activity logs. A new run
replaces the old result; restart clears results. Viewing output preserves unsaved
editor drafts. The same controls appear for selected saved commands inside Multi Actions, switches,
Action Trigger and Key Logic, including nested sequences. Read-only profiles allow
result inspection and stopping a running command without permitting configuration
edits. Unsaved reorders use a unique stable ActionID; ambiguous legacy drafts hide
controls until saved/reverted, preventing Stop/Copy from reaching a different sibling.

Streaming output, interactive stdin,
and broader desktop clipboard/action-graph interoperability remain
open. These are Linux additions rather than assertions of original vendor behavior.

Environment variables are editable as name/value rows, with an explicit Unset
choice. Values are literal, including whitespace, newlines and dollar signs. Empty
values set an empty string; Unset removes an inherited variable while retaining the
row's inactive value for editing. Missing environment settings inherit unchanged.
Names follow `[A-Za-z_][A-Za-z0-9_]*`; duplicates, NUL, invalid rows, more than64 rows
or more than64KiB of names/values fail before launch. Unknown row fields survive edits.
Unsupported imported formats are preserved until the user explicitly replaces them.

Overrides affect only the target command, including shell mode. They never mutate
Deckforge's environment. A PATH override controls executable-name lookup using
absolute directories; an empty PATH disables that lookup. Unsetting PATH uses
`/usr/bin:/bin` to locate the program, then removes PATH from its environment.
An empty working folder still uses Deckforge's home directory, independently of
any target HOME override. These are ordinary action settings, saved in profiles
and profile exports; they are not a separate credential store.

The terminal launcher keeps its inherited environment. Target overrides are applied
inside the encoded launch payload with `/usr/bin/env`, so terminal GUI variables
can be overridden for a target without breaking the launcher. If an executable
path contains `=`, a `/usr/bin/nice -n 0 --` wrapper disambiguates it from an env
assignment without changing priority or introducing a shell that rewrites variables.
The existing terminal payload limit applies to environment data too.

Terminal mode defaults to Konsole with `--separate --hold -e`. Terminal program
and argument prefix are editable for other terminals. A launcher must accept a
command and argv after that prefix. Only Konsole has real acceptance evidence here.
Its held window shows command output after completion; a launcher exit status
does not necessarily represent the inner command's exit status.
Terminal mode keeps output in its own window, leaving saved background capture
settings dormant until terminal mode is disabled.

Konsole expands environment references even in separately supplied arguments, as
confirmed by a failing literal-argument fixture and its
[Session implementation](https://github.com/KDE/konsole/blob/master/src/session/Session.cpp)
and [ShellCommand implementation](https://github.com/KDE/konsole/blob/master/src/ShellCommand.cpp).
Terminal mode therefore transports a shell-quoted exec command as base64 to a
fixed `/bin/sh` wrapper. Saved arguments never enter the wrapper as shell syntax.
This requires `/usr/bin/base64` and bounds the quoted payload to 64 KiB. No temporary
script file or additional long-lived worker is needed. The working directory is
also applied inside the wrapper so a reused terminal process cannot override it.

## Verification

- `cargo test system_command`: literal arguments, directory, explicit pipelines,
  terminal transport, invalid settings and quoting. Clipboard planning tests
  cover exact Unicode, empty text, standard/terminal shortcuts and invalid modes.
- Existing System worker tests exercise keyboard ownership and asynchronous work.
- Frontend field tests cover switching modes without discarding inactive settings
  or unknown imported fields. Native WebKit saves Clipboard and Run command,
  including shell/terminal fields, and verifies restart without executing actions.
- `tests/os_actions_interop.py` starts a private virtual KWin compositor and private
  session bus/runtime/config/data directories. An independent GTK application reads
  exact Unicode and empty clipboard data from the actual host. Owned commands test
  release ownership, literal quotes/newlines/dollars, directories, pipelines,
  background responsiveness, exit errors, real Konsole TTY execution and restart.
  It also verifies command-output copying, failed/binary/NUL/timeout/cancelled
  output leaving the clipboard untouched, empty-output copying, runtime-only
  results and stopping a captured command on host shutdown.
  DISPLAY is unset; the private Wayland socket prevents access to the user clipboard.

Proof is under `target/os-actions/`. No physical USB inputs or shortcuts were sent
to the user's focused application. User acceptance of Copy/Cut/Paste shortcuts and
other desktop environments remains separate from these automated checks.

Capture acceptance is under `target/command-capture/`, with the installed binary
hash in its interop/install reports. Nine command tests cover transport, capture,
both-stream overflow, invalid text, cancellation, timeout, orphaned pipes, bounded
retention and shutdown. Three result-viewer tests cover escaped text, captured
context/run IDs and stale asynchronous replies. Native WebKit acceptance verifies
the output settings, Stop, preview, stale-run rejection and unsaved-title retention.

Nested-result acceptance is under `target/nested-commands/`. Six focused tests cover
all four container forms, read-only controls, stable owners after unsaved reorders,
ambiguous legacy drafts and saved duplicate-ID siblings. The native fixture executes
and stops commands in Multi Action, Switch, Action Trigger and Key Logic containing
a Multi Action, reads the correct result and rejects another-device requests. No
clipboard writes or USB input are performed by this fixture. Existing ten Sequence
and three result-viewer tests pass; the native screenshot was visually inspected.

Environment acceptance is under `target/command-environment/`. Thirteen command
tests cover the previous execution/capture contracts plus literal environment
values, empty/unset entries, custom PATH, invalid/duplicate rows and `=` executable
paths. Existing thirteen System action tests pass after sharing the PATH resolver.
Two environment-editor and sixteen Fields tests pass. Native WebKit saves literal
Unicode/multiline values and Unset state, restores them after restart, and exports
them in a profile. The final private Wayland test executes background and real
Konsole commands with custom PATH, `=` filenames, literal values and unset/empty
entries. A deliberately unusable target Qt backend leaves Konsole working, and a
following command sees the original inherited environment. No user environment,
clipboard, profiles or hardware is changed by these fixtures.

## Focused application acceptance

`tests/focused_clipboard_interop.py` now verifies the actual System action worker
against a focused GTK TextView and a real Konsole terminal in a private virtual KWin
session. Copy, Cut and Paste move exact Unicode text; Text pastes multiline Unicode
and optional Enter; Copy saved text emits no keys or document changes. Konsole
accepts Ctrl+Shift+V, and Text accepts an imported custom Ctrl+Shift+V paste shortcut.
The terminal receiver reads text as data, never as shell commands.

Only the keyboard's file descriptor is replaced in the opt-in Rust test. The same
worker, action planning, ownership and input_event encoding run; a small C++ relay
forwards those packets through KWin EIS to the private compositor. No production
backend changes and no /dev/uinput access occur. The relay checks that the private
session bus belongs to the compositor PID created by the fixture. DISPLAY is unset,
the Wayland socket/runtime/config/data are private, and both ignored tests refuse
to run without the explicit fixture environment. The relay observed34 key events.

This proves focused client behavior with the worker-generated events. It does not
prove physical Stream Deck input, uinput discovery by the user's compositor,
other applications, layouts or desktop environments. Hardware acceptance remains
with the user. No rebuild/install is needed to use the behavior already installed.
Proof and logs are in `target/focused-clipboard/acceptance/`; the proof records the
Rust test binary and production source hashes rather than claiming a release-binary
input path was exercised.

The installed private KWin exposes an EIS keyboard but did not offer a usable text
device. The installed libei1.6 headers support text APIs and upstream
[KWin EIS source](https://github.com/KDE/kwin/blob/master/src/plugins/eis/eisbackend.cpp)
conditionally enables a text capability. API presence alone is not runtime acceptance.
A later public-portal acceptance resolved clipboard-free Unicode typing on this KDE backend; see [TEXT_TYPING.md](TEXT_TYPING.md). The older helper did not observe the KDE fake-input interface; that client-scoped
observation did not rule out the public portal backend. The earlier packet-relay
test uses EIS, while the new product typing path uses the public portal.
These probes do not establish support in a different compositor or the live session.
