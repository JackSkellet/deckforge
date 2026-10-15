# Deckforge continuation handoff

## Latest checkpoint: native drag repair, plugin setup and Linux OBS, 15 October 2026

The user requested these focused fixes after pausing the broader parity work and
explicitly approved Linux OBS support. No active goal exists; do not resume the
broader parity/clipboard restoration project without a new request. GitHub push
is authorized to JackSkellet/deckforge, codex/streamdeck-linux; CI remains manual.

- Fixed native GTK file-drop interception of internal WebKit control drags. Wry
  treats target-info 2 as files even for serialized DOM drags and finishes the
  drag before its callback. The replacement claims only external text/uri-list
  drags and leaves internal DOM drags to WebKit. Actual pointer tests cover move,
  occupied-slot swap, Ctrl-copy and external file import on an owned Xvfb.
- Plugins now offers Configure and a choice of the plugin's own property
  inspectors. Temporary setup actions have balanced lifecycle events, isolated
  settings/message routing, no hardware rendering or profile assignment, and
  reconnect replay. Global settings persist; per-action selections in setup are
  temporary. Existing assigned-action inspectors and device scoping remain intact.
- Built the GPL upstream OBS bridge at fcddcde567bdeafd235829d4fe8519894b158e29
  against native OBS 32.2.1 / Qt 6.12. A small Linux CMake wrapper supplies missing
  upstream build files and corrects the optional frontend-library .so probe.
  Bridge packages include complete source, submodules, licenses and checksums.
  Native install/check is available in Plugins > OBS Studio > Configure and via
  `deckforge obs-bridge [--install]`. Flatpak OBS is not covered by this installer.
- Added explicit native `obs` defaults for the official plugin's obs64.exe and
  com.obsproject.obs-studio subscriptions. The real HTML backend otherwise never
  receives applicationDidLaunch and remains at Waiting for OBS even with a
  reachable bridge. User mapping overrides, including blank values, take priority.

Installed executable SHA256:
`1dd221f1b9955023f6312a7fc9840587067b47ee0e01fbd19e080c11b15c0929`.
Previous executable retained at ~/.local/bin/deckforge.previous with SHA256
`6ffb701c82dcbd3ca3625b43b2d0af44bc15f09e34865924028cbf74a7431c29`.
Native OBS module installed at
`~/.config/obs-studio/plugins/StreamDeckPlugin/bin/64bit/StreamDeckPlugin.so`,
SHA256 `48e1c262d3523379f6ea2232fa75a6cd88f0e8c81dcdf6ff7f27fd7c77f9e73a`.
2388 saved profile/plugin/icon files and modes verified unchanged. No running
user application was restarted, no OBS scene/output changed. User must fully
quit/reopen Deckforge and OBS to activate the installed changes.

Verification: targeted setup, bridge installation, OBS mapping and 12 existing
inspector/lifecycle tests; six frontend setup/lifecycle tests; frontend build;
Clippy all-targets with warnings denied; release build. Native evidence is under
`target/control-drag/final-setup`, `target/plugin-setup-interop`, and
`target/obs-interop`. The latter uses private OBS config, Xvfb and an isolated
network namespace: unchanged official protocol classes connected, enumerated
scenes, switched/restored two private empty scenes; Deckforge's read-only bridge
check returned OBS 32.2.1. The full installed protected plugin also connected in
native WebKit, and Plugins > Configure opened its original Scene inspector.
The inspected screenshot showed scene controls with no Waiting for OBS overlay.
This does not validate every OBS action, unrelated plugin login, Flatpak, or USB.
Optional bridge distribution archive and every included checksum were verified.
See docs/OBS_BRIDGE.md for setup/build details.

Investigation boundary: headless library-card OS drag did not finish in Xvfb even
with all native file-drop hooks disabled. This was not caused by the control-drag
fix, was not changed, and is not claimed as verified by the new pointer fixture.
Existing synthetic native file routing tests also passed during the drag repair.

## Latest checkpoint: MIME capture groundwork; user requested pause, 15 October 2026

User said "lets stop after that for the time being" while this checkpoint was
being prepared for publication. Finish this checkpoint's normal GitHub push and
remote verification, then mark the existing goal paused. Do not continue feature
work until the user resumes. Full parity is not complete. The prior turn and this
turn both made progress; there is no blocking streak. No subagents or other-chat
messages. GitHub push remains authorized on verified JackSkellet/deckforge origin,
branch codex/streamdeck-linux, without force. CI stays manual and was not dispatched.

This increment adds only test-compiled clipboard capture groundwork, a private
reference fixture and documentation. Production restoration remains DISABLED.
Installed binary verified unchanged at
`6ffb701c82dcbd3ca3625b43b2d0af44bc15f09e34865924028cbf74a7431c29`.
No install, runtime restart, user clipboard access or saved-profile changes occurred.
Last published prior checkpoint was6e54e09. Current publication tip can be verified
from the branch/ref. Evidence/build artifacts remain ignored, not published.

Native audit now identifies actual global configuration keys:
- text_action_restore_clipboard_enabled: initializer0x14011db40 maps global
  0x141a128d0; constructor0x1403956b0 supplies false fallback.
- text_action_delay_ms_before_restore_clipboard: initializer0x14011dbb0 maps
  0x141a128f0; constructor supplies250ms fallback.
- Clone0x140373e00 copies HTML/color/image/text/URLs and remaining arbitrary formats.
  This requires full MIME preservation, not just text. Manager0x140252390 restores
  after its queue drains. No vendor runtime configuration was observed.
- Other defined keys: text_action_paste_shortcut_enabled,
  text_action_delay_ms_before_send_enter, text_action_delay_ms_between_pastes.
  Their complete UI/runtime contracts were not newly traced.
Evidence ../evidence/text-typing/clipboard-restore-analysis.json; original executable
SHA2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3 remains OPEN/HOT in
REA. Never close/reimport it. No vendor executable ran.

New src/system_clipboard/mime_wayland.rs is behind cfg(test). It connects to a local
Wayland socket with500ms bound, performs2s setup and captures within3s. One
unambiguous seat; inherited WAYLAND_SOCKET rejected to avoid sharing another
client's protocol stream. ext-data-control preferred; wlr fallback binding included.
Snapshot is tied to one offer and selection generation. Transfers use nonblocking
read pipes and memory buffers,32formats/256-byte names/16MiB total. Selection
changes reject the whole capture; timeout/cancellation discard partial data and
normal subsequent capture recovers. Manager/seat failures stay latched. PRIMARY
metadata is discarded; its contents are never requested. Per-offer invalid metadata
prevents an oversized PRIMARY offer from breaking regular clipboard capture.

New mime_reference.rs and tests/clipboard_mime_reference.py use owned virtual KWin,
private bus/runtime/config/data/TMPDIR. Socket peer PID/UID checked before access.
They prove exact text/HTML/file-URI/PNG/custom binary round-trip through temporary
product set_text and a reference restore. Independent wl-clipboard-rs read calls
mix old text and new HTML with identical format lists; new capture rejects that
race. Count limits, PRIMARY isolation, empty selection, a stalled provider,
cancellation and recovery pass. Stalling pauses only an owned child (SIGSTOP),
with RAII kill/reap and runner SIGCONT/TERM cleanup of the owned process group.

The real compositor advertises ext_data_control_manager_v1 only: ext runtime
acceptance PASSED; wlr_capture=false and wlr_protocol=null in proof, not a fake
success. X11 is not implemented. Prototype capture does not prove arbitrary provider
internals immutable; a silent empty response cannot be distinguished from legitimate
empty bytes. The16MiB guard exists but oversized-payload stress acceptance remains.

wl-clipboard-rs0.9.3 is a direct dev dependency only, along with Wayland protocol
bindings already present transitively. Its convenience writer is used only as a
reference: it writes temporary files, and per-call readers use different offers.
Do not use that combination as the production full-MIME lifecycle. PreparedCopy
queues ownership until serving; fixture readiness now checks actual types/payload,
not merely thread creation. Test artifacts: target/clipboard-mime-reference/.
Proof records source/test-binary hashes, actual backend and production_restore_enabled=false.
Latest test binary target/debug/deps/deckforge-e5842b18a61f7ea3. Real fixture passed,
existing clipboard planner test passed, all tests compiled, strict locked all-target
Clippy and formatting/diff checks passed. No frontend or release behavior changed.
No live build/test handles remain at this checkpoint.

When user resumes, finish the full restoration feature rather than declaring the
capture prototype sufficient: add memory-only owned MIME serving with ownership-loss
signals; integrate asynchronously with Text so capture never blocks held-key releases;
retain original clipboard across consecutive pastes and restore only while still
owning temporary text; preserve newer external copies; define bounded shutdown and
failed-provider behavior; implement/validate X11 and wlr; add opt-in global setting
and delay (off by default), native save/restart tests and oversized-payload checks.
Keep clipboard contents out of profile/session/log files. Current capture code can
be extended with retained manager/device proxies and data-source send/cancel events.
Never enable restoration before its write/ownership lifecycle is verified.


## Latest increment: responsive paste and GitHub publication, 15 October 2026

User explicitly said "dont forget to push changes to github". GitHub publication
is authorized for completed work in this goal. Verified origin remains
 git@github.com:JackSkellet/deckforge.git; publish on codex/streamdeck-linux with
normal non-force pushes. Preserve future remote changes. Current source commits
include38bedd1 (all accumulated parity work,243files) and6dac970 (merge origin/main
f3bc459 pinned-page exclusions plus Undo integration). The final documentation
commit follows these. Remote tip verification is part of the publishing turn;
check git refs/ls-remote when continuing. No PR created. CI remains manual-only
workflow_dispatch and was not dispatched. Private evidence, builds and state are
ignored; only project source/tests/docs/assets were staged. Credential-pattern and
large-file checks found no hits. No other workspace changes were published.

Installed SHA `6ffb701c82dcbd3ca3625b43b2d0af44bc15f09e34865924028cbf74a7431c29`;
.previous `4d9390c31dc694714114340e92a77f1f3765545b60574aeab3109fe67f1be075`.
All2,388 state/plugin/icon-pack file hashes/modes unchanged. Runtime not restarted;
save edits, fully Quit and reopen to load. Distribution archives still older.
Install proof target/paste-scheduling/install.json. Final native SequenceDrag proof
hash matches release/installed. Merged native PinOptions passed and screenshot was
reviewed. No live build/test handles remain. Goal remains active with concrete
progress this turn and no repeated blocker. No agents or other-chat messages.

Text paste now uses an interruptible150ms deadline instead of sleeping the System
worker. Clipboard writes/new presses stay FIFO, but already-owned key-ups, dial
releases/cancellation and captured-command Stop remain responsive. Pending Enter
runs only after settling; shutdown/keyboard failure suppress it. New deferred
presses retain their releases. Queue overflow reports errors and removes a matching
unstarted press when its release cannot be queued. At most32 deferred jobs, in
addition to the existing32-job channel. Captured-output completion polling waits
until paste settles because it may write clipboard; cancellation still works.

Empty Text with Enter now writes/pastes empty text before Enter (rather than keeping
old clipboard). Empty-without-Enter validation unchanged. Native KA_Text validity
0x140395cc0 and dispatch0x1403956b0 prove that case; task0x140252860 and manager
0x140252390 show timer phases/FIFO. Manager has optional clipboard restoration with
false configuration fallback, but no preference key/runtime config was established.
Linux retains clipboard replacement and prior150ms settling duration; exact original
phase timing and optional MIME restoration are not claimed complete. Native static
evidence: ../evidence/text-typing/paste-scheduling-analysis.json. REA stays OPEN/HOT.

Validation:
-Private GTK/Konsole actual System worker with test packet writer/EIS: held Ctrl
  release before paste completion, queued press/release ordering, clipboard FIFO
  across copy/twoText, Unicode, empty Enter and shutdown suppressing Enter pass.
  target/paste-scheduling/acceptance/proof.json records source/binary hashes; its
  production sources were checked unchanged after merge. No real uinput/USB or
  user clipboard touched. Existing terminal custom-shortcut acceptance passed.
-4 dial tests include active release while waiting strokes stay paused without
  a busy-spin deadline. Full merged Rust suite:778pass,23opt-in ignored; subsequent
  focused pin-exclusion Undo/Redo integration test also passes.
-Full merged frontend:263pass. TypeScript/Vite, release, strict locked all-target
  Clippy and diff whitespace checks pass.
-Native pin exclusions and Multi Action library drag/insertion/reorder/Save/restart
  pass on final binary. Synthetic WebKit events, no physical drag/USB claim.

Publication gates found4 stale frontend assertions:3 exact saved payload checks
needed the existing undo history_group UUID (now checked with UUID regex), and the
removal-error query needed its named confirmation region after an unrelated
icon-catalog error added another alert to the minimal fixture. No product behavior
or existing save assertions were weakened. Full test suite is now green.

Remote f3bc459 introduced per-page exclusions while this branch contained earlier
local changes. Resolved App.tsx by preserving current editor/drag/crop/appearance
flows and adding PinOptions in both existing pin locations and the upstream stage
label. Preserved history/icon/crop modules in host.rs; other upstream changes merged
normally. Added slot_pin_exclusions to history labels and tested Undo/Redo restoring
visibility. Incoming native pin exclusion test passes. Existing profiles untouched.

Next work remains the full OS-first parity goal. Broader Wayland/window identity,
remaining native action/reference gaps and audited non-OS gaps are still open;
physical devices are user-validated and effects/screensaver/foreign-plugin backend
work is deferred. Do not mark full parity complete or revive the disproven mandatory
vendor OS action-graph MIME contract. Keep completed changes pushed as requested.

## Latest OS increment: application artwork and quoted launchers, 15 October 2026

Installed SHA `4d9390c31dc694714114340e92a77f1f3765545b60574aeab3109fe67f1be075`;
.previous `51c606cdd37ea4ed42d1e82c4203b7b27de3299992a16f63241c97b1e21b46e5`.
Final release/installed/actual host/native/KDE regression proof hashes match.
All2,388 state/plugin/icon-pack file hashes and modes preserved. No user runtime
restart. Save edits, fully Quit and reopen to load. Distribution archives older.
Proofs target/application-artwork/{proof.json,install.json,native/,kde-regression/}.
Native restored screenshot reviewed. No live build/test handles remain. No agents,
commits, pushes, PRs, other-chat messages or memory edits. Previous goal turn
(X11) and this turn both made concrete progress. Goal remains active, no repeated
blocker. User handles physical devices; effects/screensaver and foreign plugin
backends deferred. REA remains OPEN/HOT, never close/reimport the original target.

Original KA_OpenApp image callback0x1402d0f50 selects idle/running cached images
based on process matches.0x1402cf280 derives application icon plus running-overlay
variant;0x1402d1130 loads images and subscribes to process changes. Retained static
REA evidence in ../evidence/open-application/running-artwork-native.json, from
Windows7.6 executable SHA2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3.
No vendor executable ran or overlay asset was redistributed. Initial procedure
name search returned none; explicit known vtable addresses resolved normally.

New application_artwork.rs supplies transient auto icon/running state. Reuses the
existing application_monitor thread (500ms scans) for both plugin subscriptions
and at most512 visible Open Application controls across devices. No new poller or
filesystem work on render path. Worker receives context/settings requests and
returns Arc visuals. Host rejects stale settings/contexts, redraws only changed
visuals. Config/icon cache follows visible context/settings and is pruned; desktop
entry/icon replacements need app restart. freedesktop-icons0.4.0 dependency gives
headless hicolor/pixmaps lookup; absolute icons use same bounded thumbnail decoder.
Manual exec icons require one unambiguous visible launcher; absent/ambiguous icons
keep normal background. Current desktop theme/live icon changes remain open.

Running feedback uses exact executable plus literal process-argument prefix,
including manual interpreter commands. Candidate args read from opened proc dirs
with same-user, start-time and executable checks before/after, bounded64KiB. The
shared plugin inventory otherwise keeps its original no-cmdline behavior. An
unrelated unreadable process must not suppress every indicator; matching unreadable
identity yields unknown. Unknown/no match differs from confirmed running, but this
is periodic observation, not an audit log. Multiple instances keep dot on until
last matching exit. Runtime snapshot field application_running is not persisted.

Display's auto background and Deckforge green dot respect custom/imported images.
Hardware and software use the same renderer. Native Appearance reset/custom image
previews also follow that rule. Other settings/unknown fields and key titles remain
untouched. User images never acquire the running overlay. Native screenshot has a
purple owned terminal-like SVG and green dot, not user app/device artwork.

End-to-end test exposed GIO get_executable returning a truncated quoted path:
'"/tmp/.../owned' for Exec='"/tmp/.../owned sleep" 30'. system_app::desktop_command
now parses complete commandline via existing GLib quoted-word parser. %% is literal;
first field code ends fixed match prefix. Config parsing, catalog and unambiguous
manual icon lookup use this helper. Existing real launcher unit now copies Python
to a path with spaces and executes it, verifying literal arguments and cwd. No shell
introduced. This fix applies beyond the icon feature.

Checks passed:
-10 application_monitor checks, including new candidate argument/PID generation
  check;4 plugin subscription regression checks;2 host OpenApp tests, including
  auto image/dot pixels, hardware frame, user override and stale target rejection.
-2 system_app tests, including actual quoted executable launch with literal argv.
-application_artwork_interop.py actual host, private state/data and copiedSleep:
  absolute/hicolor icons, external start/exit, several instances, argument isolation,
  changed target, restart, manual exec and byte-identical session while observing.
-native_application_artwork_interop.py actual WebKit running pixels, reset/custom
  Appearance previews and restart. Owned process survives observation; no action
  executed and no USB/user state. Final screenshot inspected.
-close_application_interop.py private KWin regression including actual Open
  activation, hold/page cancellation, Close veto/accept, helpers and explicit signals.
-Strict all-target Clippy, cargo check --tests, final release, git diff --check.
Frontend code/build unchanged; current embedded UI includes prior Multi Action DnD.

Fixture fixes: state argument is a directory, so saved file is state/session.json;
DOM selector is .deck-key img, not .key. Neither changes product assertions.
A global partial-scan warning initially made every idle app unknown; restricted
unknown to candidate identities, consistent with plugin retained-inventory rules.

Docs OPEN_APPLICATION.md and AC-01 parity row updated; OS_ACTIONS.md removes stale
wording that treated a vendor OS action-graph MIME contract as established. Original
action clipboard is in-process; broader OS MIME is optionalLinuxextension, not a
mandatory missing vendor feature. Broader Wayland window integration, launcher
wrappers/Flatpak identity, live desktop icon-theme updates and other audited parity
gaps remain. Continue against current evidence; historical handoff rows can be stale.


## Latest OS increment: local X11 application windows, 15 October 2026

Installed SHA `51c606cdd37ea4ed42d1e82c4203b7b27de3299992a16f63241c97b1e21b46e5`;
.previous `297ca9f306aab4e79a3e067b2256d9cc5f8bcbdecaeab67e8c1819d8ba74e993`.
Installed/release/X11 acceptance/native form proof hashes match. All 2,388
state/plugin/icon-pack files and modes are unchanged. Running app was not restarted.
Save edits, fully Quit and reopen to load. Proof: target/x11-windows/install.json.
Distribution archives were not repackaged. No live tests/builds remain. No agents,
commits, pushes or other-chat messages. Full parity goal remains active; this turn
made progress, with no repeated blocking condition. Device acceptance stays with
user; background/screensaver polish and foreign-plugin backends stay deferred.

New src/system_x11_window.rs uses x11rb0.13.2 with res feature and x11rb-protocol.
Local Unix DISPLAY only; Wayland remains authoritative when WAYLAND_DISPLAY is set.
A bounded Stream gives protocol I/O a two-second deadline and connect a 500ms bound;
no detached request can send later. Authority-file reading follows x11rb's standard
no-auth-info fallback; the server still enforces authentication. Local file reading
is not covered by the protocol I/O deadline. EWMH WM self-check, advertised typed
client inventory capped at4096 and XRes>=1.2 are required. Missing/malformed
inventory is an error, never a successful empty inventory that permits SIGTERM.

XRes LOCAL_CLIENT_PID verifies actual window owner instead of trusting _NET_WM_PID.
Process pidfd/executable/argument identity and window ownership are rechecked before
dispatch. Normal/dialog windows only, respecting the first WM-supported type in
the ordered type list (Qt utility windows advertise NORMAL as fallback). Matching
unsupported windows still count as found, so Auto never mistakes them for headless
processes. Close sends only advertised WM_DELETE_WINDOW, never XKillClient or a WM
close request that could kill an unsupported client. Applications can veto/show
save prompts. Status reports requested count of found windows, not guaranteed exit.

Activation uses _NET_ACTIVE_WINDOW with source2 (direct user action). When supported
and valid it first requests _NET_CURRENT_DESKTOP for the target's workspace; sticky
windows don't change workspaces. Openbox required this explicit switch; activation
alone restored minimized windows but stayed on the previous workspace. Final actual
acceptance passes both. Pinned activation supports self; signals explicitly reject
self. KDE inventory now captures all matching windows but dispatches only eligible
normal/dialog stable UUIDs after process recheck, preserving helpers/utility windows.

Verification:
- tests/x11_window_interop.py: owned Xvfb/Openbox, real Qt main/dialog/helper/utility
  processes. Minimized/workspace activation without duplicate, close veto/accept,
  held Open close, spoofed PID isolation, missing protocols/inventory/WM no-signal
  behavior, explicit force/SIGTERM, and absent authority-file support pass.
- tests/close_application_interop.py: private KWin regression including Open
  Application activation/hold/page cancellation passes. This proof precedes only
  X11 type/auth fixes; unchanged KDE code. target/x11-windows/kde-regression/.
- Native Close form/restart proof on final binary, screenshot reviewed; no actions
  executed, USB or user-window acceptance claimed. target/x11-windows/native/.
- 4 process,2 application,1 Close focused Rust checks pass; 23Fields checks and
  TypeScript/Vite pass. Strict all-target Clippy passes after final production edit.
- Tools extracted locally without system install; target/x11-windows/tools/provenance.json
  records Openbox/startup-notification package hashes/source. Xvfb provenance remains
  under target/x11-layouts/tools/. No user display used for X11 acceptance.

Docs CLOSE_APPLICATION.md, OPEN_APPLICATION.md and parity AC-01 updated. Other
Wayland window backends, launcher executable indirection and running-state artwork
remain open. Multi Action drag/drop below remains included. REA target remains
OPEN/HOT: ../evidence/elgato-7.6/raw/AppExecutable, SHA2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3.
Do not close/reimport it, or revive the disproven mandatory vendor OS action-graph
clipboard MIME requirement. Use current native evidence and latest parity rows
when selecting the next OS increment; older handoff sections are historical.


## Latest user request completed: Multi Action drag/drop, 15 October 2026

User steered the active goal with screenshot
/tmp/codex-clipboard-9345c6b9-e2e3-41f4-851e-0ee6062eac65.png: Multi Actions should
accept library drag/drop like the main canvas. Prioritized and implemented that.
The X11 window activation/close investigation continued after this request;
see the newer OS increment above. Full parity goal stays active.
No subagents, commits, pushes, other-chat messages or user runtime restart.

Installed SHA `297ca9f306aab4e79a3e067b2256d9cc5f8bcbdecaeab67e8c1819d8ba74e993`;
.previous SHA `349192ebcf82fc17a497f67a48e862b93eb2d104d8cd0b5a6568d239c9483b4c`.
Exact installed/release/native proof hashes match.2,388 state/plugin/pack files
preserved. User should save edits, fully Quit and reopen to load the new UI.
Proof target/sequence-drag/install.json. Empty/populated native screenshots reviewed.
No live build/test processes remain. No backend behavior changed in this increment.

Sequence.tsx previously accepted library drops only for Action Wheel and fixed
Trigger/Logic slots. Shared list containers now accept library UUID payloads for
append or insertion (upper/lower half of a row). Multi/Switch/Random, Wheel and
Stack use existing childAllowed/controller/plugin checks. New children use newChild,
so defaults and identity behavior match the existing dropdown. Empty editor has a
visible Drag actions here area; populated lists have an end drop target. Insertion
marker and edge scrolling in inspector-fields give destination feedback.

Dotted button handles reorder existing steps by splice, preserving complete raw
records/IDs/settings/resources. Step payload is separate from library-copy payload,
contains owner/useId + branch/index, and requires a local drag reference with an
unchanged entry signature. Cross-container/branch or stale step moves are rejected;
existing Copy/Cut/Paste remains the route for cross-container transfer. Native GTK
standard text/plain drag feedback is included, like the existing library source.
Dropdown/Add and Move up/down keyboard controls remain. No changes persist until
Save. Read-only and disabled ancestor fieldsets block drops/dragstart, including
saving/editBusy. Known nested drags stop propagation. Trigger/Logic keep slot
replacement; their blank editor areas reject drops instead of bubbling into an
outer sequence. Parent/child selection, runtime Stop guards and clipboard flows
retain existing rules.

Tests: existing10 Sequence +6 SequenceCommands tests pass; new4 SequenceDrag tests
cover insertion/reorder preserving IDs/data, incompatible actions, stale/disabled
edits, switch branches, nested isolation and gesture-gap rejection. TypeScript
check and UI build pass; release built. Native_sequence_drag_interop.py tests actual
library dragstart custom/text payloads, DataTransfer/DragEvent insertion and handle
reordering, explicit Save, exact child-record preservation and restart. Three phases
empty/edit/restart; no actions executed, USB or user state. These are synthetic
native WebKit events, not recorded physical pointer drags. Reports/screenshots in
 target/sequence-drag/native/. New docs/SEQUENCE_DRAG_DROP.md and parity row AC-05.

Native fixture fixes: zone() initially dereferenced list before render; changed to
optional lookup. Save wait initially stopped at backend mutation while UI was still
Saving; handle was correctly disabled and dragstart cancelled. Wait now requires
clean disabled Save changes button and enabled editor fieldset. Unit fixture used
wrong literal Trigger UUID; replaced with exported TRIGGER constant. No product
checks/assertions weakened. New fixtures now have fully typed Action data. Avoid
rerunning all unrelated backend tests for this UI-only change.

The X11 research described here was implemented in the subsequent OS increment.
See the current entry above and docs/CLOSE_APPLICATION.md for verified scope.
REA target remains OPEN/HOT. All user priority/deferred scope boundaries remain.


## Latest OS increment: legacy modifiers and Text paste editor, 15 October 2026

Installed SHA `349192ebcf82fc17a497f67a48e862b93eb2d104d8cd0b5a6568d239c9483b4c`;
.previous SHA `71b6a78b39ca9ff83e68c142660264e3ecd9b92fa9e90ab0cab135ea2f6a32ad`.
Final native/settings proofs match release and installed SHA. Four-layout actual
key-delivery source hashes match current runtime files. Installation preserved
2,388 state/plugin/pack files. Native screenshot inspected; no user runtime restart,
keyboard input or USB. Proof target/shortcut-legacy/install.json. Full goal active;
this turn made progress. No blockers, subagents, commits, pushes or active processes.
Save/full Quit/reopen loads the new build.

Default audit found a concrete native compatibility bug. Original Hotkey loader
0x1402bb4e0 uses missing KeyModifiers or exact 0x2000 (8192) as legacy-format marker,
then reads KeyCmd/KeyOption/KeyCtrl/KeyShift booleans. With any other mask it skips
those boolean reads. Previous Linux decoder rejected explicit8192 and contradictions.
Now system_keyboard::effective_modifiers centralizes the observed rule. Runtime
chords, UI checkbox states and dial30/50ms timing all use effective modifiers.
Modern masks win even over stale/invalid dormant boolean fields. Active legacy
non-booleans remain errors. Existing portableQt/NativeCode policy is unchanged;
this does not magically translate arbitrary old platform scan codes.

Frontend renders legacy8192 as normal portable key + booleans. Editing a modifier
or key deliberately writes a modern mask, preserving unknown settings. Editing only
mapping or unrelated fields preserves original dormant booleans and legacy format.
Removed misleading unconditional Linux-key-positions hint. Legacy modifier-only
sentinel and right-sided modern masks with stale booleans pass actual Qt delivery.

Broader default-policy decision: do NOT blanket-convert native/general Hotkeys to
logical characters. Windows resolver0x14059e1c0 prefers VKeyCode, and label function
0x14059dda0 consults current Windows layout. StoredQt codes do not establish one
uniform Unicode interpretation for OEM/virtual keys. Physical/logical remains an
explicit general-shortcut choice; absent saved/imported modes stay physical.
Evidence ../evidence/shortcut-legacy/native-analysis.json. REA target remains
OPEN/HOT; no vendor app was executed. Do not repeatedly reopen this default question
without new evidence; the conditional interpretation is now documented.

Safe new defaults apply only to fresh Clipboard and standard Text-paste actions:
root linuxKeyMode logical. Existing sessions/imports remain unchanged. Text standard
CtrlV fallback now reads the root mapping unless an empty customHotkey explicitly
supplies its own mapping. A valid older custom chord retains its own physical/default
mapping even when the root is logical. Typing mode ignores dormant paste settings.
No action_defaults.rs import/export migration was introduced in this increment.

Text now exposes expandable Paste shortcut editor using existing Hotkeys control.
It renders implicit CtrlV without writing a Hotkey until edited. It supports key,
modifiers, side choices and mapping, including CtrlShiftV for terminal paste. Reset
paste shortcut deletes the custom record and restores implicit CtrlV/root mapping.
Switching to typing retains the custom record but hides the editor. Non-object
custom data is retained with a truthful fallback explanation until reset/replaced.
Text hides the generic Clear button (actual clearable=false variant) to avoid a
redundant control beside Reset. General Hotkey Clear remains unchanged. No new
checkbox/parallel enabled state or schema flag was added.

Verified13 keyboard Rust tests,17 System tests (including legacy/modern dial timing
and new/legacy/custom paste-default branches),23 Fields tests, typecheck/build,
strict all-target Clippy. Four-layout logical acceptance now includes legacy8192
Ctrl, modern mask with stale false/true booleans, rightCtrl precedence and legacy
modifier-only dispatch, plus existing shared ownership/layout/dial/failure checks.
Dvorak GTK/Konsole acceptance passes with standard Text root mapping and legacy
custom CtrlShiftV. Test-only focused fixture changed to exercise root mapping;
no runtime changes occurred after those tests. Latest debug test artifact remains
 target/debug/deps/deckforge-1e2c6c9aea2eeeec.

New native_shortcut_legacy_interop.py verifies legacy rendering/edit-to-modern,
modern mask display without rewriting dormant fields, fresh Text logical defaults,
CtrlShiftV editing, typing/paste transitions, Reset and restart. No actions executed.
Actual host logical_settings_interop.py now covers eight controls, including legacy
Hotkey/Text records, asserts fresh Clipboard/Text defaults, and verifies unchanged
saved data through export/import/restart. Original mode/unknown values are retained.
Native and settings reports target/shortcut-legacy/; backend proof remains
 target/logical-hotkeys/proof.json. Screenshot shows one Reset button, no generic
Clear inside Text. All final proof hashes were checked before install.

Next concrete OS gaps include X11 bring-to-front/normal Close (currently KDE scripts
only), other Wayland desktop/per-device integration, and broader original action
behavior. Avoid endless tiny mapping-default changes now that the evidence-based
policy is defined. Existing action clipboard is in-process; vendor OS MIME contract
is not established, so that remains optional Linux extension rather than mandatory
native parity. User handles physical devices; backgrounds/screensaver polish and
foreign-platform plugins remain deferred. Full parity is not complete. Distribution
archives remain older; no packaging/PR/publishing in this increment.


## Latest OS implementation: X11 logical layouts and cancellable lookup, 15 October 2026

Installed SHA `71b6a78b39ca9ff83e68c142660264e3ecd9b92fa9e90ab0cab135ea2f6a32ad`;
.previous SHA `9e57e0a80ce660183d79e8ab0bb2515aa18c070d10742d15b09ced3f448c673e`.
Exact native editor/settings proofs match release/installed SHA. X11 backend proof
source hashes match runtimefiles.2,388 state/plugin/pack files preserved; no user
runtime restart/input/USB. Native screenshot inspected. target/x11-layouts/install.json.
Fullgoalactive; thisturnprogress, no blockers/subagents/commits/pushes. No live
build/test handles remain. FullQuit/reopen loadsupdate.

keyboard_layout.rs now supports X11 with authenticatedXCBconnection, XKBextension,
corekeyboarddevice map/effectivelayout snapshot. xkbcommon-dl0.4+x11 isdirectdependency
(existing0.4.2 inlock); xkbcommon-x11 loads optionally atX11runtime, missinglibreturns
expliciterror. libxcb linked (already standardGUIdependency). WhenWAYLAND_DISPLAY
present, Wayland remainsauthoritative; no silentfallback topossiblydifferentXwayland
map onKDEfailure. NoKDErequired onX11. Existing keymapresolver/physicalownership reused.
UIhints now state X11 orKDEWayland; mode defaults unchanged (physicalunlessexplicit).
Systemkeyboardmoduleheader clarified portabledecode versuslogicalChordmapping.

Newsingleprocess-global metadata-only lookupworker uses boundedqueue1, owns all
blocking/native mapqueries and sendsONLYresolvedcodes. Inputworker deadline500ms,
cancellationpoll5ms andcheckbeforeusing result. Atmostone active andonequeuedquery;
expiredrequests skipped andlate resultsdiscarded. A stucknativecall canoccupythisone
thread untilitreturns butcannotblockinputworker indefinitely orspawnmorethreads.
Noexternalhelperprocess installed. Staticworker endswithprocess; XCBresources owned
insidejob andfreednormally. Tests prove readerrecovery afterstalleddisplay resumes.
Wayland retains150msindividualI/O limits, nowinsideboundedreader.

Keyboard nowbinds Systemworker's sharedstopAtomicBool. press_chord checksstop before
mapping, resolver checkswhilewaiting/afterreply. This fixes possiblelatekeypress
afterStop duringprevious synchronouslookup. Releases ignorecancel soownedkeys still
clean up. prepare_keyboard bindsflag afterconstruction; injectedtestkeyboard bound
atworkerstart. Existinginitialuinputregistrationgrace remainsseparate (notchanged).

No Xvfb installed, so fetched configuredCachyOSpackage URL frompacman -Sp andextracted
ONLY usr/bin/Xvfb under target/x11-layouts/tools/. No pacmaninstall,rootconfigor user
services changed. Archivehash/packageidentity in tools/provenance.json. Version
21.1.25-1.1 x86_64_v4; libdependencies resolvedonhost. Xvfb -displayfd choosesfree
server; no lockfile generated inthismode, sofixture validates UnixSO_PEERCRED
PID/UID instead beforeany setxkbmap/input. Stronger kernelidentity guard, not weakened.

New tests/x11_keyboard_bridge.cpp forwards actualinput_eventframes viaXTest onlyto
exactownedXvfbPID. Productionusesuinput, neverXTest. tests/x11_layout_interop.py checks
US/de/no/fr activegroups,logicalY/punctuation/digit,RightCtrl, physicalopt-out GermanY
->Z, heldreleaseacrossgroupchange, livekeymapreplacement toDvorakCtrlV. Itthenruns
actualSystemworker GTKcopy/cut/paste,UnicodeText/Enter andKonsole/customTextpasteunder
Dvorak. Focus confinedtoownedX11windows; terminalwindow selectedby_NET_WM_PIDmatching
ownedKonsolePID. PrivateD-Bus/config/data/runtime andemptyXauthority. DBusactivation
DISPLAY/XAUTHORITY updatedONLYonprivatebus; XDG_CURRENT_DESKTOP GNOME avoids invoking
userKDE-onlybackend. No userdesktop inputor realuinputused.

The samefixture has anownedlocalTCPproxy that acceptsX11handshake andpausesbefore
forwardingtoownedXvfb. Newignored layout_lookup_failure_on_private_x11 in
system_actions/focused_clipboard_test.rs proves bothStop anddeadline: cancellation
returns promptly(measured0msrounded, bound250ms), timeoutcompletesabout550msincluding
workercleanup (lookupdeadline500ms). No keypacketssent. Proxythenresumes, lateoldquery
returnswithoutinput, andfreshlookupreusesreader successfully. Neverclientinput
throughproxy; itonlyinterceptsmetadataqueries fromcontrolledtestprocess.

Existingprivate_root testgate now permitsX11 onlywithlocalnumericDISPLAY, noWayland,
exactruntime/ownedroot andserverPID/UID verifiedbyUnixpeercredentials. Waylandgate
remains exactprivate socket. Focusedclipboardtest outputfile changedcreate_new to
append/create becauseX11suite alreadyhascapturedframes; existing-byte baselines still
proveCopyText emitsnone. Backendreport nowcorrectlynames XvfbXTestorKWinEIS. Initial
X11cliptest failed Fileexists, fixedfixtureonly. A priorprivateportalactivation lacked
DISPLAY andcrashed; updatedprivateactivationenvironment andgenericdesktopfixture.
No production clipboardfallback orassertions weakened.

Verified12keyboard,15Systemworker,2keymap Rusttests;20FieldsUI;typecheck/build;
strictalltargetClippy. FinalX11acceptance includesallabove andsourcehashes. KDE
4-layoutlogical/sharedmodifier/dial/failure regression andDvorakGTK/Konsoleclipboard
pass afterreaderchange. NativeWebKitSave/restart andactualhostexport/import/restart
pass onfinalrelease. NativeUIruns onuser'sdesktop withprivate simulatedstate, no
shortcuts executed. Finaltestbinary target/debug/deps/deckforge-1e2c6c9aea2eeeec;
olderacefa68d... isstale. Reports target/x11-layouts/,target/logical-hotkeys/.
SomeWaylandproofs preceded later test-only fixturegate/append additions; runtime
codeunchangedafterthosechecks. No redundantretestingneeded fordocs alone.

RemainingOS priorities: native/newaction logicalmappingdefault policy (stillexplicit
opt-in), otherWaylandcompositors/per-device maps andphysicaluseracceptance. X11core
map assumption verifiedonXvfb, notall unusualXInputdeviceconfigs. Existingnative
clipboard is inprocess; no vendorOSactiongraphMIMEcontract established (see prior
scopecorrection), so do notinvent that asmandatoryparitygap. Fullgoalnotcomplete.
Background/screensaverpolish andforeignplatformplugins stilldeferredbyuser.
REAoriginaltarget remainsOPEN/HOT; no nativeanalysisneededthisincrement.


## Latest OS acceptance: locks, external device modifiers and repeat, 15 October 2026

This turn made progress by closing controlled software-acceptance gaps. No production
code/build/install changes were needed. Installed SHA rechecked:
9e57e0a80ce660183d79e8ab0bb2515aa18c070d10742d15b09ced3f448c673e. Goal remainsactive,
no blockers/subagents/commits/pushes/runtime restart/user input. No live testprocesses
remain. REAoriginaltarget still OPEN/HOT. Do not reimport/close.

New tests/keyboard_state_interop.py reuses actualRust imported_chord/resolver/writer
through test-only EISrelay, plus a SECOND separate ownedvirtualkeyboard with its own
packetfile. Bothverify exactprivateKWinPID. PrivateQt records scan/key/modifiers/text
and isAutoRepeat. US/de/no/fr allpass CapsLocktoggle,preservation,Shiftinversion,
digit1; NumLockchanges physicalKP7 digit/navigation andrestores it. Left/rightCtrl
andShift held byseconddevice affect Deckforge keys; Deckforge also pressing/releasing
same modifier doesNOTclear externaldevice state. Releasingexternal restores plainY.
German/Norwegian externalAltGr/share/release also pass. FinalheldY test observes19
repeatpresses during1.3s; heldCtrl doesnotrepeat; exactlyone finalnonrepeatrelease
and noevents afterrelease. NoextraLinuxrepeattimer needed. Physicalhardware isNOT
tested; userowns thatacceptance. Reports target/keyboard-state/proof.json pluslogs,
27cases us/fr and31cases de/no. Sourcehashesmatchcurrentruntimecode; currenttestbinary
acefa68d9e821d49. Do not rebuild/reinstall merely for these test/docs changes.

OriginalKA_HotKey staticpaths 0x1402bc670 pressmods thenkey and0x1402bc710 releasekey
thenmods. Noexternalmodifierreset inthese tracedpaths. OShelpers0x14059d990/59d9f0
read Windowsrepeatdelay/speed. Evidence exported ../evidence/keyboard-state/
native-dispatch.json andnative-os-helpers.json. OShelper0x59d920 wasmousewheel,
notamodifierreset. ThisisstaticWindows evidence, notphysicalWindowsruntimeacceptance.
New docs/KEYBOARD_STATE.md plusupdatedlogical/modifier/paritynotes distinguish
virtual-device evidence fromphysicalhardware. Do not generalize toallcompositors.

Scope correction from authoritative docs/NESTED_CLIPBOARD.md: originalactionclipboard
is INPROCESS (ESDActionClipboard). Noportable vendorQClipboardMIMEcontract hasbeen
established. Olderhandoff entriescalling originalOSactiongraphMIME an obligatory
missingvendorfeature overstateevidence. Itis an optionalLinuxinteroperability
extension unless newnativeevidence provesotherwise; do notinventit tofillparity.
The userrequestedOSclipboardwork alreadyhas realtextclipboard acceptance. Fullgoal
stillhasotherconcretegaps, so keepactive ratherthanredefinecompletion.

Next usefulOS work: decide native/newHotkey logicalmappingdefaults from original
contract whilepreserving legacyphysicalsavedprofiles; currentlyallnew/importedmissing
modes stillphysical anduserexplicitopt-in logical. Existing action_defaults.rs has
Native/Linux boundarypolicy forText/Close thatcanbeextended onlyaftervalidating
hotkeysourceOS/portableQt semantics. Newdefaultmustnot silentlybreakunverifiedother
desktops; explicitunsupportedbackenderror isbetterthanwrongkeys butcrossdesktop
policyneedsconsideration. Otherdesktop/per-devicekeymaps remainsoftwaregaps.
KeepuserpriorityOS, deferbackground/screensaverpolish andforeignpluginplatformwork.
Physicaldevicevalidation belongs touser, notablockerfor softwareprogress.


## Latest OS implementation: logical shortcuts installed, 15 October 2026

Installed SHA `9e57e0a80ce660183d79e8ab0bb2515aa18c070d10742d15b09ced3f448c673e`;
.previous SHA `6dc3ad54a59563adc30daf55bee14a93cccd7ae657fc8d2721f5f42a12360711`.
Native editor and actual-host settings proofs match exact release/installed SHA.
Private logical acceptance source hashes match current runtime files. Installation
preserved2,388 state/plugin/pack files, no real runtime restart or user input/USB.
Native screenshot inspected. Proof target/logical-hotkeys/install.json. Full goal
active, this turn progress, no blockers/subagents/commits/pushes. No live build/test
processes remain. Save/full Quit/reopen loads update.

Actual implementation now exists in new src/keyboard_layout.rs. A bounded snapshot
reader connects to Wayland without a surface, obtains one unambiguous keyboard seat
and <=1MiB map through private read-only mmap, queries active KDE layout group, then
uses small libxkbcommon FFI to resolve supported printable identities. Static
xkbcommon linkage is added; it was already a GUI runtime dependency. Cargo now has
direct wayland-client0.31 (existing lock version0.31.15). No extra portal permission,
EIS backend, persistent thread/cache or keyboard owner. Snapshot choice is supported
by measured3.25–3.96ms press resolution across tests. Connection/map/query I/O each
bounded150ms; nativeXKBparsing is synchronous, not a hard overall deadline.

system_keyboard::Chord {keys:Vec<u16>,logical:bool} carries metadata through System
Plan Chord/Press/Repeat/Paste and Dials Stroke/Event. imported_chord wraps existing
portableQtdecoder and strict linuxKeyMode parsing. imported_hotkey still returns
physicalVec for its existingdecoder tests. Keyboard::press_chord resolves at actual
press, then existing Ownership captures concretephysicalkeys. send_chord reuses it.
Release never queries layout. Dials resolves every queuedstroke at emission, not
queuecreation. No sentinel keycodes or duplicated ownership. Existing keyboard error
cleanup still closes/releases all ownedkeys; failed layout emits no physicalfallback.

Printable mapping covers existing supported letters/digits/punctuation. Nonprinting
and numericpad identities remainphysical. XKBlevels up to8 and modifier masks up to16
are searched for supportedevdevcodes. Adds only Shift/Mod5(AltGr) when the map proves
LeftShift/RightAlt have those roles; existingRightShift prevents addingLeftShift.
Missing symbols/maps/group/service and multi-seat are expliciterrors. Currentcode
still doesn't support arbitraryUnicodehotkey identities or arbitrarymodifierroles.
Per-device custommaps, externalphysicalmodifiers, Caps/NumLock and nonKDE/X11 need
acceptance. Tightpoll interruptions currently produce expliciterrors (no unbounded
retry); not observed in tests. Physicaluinput discovery remains user acceptance.

UI Hotkeys component adds Key mapping selector and KDEWaylandhint, shared by hotkey,
switch,dial,customTextpaste. Clipboardadds Shortcut key mapping atrootsettings.
Perhotkey linuxKeyMode lives inside eachHotkeysentry/customHotkey; Clipboards atroot.
Modes physical/logical; absent preservesphysical. Newassignments/imports also retain
legacyphysicaldefault for now; explicitopt-in is installed. Decide native/default
boundarybehavior separately with originalevidence and portability, never silently
migrate saved sessions. Key changes preserve mode and modifiers/unknownfields.
Unknownfuturemode remains stored/displayed and runtimeerror. EmptycustomTextHotkey
uses defaultCtrlV with its selectedmode. Texttypingportal unchanged.

Verified:12keyboard,2keymap,3dial,15Systemworker,1Clipboard Rust tests;20Fields UI
checks; typecheck/build and strictalltargetClippy. Changed nativeSystemnext selector
to locate Key labels instead of selectindex after newmappingselector. Other tests'
Vecconstructors updated to .into() typedphysicalChord; assertions not weakened.
Keymapunits cover GermanY, FrenchShift+1, DvorakCtrlV, sidedkeypad, groups, malformedmap,
Russian-only unavailableLatinsymbol. Malformedmap test intentionally logs XKBerror.

Actualprivate tests/logical_hotkeys_interop.py usesproductionRust decoder/resolver/
writer andSystemworker through test-only EIS relay intoownedQt. US/de/no/fr: logicalY,
Ctrlleft/right/both,Shift,AltGr,slash/backslash/digit1, heldrelease acrosslayouts,
sharedCtrl ownership, nextqueuedDialstroke mappingafterlivegroupchange, nofallback
whenWaylandsocketmissing. Qtphysicalscans/modifiers/text checked. Newignored
logical_worker_on_private_wayland sits in system_actions/focused_clipboard_test.rs;
controlled250msdialinterval allows precisegroupchange; nativeinterval behavior still
covered by existingDials suite. Gatedsided_hotkeys test nowuses imported_chord and
writes resolutiontiming/physicalkeys; optionalfilebarrier testsheldrelease. Fixture
initialwait mistakenlyincluded an alreadyfinished ownedchild; removed it fromlive
waitset without weakening outputchecks. Allfinaltests pass.

ActualSystemworker focusedclipboard acceptance also passes underUS Dvorak using
--logical --layout us --variant dvorak on focused_clipboard_interop.py. Copy/Cut/Paste,
UnicodeText/Enter, realKonsolepaste andcustomTextshortcutwork; no realuinput/userinput.
This validates clipboardpositions beyondQWERTY. Newnative_logical_hotkeys_interop.py
saveslogicalmode withrightAlt/bothCtrl, editsmainkey, restartpreserves all. Actualhost
logical_settings_interop.py export/import/restart preserveslegacyabsent,logical,
physical,unknownfuturemodes, opaquechord/rootfields, Clipboard/Textcustommodes; no
sessionrewrite onexport and no keyboardpermission/actions. Reports target/logical-hotkeys/.
Dvorakproof predates subsequent test-only ownership/timing fixture additions; no
runtimealgorithm changes occurred afterward. Nativeproof matchesfinalrelease.
Current debugtestbinary target/debug/deps/deckforge-acefa68d9e821d49 (dependencychange
replaced f8f354d2c0ef1b7c as newest artifact). Do not pick stale testbinary byname.

Next OS work should close remaining evidenced requirements: native import/newaction
mode defaults vs originallayout semantics, lock/externalmodifier acceptance and
otherdesktopbackends, or original actiongraph systemclipboard MIME interchange.
User still owns physicaldevicevalidation; backgrounds/screensaverpolish and
foreignplatformplugins deferred. Fullparity not complete. Distributionarchives not
rebuilt. REAoriginaltarget remains OPEN/HOT; no nativeanalysis needed thisincrement.
Historical prototypes and corrected EISinitialstate limitation are retained below;
read latestimplementation first. docs/LOGICAL_SHORTCUTS.md is current userguide.


## Latest OS progress: public Wayland keymap verified, 15 October 2026

Correction from final proof inspection: EIS DOES send state after modifier/group
changes; only initial state was absent. Earlier section below overstated this.
Final Wayland-reader probe now forces every group lookup through KDE D-Bus, so it
really proves independence from EIS state. EIS-only probe records initial/final
state sources explicitly. Final independent Wayland and EIS-only reruns passed after forcing the independent
state source and recording initial/final sources.

This turn made concrete progress with a passing unfocused Wayland keymap reader.
No production behavior or installed binary changed. Previous installed SHA remains
6dc3ad54a59563adc30daf55bee14a93cccd7ae657fc8d2721f5f42a12360711. Goal active,
no blocker. No subagents, commits, pushes, user input, USB or permission changes.

New tests/wayland_keymap_probe.c connects only to private wl_display/registry/seat/
keyboard, creates no surface, copies bounded8MiB XKB keymap, closes all objects/fd,
and fails if it receives enter/key events. It checks owned /tmp/deckforge-focused-*
and exact private XDG_RUNTIME_DIR. It uses bounded read-only MAP_PRIVATE mmap
without changing the shared fd offset. The common C++ EIS relay now also uses
MAP_PRIVATE for keymap capture. Built using pkg-config wayland-client1.26.0.
The extended tests/eis_keymap_probe.py accepts --wayland-reader; it uses that map
instead of the EISmap and records keymap_source and source hashes. Public EIS is
still used only as the owned test transport. Keymapreading itself requires no extra
portal session/permission or focusedwindow. Current tests all pass forUS/de/no,
CtrlY,RightCtrlShiftY, capturedrelease acrosslayouts andswitchback. Report
 target/wayland-layouts/proof.json; helper target/wayland-layouts/keymap-reader.
Command: python tests/eis_keymap_probe.py --wayland-reader
 target/wayland-layouts/keymap-reader --output target/wayland-layouts.
No live test/build process remains from that run.

This removes the main reason to add a second EIS permission/backend. Next actual
production implementation should reuse existing system_keyboard::Keyboard physical
writer/Ownership, resolve logical keys before press using read-only Waylandkeymap
and verified activegroup, then store concretephysicalcodes in existing holdtokens.
KDE getLayout works/read-only; no EISgroup-events assumed. Release must notresolve
again. Existing Systemworker and Dials remain timing/ownership authorities. Need
explicit typed logical/physical chord metadata through Plan and Dials rather than
magic sentinel keycodes. Existing imported_hotkey returnsVec<u16>, Plan::Chord/
Press/Repeat and system_dial::Stroke/Event useVec<u16>. A Chord struct containing
keys+mappingmode is a plausible small interface; preserve old physical records and
provide deliberate UI mode. Native/new logical defaults require explicit import
rules later, not silently rewriting sessions. Avoid generic trait frameworks.

Publicmap decoding/resolution currently exists only as a base-levelY ctypes test;
production needs full supported key table, shifted symbols/numberrows, keypad/OEM,
AltGr/modifiers, invalid/missingmap/group errors and bounded time/resource behavior.
Do not claim the whole feature delivered becauseYworks. Consider a bounded snapshot
reader vs persistentcachedreader based on measuredlatency; do not block key-release
jobs behind long compositorqueries. Resolve at actualpress, including queued dial
strokes, not when enqueuing. Capture physicalidentity for allreleases. Existing
uinput permission/discovery boundaries remain unchanged. Wayland-client0.31.15
and xkbcommon-dl are alreadytransitive lockfile deps, not direct Cargo dependencies.
Local wayland-client examples and libxkbcommon headers available if implementing.

Previous portalkeysymheldrelease defect and publicEISproof remain authoritative
below. Updated docs/LOGICAL_SHORTCUTS.md with newly verified alternative. No install
needed for test-only progress; do not ask user to restart for this increment.


## Latest OS investigation: logical shortcut backend, 15 October 2026

This turn is progress: working private-desktop prototypes and a reproduced backend
release defect change the implementation decision. No production behavior installed
or rebuilt. Installed binary remains6dc3ad54a59563adc30daf55bee14a93cccd7ae657fc8d2721f5f42a12360711,
with previous c576f86c4fc969b263afa6d2c31820fa2d43066a464378dba24004d5dbf15bdd.
Goal remains active, no blocker. No user input/USB/runtime restart, commits/pushes,
subagents or other-chat messages. codebase-design skill was read/applied to keep
ownership and layout management centralized, not spread among action handlers.

Current installed modifiers work remains as described below. New cfg(test)-only
portal_shortcut_probe_on_private_wayland in system_typing.rs uses existing Portal
setup/key methods plus test-only physical-keycode and private-layout-change requests.
It validates /tmp/deckforge-focused-* and private runtime, bounded100events, closes
its session after taking an events snapshot. tests/portal_shortcut_probe.py runs
US/German/Norwegian private KWin+Qt, private verified PermissionStore; real public
RemoteDesktop API. PureY,CtrlY,ShiftY,CtrlShiftZ,RightCtrlY and AltGr work. BUT when
layout changes between keysym press/release, original key has no release until
Session.Close. The before/after-close client snapshots prove this; ordinary full
post-cleanup event balance alone was misleading. target/portal-shortcuts/probe.json
now calculates assessment, and assessment.json retains the earlier explicit audit.
Do NOT use NotifyKeyboardKeysym for persistent held shortcuts. Original stable
Text typing behavior was not changed.

Primary source https://raw.githubusercontent.com/KDE/kwin/Plasma/6.7/src/backends/fakeinput/fakeinputbackend.cpp
corroborates per-event keysym re-resolution. Invent commit URL returned403; GitHub
raw official mirror worked. Installed kwin6.7.5-3.1, portal-kde6.7.5-1.1,
xdg-desktop-portal1.22.1-2.1, libei1.6.0, xkbcommon1.13.2. Never infer current
behavior only from primary source; owned-client probe is authoritative here.

Alternative tests/eis_keymap_probe.py PASSES actual public ConnectToEIS. PythonGio
requests keyboard-only session and passes returned fd to existing C++ test relay.
The relay now accepts optional DECKFORGE_PRIVATE_EIS_FD, still requires exact private
KWin PID and owned temp root, captures provided XKB map and any EIS keyboard-state
messages. Without envfd it retains the previous direct private KWin EIS test path.
C++ files are test-only. EIS keymap capture is bounded8MiB and fd borrowed with QFile.
Actual key resolution uses libxkbcommon ctypes in the prototype, base level0 and
currentgroup only; it is NOT a complete key resolver. ResolvesY once perpress,
physically sendsCtrl/RightCtrl/Shift, releases capturedcode across layoutchange;
all3layouts andswitchback pass. Ctrl/Shift mods explicitly asserted inQt traces.
Evidence target/eis-layouts/proof.json (prototype_only true, source hashes).

KWin did NOT send EIS keyboard modifier/group events in these runs (initial wait
failed). The prototype explicitly falls back to read-only KDE D-Bus getLayout on
org.kde.keyboard /Layouts org.kde.KeyboardLayouts; no default-group guessing. Source
recorded in each proof. Queries on user's session were introspection only; all
setLayout calls target privatebus. This fallback works for actual input through
EISkeymap on live groupchanges. Other desktops not proved.

The public portal contract forbids mixing Notify methods after ConnectToEIS on
one session. Existing Unicode Text relies on NotifyKeyboardKeysym; tested KWin EIS
has no Text capability. Do NOT switch that session and break Unicode, or naively
add parallel keyboard ownership. Before productionintegration, next concrete option:
probe a separate public Wayland wl_keyboard keymap reader (no surface/input capture)
plus read-only KDEactivegroup, then use captured physical keycodes through the existing
portal. If verified this avoids an extra EIS permission/session and dependency while
retaining UnicodeNotify in same session. Wayland keymap availability without focus
is not yet proven. Existingtransitive wayland-client/xkbcommon-dl inCargo.lock, not
directdependencies. Alternatively integrate publicEIS deliberately with separate
sessionpermissionlifecycle. See docs/LOGICAL_SHORTCUTS.md for constraints/gates.

Keep same Systemworker ownership, dial deadlines, FIFO, cancellation, page-disconnect
release. Logical-vs-physical saved semantics must be explicit; never rewrite old
physical profiles silently. Capture device/keycode onpress and release sameidentity.
Keymap/device replacement cancels affectedholds. Need fullsymbol resolver beyondY:
numberrows/shiftedpunctuation/keypad/OEM, Caps/Numlock, externalheldmodifiers, missing
symbols. PublicEIS source docs primary /usr/include/libei-1.0/libei.h describes
EI_EVENT_KEYBOARD_MODIFIERS forsender, but observedKWin lacksstateevents; don't assume.

Focused GTKcopy/cut/paste/Text and realKonsolepaste regression passed after testrelay
changes; strictalltargetClippy passed. Newprobes pass after strengthenedmodifier
assertions. Probe privateDocumentPortal FUSEcleanup once raced afterserverexit;
newrunner teardown usesfusermount3 only onits exact owned root/runtime/doc when
available. Oneoldtemp root /tmp/deckforge-focused-tl8ur8jy mayremain; findmnt showed
no active matchingmounts afterfailure. No broadcleanup needed. Final probe pipeline completed successfully: current portalprobe assessment confirms
the held-release limitation; publicEIS prototype passes all3layouts. No active
build/probe processes remain.


## Latest OS increment: left/right shortcut modifiers, 14 October 2026

Installed SHA `6dc3ad54a59563adc30daf55bee14a93cccd7ae657fc8d2721f5f42a12360711`;
.previous SHA `c576f86c4fc969b263afa6d2c31820fa2d43066a464378dba24004d5dbf15bdd`.
Final native editor proof matches release/installed SHA. Actual packet acceptance
records test-binary and keyboard source hashes; current keyboard source matches.
Installation preserved2,388 state/plugin/pack files; no user runtime restart/input.
Native screenshot reviewed. Proof target/sided-hotkeys/install.json. Goal remains
active and this turn was progress, with no blockers/subagents/commits/pushes.

Auditing layouts found a concrete unsupported original feature: side-specific
modifiers. REA0x14059d3b0 branches on KeyModifiers0x10000 to select either generic
4-entry or sided8-entry table. Extracted tables0x1415a1f38/0x1415a1f60 and
GetKeyState0x14059da90 prove low summary bits1/2/4/8, left summary<<4, right<<8.
Generic mode ignores captured side bits. Evidence ../evidence/sided-hotkeys/;
REA original target remains OPEN/HOT. Do not close/reimport. Some exploratory
interior addresses resolved;0x14059d150 invalid, no need to retry it. Tool results
are large: print normalized_result only, and never rg a minified evidence JSON line.

system_keyboard now supports right/both modifiers and modifier-only chords,
separates modifier flags0x10fff from keypad/OEM identity, and avoids adding Left
Shift to Backtab when Right Shift already selected. Explicit legacy booleans must
agree with effective modifiers. Generic order preserved; sided flags use existing
ownership/reverse release rather than resetting external keyboard state.
Hotkeys editor exposes Choose modifier sides and Off/Left/Right/Both for each
family. Changing main key preserves sides; switching to generic collapses sides,
reenabling initializes enabled modifiers left; Clear clears all. Applies to shared
Hotkeys form (Hotkey/Switch/dial/custom Text paste). No profile schema migration.

12 keyboard Rust tests pass plus one intentionally gated private-desktop fixture;
15 System worker tests pass;20 Fields tests pass; frontend typecheck/build and
strict all-target Clippy pass. Native WebKit fixture imports Right Alt, selects both
Ctrl sides and Q, preserves unknown chord/root settings, saves/restarts correctly.
It executes no shortcuts. Existing focused GTK clipboard and real Konsole terminal
paste suite passes after parser changes. Test-only backend uses actual imported
chord decoder and Keyboard event writer into an owned file; EIS relay verifies exact
private KWin bus PID before forwarding. Never real /dev/uinput or user windows.

New tests/sided_hotkeys_interop.py runs private KWin for US/German/Norwegian with
kxkbrc config and XKB_DEFAULT_LAYOUT; owned Qt records actual scans, symbols, text
and modifiers. Left/right/both Ctrl+C, Right Shift+A, release cleanup pass. German
RightAlt+Q and NorwegianRightAlt+2 produce @. USRightAlt+Q produces q with Alt.
This is positive AltGr acceptance, not full layout parity. Source/report hashes
are in target/sided-hotkeys/proof.json; native proof/screenshots in native/.

The same tests demonstrate the next concrete OS gap: physical Y key emits Z under
German layout. Current imported_hotkey returns fixed evdev positions and has no
active-layout translation. The original Windows key backend0x14059d210 uses mapped
virtual keys/scan fallback, with layout-aware MapVirtualKey functions. Need plan
and implement logical active-layout behavior without silently breaking deliberate
physical shortcuts, modifier ownership, FIFO/cancellation or saved records. Reuse
existing public keyboard portal (system_typing.rs) where suitable, or prove another
backend; do not assume GDK focus-local state is current global compositor state.
Physically held external modifiers and Caps/Num Lock acceptance also remain open.
Other compositor backends and original OS action-graph MIME clipboard remain gaps.
Keep user priority OS actions; device acceptance user-owned and backgrounds/plugin
platform work deferred. Full goal is not complete. Distribution archives older.


## Latest OS increment: normal Close requests, 14 October 2026

Installed SHA `c576f86c4fc969b263afa6d2c31820fa2d43066a464378dba24004d5dbf15bdd`;
.previous SHA `4c58bd705d2ef8ade985bb148c010c7c91a6ad69f812c9e48736b66738b23303`.
Exact final release matches private-desktop and native-editor proof hashes.
Installation preserved2,388 state/plugin/pack files. Running app was not restarted;
save, full Quit and reopen loads this update. Native screenshot inspected with the
Close behavior and matching controls visible. Reports: target/system-close/.

Full parity goal remains active, OS priority unchanged; physical-device acceptance
belongs to user, backgrounds/screensavers and foreign-platform plugins deferred.
This turn made progress; no blocker. No commits/pushes/subagents/messages to other
chats or user-app close tests. REA target remains OPEN/HOT; do not reimport/close it.

Original KA_Close is a normal WM_CLOSE request unless forceQuit is true. Loader
0x14038dfa0, execute0x14038dc20, inverse flag assembly0x14038dd96, manager0x1407b0020,
backend0x1407bdb00. Normal one-option path uses type0/1000ms; force type3/100ms.
The backend's default fallback vector is not used by KA_Close. No matching process
is successful no-op. Native evidence exported to ../evidence/system-close/.

New system_close.rs routes Close through the application worker. New assignments
and native imports default linuxCloseMethod windows. Existing saved absent-method
records retain automatic: close GUI windows, SIGTERM only after successful empty
window inventory. Linux-tagged imports/export clones explicitly preserve automatic.
Explicit terminate bypasses compositor; forceQuit uses SIGKILL. Desktop failures
never escalate. Matching uses same-user exact exe plus optional literal
linuxMatchArgs prefix, pinned identities, post-inventory executable/argv recheck,
and captured window UUID/PID pairs. Normal/dialog windows are included. GUI veto
or stale window IDs never trigger SIGTERM. Found matching GUI also protects helpers.

Open Application held Close shares this path. Fixed a real test finding: manual
exec+args previously matched every process of the same interpreter. Manual args
now identify processes for activation and held close; desktop launchers retain only
fixed Exec argument prefix, while extra user arguments remain launch-only. UI hint
explains this distinction and tolerates malformed imported source values.

Renamed own text_mode.rs to action_defaults.rs and added Close import/export rules.
Updated library/export references and docs. Existing sessions are not transformed.
Unknown payloads/settings remain intact. Old unmarked Linux archives remain
indistinguishable from native archives, so native import defaults apply.

Passed3 process,2 application,1 close,3 default,4 text-import,15 export (including
original installer corpus),15 System action Rust tests. Strict all-target Clippy
and frontend typecheck/build pass;22 focused UI tests pass. Final binary private
KWin acceptance covers real Qt normal/dialog close events and veto, accepted normal
exit, helper protection, argument isolation, held Close, explicit SIGTERM/SIGKILL,
desktop error without signal fallback, no-match success, legacy automatic headless
behavior. Existing Open Application --kwin suite runs inside the same private
desktop and passes activation/no duplicate launch, hold timing/none, page cancellation
and restart. Existing System next host suite also passes. No user windows, USB or
real clipboard/keyboard input used.

Native editor checks all three Close modes, force preserving dormant method,
argument scope, unknown Future payload and restart, without action execution.
Fixture initially edited before external settings reached React and between partial
save responses; fixed by waiting for rendered settings and completed clean Save
state. Do not weaken settings-preservation assertions. ETXTBSY in concurrent copied
executable unit fixture is narrowly retried only during fixture spawn; production
launch behavior unchanged.

Remaining OS work: alternate keyboard layouts/modifier states, other compositor
backends, launcher wrappers/runtime executable changes and richer desktop matching,
and original action-graph system clipboard MIME interchange. Application-specific
save dialogs and physical-device behavior remain user acceptance. Do not invent
speculative command conveniences instead of auditing remaining original behaviors.
The self-executable canonicalize guard can fail in an old running process after
its installed binary is atomically replaced; full restart resolves that existing
limitation. Distribution archives remain older; no repackaging in this increment.


## Latest OS compatibility increment: legacy Text defaults, 14 October 2026

Installed SHA `4c58bd705d2ef8ade985bb148c010c7c91a6ad69f812c9e48736b66738b23303`;
.previous SHA `0805f9af3212da21c469449bdd30a7439389651b5c374d5ff7bc8600ee31d72e`.
Exact binary matches both final actual-host and native editor reports. Installation
preserved2,388 state/plugin/pack files; no real app restart or live keyboard permission
request. Native screenshot inspected. Proof: target/text-defaults/install.json.
Final native proof includes invalidModePreservedAndIdentified=true, distinguishing
it from the earlier pre-UI-build proof. The final UI suite has46 passing tests and
its separate tsc --noEmit check passed. No release rebuild is needed for subsequent
test-only changes. Existing App global typing Stop was also verified outside the
Text inspector. Next practical OS audit: compare original Close graceful-window
semantics with the current Linux SIGTERM path before changing behavior.

Previous turn was progress. This turn fixes missing Text-mode interpretation at
new import/export boundaries, preserving all existing saved-session data. Full
parity goal remains active, with OS priority and user-owned physical acceptance.

New text_mode.rs applies native nonempty-text => typing, empty/missing => paste on
new profile/shared-action imports. A package OSType Linux marker keeps missing
modes in Paste for older Linux archives. Explicit flags, including invalid values,
are untouched. Walk only recognized container action trees, max8 nesting and the
existing3/512 child limits; never traverse arbitrary plugin metadata. prepare_bundle
normalizes cloned action records/settings after retaining original source files.
This path is not used to restore saved sessions. No migration flag or persistent
schema change is introduced.

Export materializes effective Linux Paste defaults on cloned raw action trees,
including unhydrated nested actions on inactive pages. Explicit imported typing
stays true. The stored library/settings are not changed. Source member bytes and
unknown/out-of-layout data remain intact. Ambiguous old unmarked Linux exports
cannot reliably be distinguished from native files; native defaults apply there,
with mode review documented. Future exports are explicit. Missing pastedText now
means empty string for Enter-only actions; explicit null/non-string remains invalid.

Native fields show non-boolean modes as Unsupported saved mode, aria-invalid with
linked explanation, preserving their values until a user explicitly chooses a mode.
Fixed an old TypingAccess test's partial-Slot type assertion that was preventing
new UI builds. The release now includes the current nullable-settings guard/global
Stop UI. Added global Stop binding test. Existing chooser remount test now waits for
its commit button to become visible/enabled before clicking; no product behavior
was weakened. All final typecheck/build results were inspected, not inferred from
stale logs.

Passed6 Text-mode tests,15 export tests including both optional original installer
artifact tests,28 transfer,15 System actions,46 relevant UI tests, full typecheck/build
and strict all-target Clippy. The original roundtrip test now verifies only explicit
Paste additions for the old in-memory runtime fixture, then compares all other raw
fields/assets exactly; unsupported raw controls are not exempted. Fresh native
import semantics are independently covered by fixtures and actual-host tests.
Initial integration fixture incorrectly used XL ID20GAT9901; corrected to Plus ID
20GBD9901 rather than weakening the app's model guard. An opaque test fixture now
supplies its ActionID so unrelated normal export ID creation doesn't obscure the
mode-preservation assertion.

Actual release host test covers plain/Windows/macOS/Linux profiles, known nested
and hidden actions, opaque payloads, invalid explicit modes, source/asset bytes,
shared actions, old stored settings unchanged across export/restart and explicit
portable output. Native editor test checks imported typing vs existing paste,
unsupported-mode UI without coercion, and no permission request. Neither fixture
executes an action or opens USB/uinput. Reports target/text-defaults/. Screenshot
review and installation details follow in the install report.

Next OS work: audit the remaining concrete OS rows, particularly hotkey behavior
under alternate layouts/modifier states and other portal backends. The legacy mode
rules are now defined; don't invent origin for unmarked old archives. Original
action-graph OS clipboard exchange remains separate from verified text clipboard.
REA target remains OPEN/HOT; native defaults evidence is in ../evidence/text-typing/.
No commits/pushes/subagents or real-app restart this turn.


## Latest OS increment: clipboard-free Unicode typing, 14 October 2026

Previous turn was progress through focused-app acceptance. This turn implements and
installs explicit Text isTypingMode=true through the public RemoteDesktop portal.
Full goal stays active. User keeps physical validation; other priorities unchanged.

Installed SHA `0805f9af3212da21c469449bdd30a7439389651b5c374d5ff7bc8600ee31d72e`;
.previous SHA `2c74586848da13c9baa6b2b04d901bb8cb669db195e866b5c81bdb88c79eb5c2`.
Exact binary matches three acceptance reports under target/text-typing/: proof.json,
native/native.json, os-regression/interop.json. install.json records2,388 unchanged
state/plugin/pack files. No real app restart or live desktop permission request.
No commits/pushes/subagents. TEXT_TYPING.md has the complete contract and boundaries.

Key discovery: absence of an EIS text device did not rule out Unicode typing. The
public portal NotifyKeyboardKeysym route worked in private KWin6.7.5/GTK/Konsole.
Production uses only public CreateSession/SelectDevices(keyboard only)/Start, keysym
notifications and Close. No internal KWin API, permission-store writes, clipboard
access, screen/pointer capture, persistent token or new Cargo dependency in product.
Explicit Enable is once per app session; imported/loading/saving text never requests
permission. Setup sends no text. Restart returns to disabled even with mode=true saved.

system_typing.rs owns a cancellable zbus connection on its own Tokio thread. Bounded
job channel, generation-checked state, explicit requester/session cleanup, Closed
signal revocation, max120sec permission wait and bounded notification/close calls.
Response match rules are restricted to this connection's request namespace. Public
session_handle arrived as a string here; both Str and ObjectPath variants are handled.
Cancelling permission closes the real dialog, and a subsequent setup works. Phase
stopping persists until in-flight key release/session cleanup completes. Shutdown
checks the flag before the select loop to avoid a consumed-notification join hang.

Type plan validates all text before submission, normalizes CRLF/CR to Return, maps
Unicode/Tab/Enter, rejects other control chars/NUL, retains1MiB bound. Custom Hotkey
is ignored in typing mode and preserved in settings. Main System worker waits for
typing completion so queued keyboard actions stay FIFO; global Stop bypasses that
queue. Held owned shortcuts block typing. Overflowing only Type/PrepareTyping jobs
rejects them without permanently cancelling the keyboard; lost releases retain the
existing cancel/release-all behavior. Stop and host cancel invalidate the portal
session early. Initial unpaced test exposed a large downstream text backlog after
shutdown;5ms per-character pacing fixed the GTK/Konsole acceptance. Already-delivered
input cannot be recalled; do not claim instantaneous rollback in arbitrary clients.

UI Text mode selector + TypingAccess controls in root/nested editors, outside readonly
configuration fieldsets. Header Stop remains accessible after selection changes.
Test press is disabled for typing actions/containers to avoid sending Enter back
into the editor button. Existing fields/unknown/custom-shortcut data preserved.
Legacy egui editor now directs typing setup to the main editor. No app state schema
migration. Settings lacking isTypingMode intentionally retain previous Linux Paste
behavior for now, so existing profiles weren't silently switched to permission mode.

Final tests:1 keysyms test,14 System worker tests including typing-queue overflow,
52 focused UI tests, typecheck/build, strict all-target Clippy. Release private-portal
suite proves Unicode Latin/Greek/CJK/emoji/combining, Tab/CRLF/CR/Enter, clipboard
sentinel preservation, no typing before permission/setup, FIFO, control-char preflight,
external backend Session.Close revocation, mid-text Stop, actual permission-dialog
cancel and recovery, real Konsole direct typing, shutdown and restart. The private
PermissionStore process's XDG_DATA_HOME is verified BEFORE test-only authorization.
No real uinput/USB or user desktop input/permissions touched. Native UI two-phase
save/restart proof and screenshot inspected; no OS actions executed there. Existing
command/clipboard/environment OS suite passes on the exact same final binary.

REA remains OPEN/HOT on supplied executable SHA2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3.
Private evidence ../evidence/text-typing/{native-analysis.json,bool-default-analysis.json,
ka-text-vtables.json}. KA_Text loader394bf0/serializer395100; execute3956b0 dispatches
false to clipboard and true to Windows text manager, normalizing CRLF/CR. Manager
59d700 builds Unicode INPUT records and calls SendInput. Helper1b9ed0 via1b8380 returns
param3 when JSON key missing, confirming loader's !text.isEmpty() missing-mode default.
No original app executed. Avoid broad ESD.*Text regex: case-insensitive Text also
matches Context and can return huge RTTI data. Always filter/limit stored tool results
before printing. Session annotations/database have not been closed or reimported.

Next OS compatibility work: reconcile native missing-isTypingMode import defaults
with existing Deckforge sessions without changing existing behavior or raw unrelated
data. Native nonempty legacy text defaults to typing, while old Linux saved records
without the flag currently paste. Consider new-import inference plus explicit old
session preservation, backed by fixtures/portable export provenance, before editing.
Other open boundaries: arbitrary modifiers/layouts/portal backends, original action-
graph OS MIME exchange, remaining full feature matrix. Do not replace these with
speculative command conveniences or declare full parity complete.


## Latest OS acceptance: focused clipboard, 14 October 2026

Previous goal turn was progress. This turn adds real focused-application acceptance,
not a production behavior change. Installed binary stays SHA
`2c74586848da13c9baa6b2b04d901bb8cb669db195e866b5c81bdb88c79eb5c2`, verified live.
No install/restart/state edits this turn. Goal remains active with OS priority;
physical device/uinput acceptance is user-owned. No commits/pushes/subagents.

Added ignored/gated Rust tests in src/system_actions/focused_clipboard_test.rs,
compiled via test-only module in system_actions.rs. Real Worker/plan/keyboard ownership
and input_event writer execute using Keyboard::with_test_writer, never /dev/uinput.
tests/focused_clipboard_interop.py starts private virtual KWin/bus/runtime/config/data,
GTK TextView and then actual Konsole with an owned Python stdin receiver. C++ bridge
checks the KWin service PID equals the compositor it started and forwards only valid
EV_KEY/EV_SYN frames to the private EIS keyboard. GTK commands/state use private
atomic files; terminal input is read as data, never shell commands. Both Rust tests
refuse missing/private-environment gates; normal tests skip them. No user desktop input.

Passed actual focused Copy/Cut/Paste, multiline Unicode Text+Enter, copy-only no keys,
Konsole CtrlShiftV paste and imported custom Text CtrlShiftV+Enter.34 key events observed.
GTK checks focus/active state and exact document/clipboard text. Konsole verifies
real TTY and two exact Unicode input lines. Report target/focused-clipboard/acceptance/
proof.json records test-binary/source hashes, not a false release/uinput claim. Strict
all-target Clippy passes; no runtime changes justify rebuild/install. OS_ACTIONS.md,
SYSTEM_ACTIONS_NEXT.md and FEATURE_PARITY.md updated. Source test binary built with:
`cargo test clipboard_on_private_wayland --no-run --message-format=json`, artifact
path selected from the compiler-artifact entry with profile.test=true. Then run
`python tests/focused_clipboard_interop.py --test-binary <that executable>`.

Evidence changed next action: installed KWin6.7.5/libei1.6.0/GTK3.24.52/Qt6.12.0.
Old KDE fake-input interface absent. Direct private KWin EIS connects; keyboard present,
no usable text device was offered. libei1.6 header has text_utf8/keysym APIs; upstream
KWin master conditionally offers TEXT, but this does not prove runtime support here.
Direct clipboard-free Unicode typing remains unimplemented. Probes under target/
focused-clipboard/probe* are throwaway, not product code. Initial private PID guard
using /proc/environ could not read KWin data; replaced by authoritative private-bus
service PID equality against the exact Popen child. C++ needs -fPIC for local Qt.

Next OS parity work: investigate original Text typing contract and a supported Linux
provider, including whether portal/keymap-based typing is available for the current
compositor; do not infer support from headers or a synthetic server alone. Original
action-graph OS MIME exchange also remains open, distinct from proven OS text clipboard.
Avoid further speculative command conveniences. REA native target remains OPEN/HOT.
User's real application/settings and full feature-parity objective are preserved.


## Latest OS increment: command environment, 14 October 2026

Previous goal turn was progress. This turn installs per-command environment rows,
literal values, empty/unset entries and custom PATH, for background and terminal
commands. Goal remains active; keep OS focus, user-owned physical validation and
deferred backgrounds/screensavers/platform-specific plugins. No real app restart.

Installed SHA `2c74586848da13c9baa6b2b04d901bb8cb669db195e866b5c81bdb88c79eb5c2`;
.previous SHA `877d0f14470fe439fb321c5c6666cc834641cac6f5994050c4481b88fdfe44e3`.
Exact OS-tested binary installed with2,388 unchanged state/plugin/pack files. Proof
is target/command-environment/install.json. No commits/pushes/new chats/subagents.

system_command::Config parses settings.environment rows {name,value,unset}. Missing
list inherits unchanged. Names ASCII identifier, unique, <=128 chars; <=64 rows and
64KiB total; values preserve Unicode/whitespace/newlines/dollars with no expansion.
Unknown row metadata stays in raw settings; inactive values remain when Unset is on.
Background Command applies env/env_remove only to the child. Shared find_program_in_path
keeps original behavior for other System actions. Target PATH override resolves
absolute search entries; unset uses /usr/bin:/bin for resolution then removes PATH;
empty disables lookup. Home working-directory default still belongs to the host.

Terminal GUI gets no overrides. Encoded target payload applies /usr/bin/env changes
before target exec. A discovered env ambiguity for executable paths containing '='
is handled by /usr/bin/nice -n0 -- as a zero-adjustment exec wrapper in that case only.
This avoids another shell rewriting override values. Both literal values and a real
Konsole launch using an '=' custom-PATH program are proven. Target QT_QPA_PLATFORM
was deliberately invalid while Konsole retained its inherited working backend; a
following command proved the host environment unchanged. No global env mutation.

CommandEnvironment.tsx adds expandable rows in existing Fields, supports Add/Remove,
multiline values and Unset with retained disabled value. Unsupported imported formats
remain unchanged until explicit replacement with an empty list. UI notes ordinary
profile/export storage. Native save/restart/export preserves rows. Screenshot reviewed.

Passed13 command tests including capture regressions and4 env checks,13 System action
tests,2 env UI +16 Fields tests, typecheck/build and strict all-target Clippy. Final
exact binary passed expanded tests/os_actions_interop.py in private virtual KWin/bus.
Native tests/native_command_environment_interop.py passed forms/restart/profile export
before the backend-only '=' fix; frontend is unchanged. Initial native fixture used
wrong command name profile_export, corrected to existing export_profile. Proofs in
target/command-environment/. Existing no-USB/no-user-clipboard evidence boundaries hold.
OS_ACTIONS.md and FEATURE_PARITY.md updated. No remaining env work is required for this
subset; interactive stdin and other terminal/compositor acceptance remain open.

Next: return to core OS clipboard/keyboard behavior, especially focused-application
copy/cut/paste and Text paste acceptance using isolated clients. Direct Unicode typing
and original action-graph OS MIME interoperability remain open; inspect existing evidence
before choosing an implementation. Avoid adding further speculative command conveniences
instead of closing original OS behavior gaps. REA native target stays open/hot.


## Latest OS increment: nested command results, 14 October 2026

Previous goal turn was progress. This turn installs nested Run command result/Stop/
Copy controls in Sequence, reused through Multi Action, Switch, Trigger and Key Logic.
The broad goal stays active, with OS priority and user-owned hardware acceptance.

Installed SHA `877d0f14470fe439fb321c5c6666cc834641cac6f5994050c4481b88fdfe44e3`;
.previous SHA `7c962792925039bf38e30768e418e00b1bb0ca2d3e448d1ece2f747320e68c79`.
Proof target/nested-commands/install.json: exact native-tested binary,2,388 unchanged
state/plugin/pack files, no real-app restart. Save/full Quit/reopen loads the update.

Sequence.tsx reuses CommandOutput outside its readonly edit fieldset. Saved entries
match runtime positions with UUID/ActionID checks. Dirty containers require a unique
stable ID in both draft and runtime; otherwise controls hide with save/revert guidance.
This avoids stopping a different same-UUID sibling after a legacy unsaved reorder.
Saved duplicate-ID rows get unique React keys; stable unique IDs keep normal keys.
Controller cancel/copy validates the current nested slot through existing slot_any.
Native command_output already validates device/context/run ID. No new persistence.

19 UI tests passed:6 new SequenceCommands,10 existing Sequence,3 CommandOutput.
Typecheck/build and all-target strict Clippy pass. Exact release native fixture
`tests/native_nested_commands_interop.py` passed execution/Stop/result preview in
Multi Action, Switch, Trigger, and Key Logic -> Multi Action, plus wrong-device guards.
Screenshot reviewed. Test uses harmless owned sleep commands, simulation, no USB or
clipboard writes. Prior capture/clipboard/process tests reused unchanged. Initial
fixture failure was incorrect flat Multi Action Actions data; corrected to the real
Actions[].Actions[] contract. A nullable child TS guard was corrected before release.
OS_ACTIONS.md and FEATURE_PARITY.md reflect the resolved nested UI gap.

Next OS work: configurable command environment, then focused clipboard shortcuts
or original action-graph clipboard interoperability. Existing capture limits and
terminal semantics are below. REA target remains open/hot. No commits or pushes.


## Latest OS increment: captured command output, 14 October 2026

Prior goal turn was progress: OS clipboard and command actions were installed.
This turn extends Run command with captured stdout/stderr, output-to-clipboard,
per-run status, Stop and timeout. Full parity remains active with the user's OS
priority; physical validation is theirs and backgrounds/screensavers/plugins stay
lower priority. No commits/pushes/other-chat messages. No real app restart.

Installed SHA-256: `7c962792925039bf38e30768e418e00b1bb0ca2d3e448d1ece2f747320e68c79`.
Previous `.previous` is `6dc56d049ce089e97b24020175b13ad9be431a1d0e01214ee51fc1997b9697c3`.
Install proof target/command-capture/install.json confirms exact tested binary and
2,388 unchanged session/plugin-data/pack files. Save/full Quit/reopen loads it.

system_command/capture.rs uses nonblocking owned stdout/stderr pipes drained by
the existing System worker. No new capture threads. Cap8 active,16 result records,
1MiB stdout/64KiB stderr,64KiB drain per stream per tick. Discard excess while
continuing to drain so the process cannot deadlock on a full pipe. Running records
cannot be evicted. Distinct contexts/run UUIDs guard cancellation, preview/copy and
late responses. One captured run per action; all command starts reject overlap
with its active capture. Successful discarded-output launches forget old results.

Captured commands get a private process group,1–3600sec timeout/default30, and Stop.
Kill only before reaping the direct child, so the group identifier cannot be reused.
A wait error stops trusting ownership. Completion waits for EOF or500ms after direct
child exit; a descendant retaining a pipe fails without changing clipboard. Manager
Drop stops live captures before waiting for HTTP workers, with detached reaping.
Discard-output and terminal launches retain previous outlive-host behavior. Shell
processes that create a separate session are outside the owned process group.

New action defaults output=show; saved settings without output still mean discard.
Terminal mode keeps saved capture options dormant. Clipboard mode copies only exact,
complete successful UTF-8 stdout without NUL; empty output empties clipboard. Failure,
Stop, timeout, binary/NUL or oversized stdout leaves clipboard alone. Stderr truncation
can coexist with complete copyable stdout. Result status records process completion;
a subsequent clipboard error is reported in its message. Manual Copy uses the same
validated retained stdout. Output is runtime-only, not in session/profile or logs.

Snapshots add slot.command_run metadata only. Native command_output fetches <=16KiB
per stream with device/context/run validation. command_cancel/command_copy_output
reuse Controller commands and worker FIFO, no history. CommandOutput.tsx shows state,
Stop/Show/Copy; modal uses escaped text and a portal outside the editor form. Context/
run changes invalidate pending fetches; viewing/closing retains unsaved drafts.
Top-level slots have result UI. Nested commands execute/capture but their result
controls in Sequence remain to implement. OS_ACTIONS.md and feature ledger updated.

Verification:9 command tests pass for literal argv/terminal transport, streams,
overflow, invalid output, cancellation, timeout, inherited pipes, retention and
shutdown;3 output-viewer tests pass;16 Fields tests pass including dormant capture
settings across terminal toggles. Typecheck/build and strict all-target Clippy pass.
Native tests/native_command_capture_interop.py passed Stop, output controls, escaped
preview, separate stderr, stale IDs and unsaved draft. Screenshot reviewed. This
native UI proof predates only the shutdown ordering change; frontend unchanged.
Final exact installed binary passed tests/os_actions_interop.py in private virtual
KWin/session bus/runtime: independent GTK clipboard, command/terminal execution,
success/empty output copying, failed/invalid/timeout/Stop clipboard preservation,
output not in session/activity log, actual captured child stopped on host shutdown,
and restart clears results while retaining settings. Proof target/command-capture/.
No USB, real clipboard or focused-user keyboard shortcuts touched.

Next: finish command-result controls inside sequences/Action Trigger/Key Logic,
then continue OS workflows such as configurable command environment and focused
clipboard shortcuts. Original action-graph OS MIME exchange remains separate/open.
Do not re-run unchanged broad suites or restart the real app unnecessarily. Preserve
source/install state. REA native target remains open/hot and was not used this turn.


## Latest increment: Linux OS actions and action-file chooser, 14 October 2026

User steering: prioritize OS integration, especially clipboard and terminal commands.
The user handles device validation. Defer background/screensaver polish and deal
with Windows/macOS-only plugin issues when needed. Full parity remains active;
no measured percentage exists. OS_ACTIONS.md records behavior and remaining scope.

Installed/tested SHA-256:
`6dc56d049ce089e97b24020175b13ad9be431a1d0e01214ee51fc1997b9697c3`.
Previous installed build is `5e1478654ebafae3d3eba0e69b0548fa73289d5f4422984a44fec9f73c4434ba`.
The installer preserved 2,388 tracked state/plugin-data/icon-pack files. No restart
of the user's running app; Save, full Quit and reopen loads the installed build.
Installation proof: target/os-actions/install.json. No commits or pushes this turn.

New `system_clipboard.rs`: own UUID com.deckforge.system.clipboard, operations
copy_text/copy/cut/paste and terminal Ctrl+Shift copy/paste. Copy text preserves
Unicode/whitespace and supports empty content without uinput. Shared arboard owner
with existing Text paste; failures discard the handle for retry. Other operations
reuse Chord/keyboard ownership. OS clipboard differs from internal action graphs.

New `system_command.rs`: own UUID com.deckforge.system.command. Program mode uses
literal split argv; explicit shell mode uses /bin/sh. Absolute working folder or
home default. Async execution reuses the existing worker/child limit and failure
reports. Terminal executable/prefix configurable, default Konsole --separate
--hold -e. Actual Konsole expands $ENV in supplied argv, found by the first fixture
failure and corroborated in upstream source. Fixed with a bounded base64 transport
of a shell-quoted cd/exec payload through /bin/sh and /usr/bin/base64. Hostile quotes,
newlines, dollars and substitution strings now pass unchanged. No temp scripts.
Commands outlive host shutdown. Output viewer, output-to-clipboard, cancellation/
timeouts and environment editor remain open; other terminal launchers unverified.

Forms use existing SettingsFields controls; inactive mode and unknown fields survive.
41 focused frontend tests passed, typecheck/build passed, 3 command + 1 clipboard
planning tests passed, 13 existing System worker tests passed, strict all-target
Clippy passed. Native OS form edit/restart proof is target/os-actions/native/; this
UI test preceded the terminal-only backend fix, so its binary hash differs. The
UI inputs are unchanged and the final exact binary passed actual OS acceptance.
`tests/os_actions_interop.py` runs a private virtual KWin/session bus/runtime, no
DISPLAY fallback. Independent GTK reads exact Unicode and empty clipboard text;
actual program/shell/Konsole execution checks release ownership, working folder,
TTY, literal args, pipeline/redirection, responsiveness, exit7 and settings restart.
Proof target/os-actions/interop.json matches installed binary. No real clipboard,
USB or focused-user shortcuts touched. Shortcut/application acceptance is user's.

Also completed the pending multi-root action-file chooser. shared_action.rs inspects
bounded roots and hashes original contents; desktop_action_file.rs owns one pending
choice. Explicit root selection keeps its referenced folder tree and existing
transaction/target guards. Shared bridge routes both cold/warm files through the
same manager; main no longer handles cold native files through legacy dispatch.
Choice/query retained across native view recreation; cancellation is inert; stale
source and destination fail. Single-root remains direct. Final exact release passed
five native phases including folder/media, Undo/Redo, clipboard, source mutation,
and restart after deleting source files. Proof target/action-choice/native/. Existing
warm GIO/cold profile file-open regression passed before the independent OS additions.
Previous transfer28/history17 and chooser tests were reused, with the current 41 UI
checks covering the final forms and file chooser. One pending-test nullability error
was fixed so frontend typecheck passes. Both native screenshots visually inspected.

REA evidence saved as ../evidence/action-choice/native-analysis.json; target remains
OPEN/HOT. Import lambda0x140329b50 picks the first stash from vector3bfc50/3c2ef0;
lambda329f30 routes one stash to canvas3a79d0 or clipboard. Original first-root order
is unproven. Linux chooser is a deliberate extension, not vendor batch placement.
ACTION_SHARING.md and FEATURE_PARITY.md corrected accordingly. General action-graph
OS MIME sharing remains distinct and open. No Ghidra reimport/close needed.

Next OS work: command output capture/copy and process feedback/cancellation, or
focused desktop clipboard shortcuts. Preserve the user's existing runtime/settings;
continue isolated OS acceptance, leaving physical-device validation to the user.


## Latest full-parity increment: live icon library refresh, 14 October 2026

Installed/tested SHA-256:
`5e1478654ebafae3d3eba0e69b0548fa73289d5f4422984a44fec9f73c4434ba`.
Previous turn delivered pack management; this turn removed manual Reload for
ordinary local changes. Full parity remains active. ICON_PACKS.md is current.

New icon_pack_watch.rs owns a nonblocking inotify fd, drained by the existing
native desktop bridge worker; no extra thread or icon-byte polling. Watches cover
state parent for late icon-packs creation, library root, active pack subdirectories,
visibility directories and archive transaction metadata. Ignore reads, lock-file
traffic and unrelated state/profile/appearance-cache writes. Watch directories
before enumerating children; compare dev/inode to retarget atomic replacements.
Observe MODIFY and CLOSE_WRITE, so an open writer can invalidate too. Bursts have
100ms quiet/1s max coalescing,4096 directory cap,16×32KiB drain cap per tick. Overflow
or exhausted read budget recreates the inotify instance, not merely its map:
otherwise a lost IN_IGNORED could retain a dead kernel watch. Observer errors retry
with fresh watches after5s. Ordinary deleted/replaced-directory races are skipped;
observed symlink targets aren't watched. Native snapshots add icon_library epoch/
error. Closing bridge releases the fd/watches. Legacy HTTP/editor doesn't own this
observer and remains outside this native acceptance.

New useIconCatalog.ts serializes automatic reads, defers while management is busy,
coalesces revision changes, drops older replies and retries transient read failures
without discarding the last catalog. Manual reads can supersede older reads.
Cleanup unsubscribes/cancels timers; observer/read failures surface with Reload
fallback. IconLibrary preserves searches and valid selected IDs, repairs a removed
selection to All packs, and retains pending review payloads. A changed target
marks review stale and disables Apply instead of updating approval silently.

Passed4 watcher tests: actual late creation, nested writes/open writer, visibility,
ignored reads/profile writes, atomic replacement retargeting, removal/recovery
records, symlink exclusion, notification bounds and overflow that models a lost
kernel watch-removal event.29 frontend tests (library5,hook2,multi-device22) cover
coalescing, busy retry, old replies and stale review/search. Strict all-target
Clippy, frontend/release builds, Python syntax and whitespace checks pass.

New native_icon_pack_watch_interop.py keeps real WebKit's global library open while
Python invokes the actual CLI in a separate process, exchanges directories with
renameat2, edits metadata/SVG, removes/installs packs and atomically changes
visibility. It verifies7 observed revisions with no Reload, preserved search,
held stale remove review/disabled Apply, repaired selection, refreshed colors and
unchanged profile graph. Test-owned export files are orchestration markers, not
user work. No USB. Screenshot checked. Exact proof:
 target/pack-watch/native/native.json. Existing native management/pending-draft/
restore/restart regression passes on the exact binary in
 target/pack-watch/management-regression/. No original Stream Deck runtime used.

Linux behavior checked against https://man7.org/linux/man-pages/man7/inotify.7.html.
Network-remote writes, mmap writes and mount-over-watch changes can lack events;
Reload is fallback, not a claim of universal notification support. Directory
limits, resource errors and adversarial filesystem races remain explicit.
No new REA investigation was needed; the native Ghidra session remains OPEN/HOT.

Installed exact tested release;2 real state/plugin JSON files and2386 original
pack files unchanged. Real catalog6 packs/2361 icons, no errors. .previous is9f27ba65.
Install proof: target/pack-watch/install.json. Real app is now PID1446881, started
18:49:24; /proc executable hash9f27ba65 confirms the previous management build is
running. This restart was not performed by the agent. Full Quit/reopen activates
the latest watcher build; the agent left the running instance untouched. No distribution archive, commits,
pushes, agents, other-chat messages or memory edits.

Next: Marketplace pack handoff, background/screensaver libraries, pack ordering/
full Key Creator composition, update-on-open review, OS clipboard and multi-root
sharing, remaining plugin/device/Smart/history acceptance. Full IN-04 and overall
parity remain incomplete; do not shrink the active goal to the delivered subset.

## Latest full-parity increment: icon-pack management, 14 October 2026

Installed/tested SHA-256:
`9f27ba6582e8452679efd1cbfacd39586ef504e50dc7ba55e2347c2c1532cca3`.
Previous turn delivered desktop package opening; this turn added global management,
visibility, replacement and recovery. Full parity remains active.

Sidebar Manage icon packs opens the library without needing an assigned action;
its image cards are browse-only. Appearance opens the same component with a
captured selection callback. Hide/Show persists per ID; hidden packs remain
searchable and Show hidden exposes management. Pinned packs cannot be removed;
pinned without canhide cannot hide. Update stages a file for version/count review,
then Apply restages and checks both source content digest and installed filesystem
revision. Remove has a named review and archives the pack. Archived versions show
operation/time, restore review, and newest-first order using nanosecond record
ordering. Replacing/restoring over a current pack retains its predecessor too.

Selected assets now copy immediately into existing appearance-assets/icons/hash/
basename storage. Save remains the graph commit. Thus a pending draft stays usable
after pack remove/update, and saved SVG/export bytes remain independent of packs.
Cancelled selection files are deduplicated but not garbage-collected. No new asset
store. Native thumbnail/asset requests carry pack revision; component keys include
revision so replacement refreshes imagery. Icon library search Enter and all
buttons are prevented from submitting an outer action form.

New icon_pack/management.rs owns visibility/review/apply/remove/recovery. Managed
operations use an owner-private, nonblocking flock on .deckforge.lock. No host
mutex during pack IO. Existing atomic plugin_update::exchange helper is reused
(public to crate; error wording generalized), with no plugin runtime changes.
Private staging/recovery is explicitly700; records/files600. Existing TempDir
permission defaults initially failed the archive privacy gate, caught by tests.
transaction.json is flushed before move/exchange and records old identity/content
plus staged identity. Inode checks recognize committed predecessors vs uncommitted
candidates; restore rechecks full digest. Filesystem revision uses root identity
and ordered file metadata, not a claimed adversarial atomic content snapshot.
No fallible operation between exchange and TempDir retention. No power-loss claim.
Visibility file: icon-packs/preferences/visibility/hidden.json. Corrupt visibility
or recovery metadata yields catalog errors while retaining active pack browsing;
mutations still refuse unreadable preferences. Backups are retained under hidden
.deckforge-icons-* directories, never auto-deleted.

Original Key Creator JS/CSS confirms no-uninstall display:none for pinned packs,
locked visibility for pinned&&!canhide, confirmation for uninstall and hidden
packs included during search. Static snippets/hashes are in
 ../evidence/icon-pack-management/observations.json. One-module REA JavaScript
analysis parsed syntax but exhausted its semantic worker heap; coverage is partial,
not a complete reconstruction. Its extracted source is a subset. No vendor code
executed. Mixed evidence bundle162 records at
 ../evidence/icon-pack-management/rea-bundle.json. Native Ghidra session remains
OPEN/HOT; no cold re-import was performed. A memory search for TempDir/icon packs
found no relevant entries and none were used.

Passed8 generated icon-pack/management tests plus explicit six-original-pack
byte-preservation test;8 plugin update transaction regressions;37 frontend tests
(library4,appearance8,app picker3,multi-device22); strict all-target Clippy,
frontend/release builds, syntax/whitespace checks. Management tests cover stale
source/target, pinned/hide rules, pending bytes, remove/restore, locks and
interrupted exchange layouts with fresh reads (not actual killed-process/power-loss
acceptance). New tests/native_icon_pack_management_interop.py passes3 phases:
pending icon selected then pack removed before Save; reviewed replacement API
with blue thumbnail, old-version restore UI/red thumbnail and preserved visibility;
restart global management from an empty selected key, archives and independent
visibility, exact original SVG export. Removal/restore controls are real native
UI; update file-picker dispatch is covered by frontend tests and actual staging/
apply API is native-tested. Synthetic fixture races were fixed by awaiting enabled
controls after hide; no product conflict bypass. Screenshots visually checked.
Proof: target/pack-management/native/native.json. Original pack selection/Save/
Undo/restart/export test also passes in target/pack-management/library-regression/.

Installed exact tested release;2 real state/plugin JSON files and2386 pack files
unchanged. Real catalog6 packs/2361 icons; original pinned/canhide flags verified.
No real visibility/removal/replacement changes were applied. Only operational
library lock may be created by readback. .previous isf9189d94. Install proof:
 target/pack-management/install.json. Running real app remains untouched; full
Quit/reopen loads cumulative changes. No distribution archive, commits, pushes,
agents, other-chat messages or memory edits.

Next gaps: Marketplace pack handoff, background/screensaver libraries, pack
ordering/full Key Creator composition, update-on-open review and external-change
catalog auto-refresh (Reload is available). Continue core OS clipboard/multi-root
sharing and remaining plugin/device/Smart/history acceptance. Full IN-04 and
full parity are not complete; physical and original-app acceptance remain open.

## Latest full-parity increment: plugin/icon-pack desktop opening, 14 October 2026

Installed/tested SHA-256:
`f9189d94677b05d936ed3b746eccc82e18a676f3c9ea0961fdb0f8a4f14705e5`.
Previous turn installed the icon library; this turn extended desktop file opening
and drops to packages. Full parity remains active.

file_open::Kind now includes Plugin and IconPack with case-insensitive extensions.
Local regular-file/URI restrictions apply to all4 types; decoded/canonical paths
also reject control characters, including encoded newlines. Controller dispatch
moved from transfer/sharing.rs to host/file_open.rs. Action/profile work remains
scoped to the captured device; plugin files call existing Controller::command
install_plugin, and packs call Controller::icon_packs install outside host mutex.
Package targets are global and never replace a canvas control. Existing plugin
runtime/platform validation and icon pack no-overwrite behavior remain. Success
text directs unsupported runtimes to Plugins, and packs to Appearance library.
Duplicates fail; update-on-open review is still missing. The action clipboard
helper visibility widened only to crate::host for the dispatcher move.

Native drop payload and frontend union accept plugin/icon_pack; hover highlights
only action targets. Packages dropped on dirty/readonly controls route through
open_file and preserve drafts. A rejected plugin install now preserves history
when plugin count did not change; valid/partially-published installs still clear
incompatible history. Other existing barrier rules unchanged.

MIME XML/desktop installer advertise application/x-streamdeck-plugin and
application/x-streamdeck-iconpack alongside action/profile types. Existing default
handlers preserved; only missing handlers become Deckforge. URI scheme handler
remains deckforge-2.desktop. Actual installed desktop entry advertises all4 types.

Passed3 path/host-dispatch tests (including bad plugin preserving history and pack
not changing assignments/session),1 native-drop normalizer test,14 instance tests,
21 frontend multi-device/drop tests,4 isolated GIO MIME detections including mixed
case and default-preservation mock, strict all-target Clippy, frontend/release
builds, Python syntax and whitespace checks. Initial compile/test-only issues were
private helper visibility after module move and misnamed frontend fixture helper;
fixed before acceptance. No new reference reverse engineering needed.

New tests/native_package_open_interop.py passes6 cases: original Counter plugin
and original Color Logo pack each opened warm through gio launch, cold through
--open, and through synthetic native drops. Warm cases prove same owner PID and
hidden-window reopening with retained unsaved draft. Drops over a dirty control
retain the action. Duplicate/bad packages are rejected without graph/history
changes; duplicate opening preserves owned Counter PID. Original plugin bytes
match installed copy. Source filenames contain spaces/%/#. Native screenshot
visually checked. No USB; only fixture app/plugin processes run.
Proof: target/package-open/native/native.json, logs/captures. The original native
action/profile file-open+draft regression also passes in
 target/package-open/file-open-regression/. MIME proof: target/package-open/mime.json.

Exact tested binary installed;2 real session/plugin JSON files and2386 existing
icon-pack files unchanged. .previous isb2a04de0. New plugin/iconpack defaults now
deckforge.desktop; previous action/profile defaults and URI handler retained.
Install/default before-after proof: target/package-open/install.json. Running
real app was not restarted; full Quit/reopen loads cumulative changes. No real
plugin installation this turn, only private tests. No distribution archive,
commits, pushes, agents, other-chat messages or memory edits; REA remains OPEN/HOT.

Next gaps: update-on-open review, pack update/removal/hide controls, Marketplace
pack handoff, background/screensaver libraries, OS clipboard and multi-root action
sharing, remaining plugin/device/Smart/history acceptance. Physical OS drag remains
unverified; synthetic native notifications are not that claim. Keep full goal active.

## Latest full-parity increment: native icon-pack library, 14 October 2026

Installed/tested SHA-256:
`b2a04de060dc6a3d53b52899c0b34210a0ff0f8cf546f758a7c68af7c9e8dbfb`.
Previous turn delivered Undo/Redo; this turn delivered the icon-pack workflow.
Full parity remains active. docs/ICON_PACKS.md and IN-04 describe current scope.

Appearance -> Icon library now imports .streamDeckIconPack ZIPs or .sdIconPack
folders, lists packs, searches name/tag/path, pages60 matches and lazy-loads PNG
thumbnails. Checkerboard supports black/white artwork. Selection captures original
state/base/onEdit callback and produces an appearance draft; Save owns the exact
original image bytes and existing Undo/Redo applies. Buttons explicitly type=button
so import/cancel never submit an outer pending action form. Native picker itself
not automated; frontend tests cover import dispatch/cancellation/refresh.

New src/icon_pack.rs implements load/install/catalog/asset/thumbnail and CLI
 icon-pack inspect|install|list. Native Controller::icon_packs captures state and
reference under mutex then performs pack IO outside it. Bridge icon_packs thumbnail
shares four-in-flight queue with action/application icons. New IconLibrary.tsx
reuses Modal/Field, keeping native profile/device ownership in Appearance callback.
No executable code is loaded from packs. read_archive_entries was extracted from
package.rs; plugin-specific UUID/protected-file validation remains in its existing
wrapper. Common ZIP duplicate/path/type/size/file-directory checks stay intact.
Regular-file checks now reject FIFO sources without blocking. Safe folder reads
reuse anchored no-follow package file opens. Private staging and no-replace rename;
root700/files600 with executable bits cleared. Unknown source bytes retained.
Pack ID is the folder stem; a manifest UUID extra field is opaque, not a new rule.

Actual installer Key Creator member is
 ../evidence/elgato-7.6/raw/flsy_lZh6qrewVZPcqeNQmr2lhuQjI,
SHA b576095daf14fa35cf13f266938ea066c73ababa5093ba44980ffa3403e09251.
It contains6 packs /2361 image files: default black845, default white845,
legacy219 (214 metadata entries +5 unlisted images), logo black159,color134,
white159. Unlisted files use filename labels. Actual black/white logo icons.json
have UTF-8 BOMs; first real-corpus test exposed this. Metadata/plain manifest
readers accept BOMs without changing original files. Original manifests require
Author/Name/Icon/Version; description/other fields remain preserved.

REA source contracts: folder ID1401abfd0; duplicate-refusing install1401ad120;
manifest parser140346780/140345ee0; optional icons.json140396e60; array metadata
1403469e0, fields140346220. Named catch thunksd03780/d038f0 were only exception
handlers, not primary parsers. Evidence exported160 records at
 ../evidence/icon-packs/rea-bundle.json (REA still OPEN/HOT). observations.json has
source/manifest/metadata hashes, exact counts/BOMs and unlisted names. No vendor
execution. Standalone Marketplace envelope variants not proven by folder corpus.

Passed:3 generated icon-pack tests; explicit original-six-pack install and every
file byte comparison;8 common package tests plus original Layouts archive test;
13 frontend tests (library2,appearance8,app picker3); TypeScript/build/release,
strict all-target Clippy, Python syntax and whitespace checks. Native
 tests/native_icon_pack_interop.py has3 phases: import original Color pack through
native bridge and tag-search/render SVG in library; choose/Save/Undo/Redo/export;
restart after deleting both source and managed pack, then export exact original
SVG from owned artwork. All phases pass, screenshots visually checked. No USB.
Proof: target/icon-packs/native/native.json and log/captures.

Installed exact tested binary;2 real state/plugin JSON files unchanged;
.previous is5c8b69bf. Then imported all6 original packs into the real user's
 ~/.local/share/deckforge/icon-packs using the new inert CLI, comparing every file
byte against original extraction. Final real catalog6 packs/2361 icons,0 errors;
roots700 and no executable files verified. No existing packs were overwritten.
Original license files retained; assets remain local, not embedded in binary or
published. Install/pack proof: target/icon-packs/install.json. Running real app
still PID1182755, untouched; full Quit/reopen loads cumulative updates.
No distribution repackaging, commits, pushes, agents, other-chat messages or memory
edits. Existing broad worktree preserved.

Next pack gaps: update/removal/hide management, Marketplace pack handoff,
.streamDeckIconPack/plugin associations, background/screensaver catalogs and
full Key Creator composition. Physical/protected standalone pack acceptance is
still open. Also continue core matrix: OS clipboard, multi-root action sharing,
remaining plugin/device/Smart/history acceptance. Do not mark overall complete.

## Latest full-parity increment: native Undo/Redo, 14 October 2026

Installed/tested SHA-256:
`5c8b69bf5967ccb310a08985d0028f85ae8004e542331d0f09a7924a2213cb7b`.
Previous turn completed the pure patch engine; this turn integrated host history,
UI and native acceptance. Full parity remains active. docs/UNDO_REDO.md is current.

History module is now compiled through host.rs. The standalone
 tests/history_patch.rs inclusion was removed; its6 engine tests now run under
host::history, alongside11 host tests (17 total). History records action edits,
nested operations, transfers, page/profile operations, Smart configuration,
explicit desktop profile import, appearance/crop and PI settings/resources.
One action Save has a unique history_group across all requests. Clipboard/input/
plugin runtime/global/mixer/device effects are not replayed. Session history has
50 entries and32MiB serialized bound, labels, revision tokens and redo invalidation;
no-op edits preserve redo. Identity/model and device/plugin config changes clear
incompatible history. Full restart clears history, not the resulting saved graph.

Graph contains profiles, per-scope Smart rules and plugin bundle references.
Navigation/cursors are conditionally restored and invalid references repaired;
current canvas selection remains. Patches preserve unrelated newer fields and
keyed members. Structural removal accepts the same context/id/ActionID even after
normal plugin updates and rebases redo to retain current payload. Unkeyed/ambiguous
arrays conflict conservatively. Container children/known raw Actions branches
normalize by branch address privately, then restore before deserialization.
Synthetic history_raw_branches exists only on serialized Slot history objects;
opaque native Actions objects are preserved, never guessed or persisted altered.

Restore validates all device projections and persists graph/navigation/rules
before lifecycle events. Initial failure rolls back graph/rules/nav/slots and
leaves history/held inputs untouched. At durable commit history advances; later
refresh errors explicitly report saved state. Only affected scopes refresh.
Existing contexts receive settings/resources notifications; title/style events
use each edited state, including non-visible states. Smart rule changes invalidate
only affected scope observations. A real host test exposed lazy resources
None->empty-map on refresh breaking redo; opposite patches now rebase onto actual
postrefresh graph under host mutex. Pending/history sources extend
retained_asset_sources; live_asset_sources excludes history to avoid recursively
retaining evicted entries forever.

App buttons sit beside page tabs; Ctrl+Z/Ctrl+Shift+Z/Ctrl+Y operate outside text
fields. Dirty live action/advanced-text drafts, dialogs and busy work guard undo.
Snapshot history.contexts lists live action IDs, so orphan drafts do not block
restoring a deleted action; resurrected dirty drafts remain protected. No new
selection overwrite. Generic Save requests now carry history_group (one expected
frontend fixture updated accordingly).

Passed17 history tests;25 transfer,16 appearance,1 crop,3 backgrounds,3 inspector
and14 Smart regressions;22 frontend tests; frontend build and release build;
strict all-target Clippy; Python syntax and whitespace checks. Native fixture
 tests/native_history_interop.py uses unchanged Counter sample8cc82bad, bundle
ed811db4. It proves assignment undo/redo with current plugin payload, settings undo
preserving count4, grouped title+pastedText Save, controls/shortcut handlers,
dirty/text guards, restart-persisted graph and empty restarted session history.
Native key events are app-owned synthetic handler checks, not physical keyboard
acceptance. No USB/process launch beyond owned sample and app. Screenshot checked.
Final exact proof: target/editor-history/native/native.json. Earlier failed native
attempts were fixture startup-key assumptions, sending redo while still busy,
and using text instead of the original pastedText settings key; all corrected.

Exact tested release installed;2 real state/plugin JSON files unchanged; .previous
is4b2e774f. Running real app untouched (PID1182755 last rechecked before this pass);
full Quit/reopen loads cumulative updates. Install proof:
 target/editor-history/install.json. No new distribution archive, commits, pushes,
agents, other-chat messages or memory edits. REA stays OPEN/HOT; undo evidence
remains the compact scan below, with no claimed native Windows undo contract.

Next: complete broader parity matrix: OS clipboard interoperability, multi-root
action sharing, icon/profile packs and remaining plugin/device gaps. History still
needs physical/long-running acceptance, legacy ambiguous-record behavior review
and fault injection after commit. Do not infer full UI-04 or overall completion
from these tests. Non-KDE focus, sidebar/device artwork and vendor archive
acceptance also remain open.

## In progress: editor undo/redo foundation, 14 October 2026

Previous turn made concrete progress (installed application icons). This turn
started the next missing core workflow, undo/redo. Full goal remains active.
Do not pivot back to optional polish before completing this integration.

New src/host/history/patch.rs implements reversible field changes. Object maps
recurse only changed fields; arrays use unique context/id/ActionID identities and
separate ordering, otherwise remain atomic. Conflicts cannot mutate input or
overwrite unrelated newer plugin data. No whole-session restore. Six independent
tests pass, including60 generated undo/redo edits, nested insertion/deletion,
ActionID reorder with newer plugin state, ambiguous IDs and null vs missing.
Strict cargo clippy --test history_patch -- -D warnings passes. Logs under
target/editor-history/. tests/history_patch.rs includes the engine directly;
there is NO host module declaration or UI connection yet. A temporary empty
history.rs stub was removed, so production code is unchanged this turn.

docs/UNDO_REDO.md records the remaining integration gates and key design hazards.
Read it next. Especially: structural deletion of a newly created action must
handle normal plugin updates sensibly and retain current payload for redo;
generic exact comparison alone would make assignment undo unusable. Group one
editor Save across its settings/appearance calls. Protect pending/history-owned
media through retained_asset_sources; persist before lifecycle updates; keep
history stacks consistent on postcommit refresh failure; notify settings and
resources for surviving contexts. Cross-device edits must undo atomically.

Useful seams: host::apply_control/apply_control_local; Controller::command special
appearance path; Controller::appearance_command in appearance.rs;
Controller::image_crop in image_cropper.rs; import_profile called directly from
desktop file opening/startup; transfer::publish_transfer already stages graph
and persists before refreshing multiple devices. commit_profile_change skips
refresh for unchanged active profile/page and therefore cannot directly publish
arbitrary undo page-content edits without adaptation. Navigation is swapped
with_device/with_scope; device_records() captures it per stable scope. Current
Slot serialization includes settings/raw/states/background/feedback/appearances
but skips render caches/event coordinates. Rehydration restores child metadata.
Existing settings saves emit didReceiveSettings and notify inspectors; refresh
alone only emits lifecycle changes for changed visible contexts.

REA string scan (undo|redo) did not establish a general undo contract. One native
Auto Exit reset tooltip says it cannot be undone; other short matches unrelated.
Large embedded HTML/base64 false matches were omitted from the compact summary.
Evidence: ../evidence/editor-history/string-scan.json, ev_5ef2df06b57ba30c30eaa81ed0c8911e51690dda4710bd0c94e45190c50511e8.
REA remains OPEN/HOT, subject2963df27. No need to repeat that scan.

No new build/install: undo is not user-facing yet. Installed hash stays
4b2e774f369696b0dfde52028d94aed041b59727ab7a94551ea3a49ecf9afc9a.
No real app restart, USB, settings changes, commits, pushes, agents, other-chat
messages or memory edits. Continue implementation; do not mark undo or full
parity complete based on the pure engine tests.

## Latest full-parity increment: installed application icons, 14 October 2026

Installed/tested SHA-256:
`4b2e774f369696b0dfde52028d94aed041b59727ab7a94551ea3a49ecf9afc9a`.
Previous turn made concrete progress; full parity stays active.

Open Application and Smart Profiles now share ApplicationPicker.tsx: searchable
radio rows with icons, desktop IDs,40 results then Show more, bounded scrolling,
keyboard selection, empty state and letter fallback. Search covers name/ID/path/
executable. Icon requests start on intersection; late replies cannot replace a
new application's artwork. Open Application also shows its chosen launcher icon.
Smart selected rows resolve against stored binding identity/name; recent process
identities remain textual because they may not identify one installed launcher.

system_app::icon_path resolves only a visible GIO installed application and uses
GTK's theme lookup (or declared FileIcon). Called in the Wry IPC handler on the
native UI thread; only the PathBuf/result crosses to Job::ApplicationIcon on the
existing bounded worker. system_app::icon_png reuses action_icons' regular-file,
2MiB input/decode bounds and external-SVG restrictions. Theme symlinks resolve
before the existing containment checks. No new dependency/cache/launcher process.
bridge.ts shares the existing four-in-flight icon queue across action_icon and
application_icon; method participates in deduplication key. Only PNG data URIs
render. Missing/unreadable/unsupported icons retain labelled fallbacks.

Passed:24 focused frontend tests (Smart1,multi-device18,bridge2,picker3), frontend
build and final TypeScript check, cargo check, strict all-target Clippy, release
build, Python syntax and whitespace checks. Native Open Application test now has
3 phases: SVG file icon/search/selection/arguments/held setting Save; restart then
hicolor themed app selection in Smart Profiles; second restart proves selected
Smart radio/icon and disabled routing retained. Test-owned .desktop/theme files;
no apps launched, no USB, no real settings changed. Both screenshots visually
checked. Initial native wait was on an offscreen icon (correctly lazy); fixture
now scrolls its row into view. jsdom needed CSS.escape for user-event radio keys.
Final proof: target/application-icons/native/native.json and captures.

Installed exact native-tested binary;2 real state/plugin JSON files unchanged;
.previous isbc06417e. Running real app untouched; full Quit/reopen loads updates.
Install proof: target/application-icons/install.json. No distribution archive,
commits, pushes, agents, other-chat messages or memory edits. REA stays OPEN/HOT.

Remaining: sidebar app icons, device/running-state artwork, legacy XPM decoding,
non-KDE focus, original duplicate ordering, official-app archive acceptance and
physical acceptance. Broader parity matrix still includes undo, multi-root
sharing, packs and remaining plugins/devices. Keep the full goal active.

## Latest full-parity increment: Linux Smart binding archives, 14 October 2026

Installed/tested SHA-256:
`bc06417e1203765bdf32893bc666ab58c03e485203e427d9862c5240f7b93e3f`.
Previous goal turn made concrete progress; full parity remains active.

Profile export now writes an optional Deckforge/SmartProfile.json within the
profile root (plain and wrapped paths supported). Version1 contains binding
kind/value/name only. This is explicitly a Linux extension, not recovered vendor
metadata. Native AppIdentifier and unrelated source bytes remain intact. Export
removes retained source metadata when the user has cleared the binding. Exact
Linux identities travel; executable paths are not translated between machines.

Explicit host import reads/validates the bounded16KiB metadata before staging a
source, then commits profile/navigation/routing before lifecycle events. Malformed,
oversized, unknown-field and unsupported-version metadata fail visibly before
publication. New profile IDs receive the binding. Destination enable/default/
existing bindings stay intact; fresh state remains disabled. Inactive layouts
retain the rule without changing active controls. Failed persistence restores
graph/rules and removes only the uncommitted source. Postcommit refresh failures
retain their source. Plugin-bundled auto-imports do not install associations.

src/host/smart_profiles/archive.rs owns the format. Binding validation is shared
with settings. export_profile adds/removes metadata after normal file assembly;
write_archive still validates the completed archive before replacing output.
host::import_profile now uses commit_profile_change. That helper skips lifecycle
refresh when active profile/page are unchanged (inactive-model import); its
callers add profiles or change navigation, not existing page contents.

Passed:14 Smart tests including4 archive tests;13 export tests plus2 ignored
installer-artifact tests explicitly run;5 library tests; strict all-target Clippy;
release build; Python syntax and whitespace checks. Frontend unchanged, so prior
build/tests reused. Native copy test expanded to4 phases: duplicate/device copy,
restart, delete original session/image then cold desktop-open into fresh state,
restart/re-export. Binding/destination preferences and exact cropped image pass.
Native file-open/draft regression also passes. Reports and visually inspected
captures: target/smart-archive/native/ and target/smart-archive/file-open/.
An initial fixture selected the wrong simulated device because it passed an
explicit model while restoring multi-device state; saved-state phases now omit
that override. Final native.json is acceptance, not the earlier failed attempt.

Exact tested binary installed,2 real state/plugin JSON files unchanged; rollback
.previous is47566065. Running real app remains PID1182755 (last checked previous
increment); not restarted. Save + full Quit/reopen loads updates. Install report:
target/smart-archive/install.json. No USB/personal rule changes, distribution
repackaging, commits, pushes, agents, other-chat messages or memory edits.
REA remains OPEN/HOT; this format work reused already established vendor contracts.

Next Smart gaps: application icons, non-KDE focus backends, exact native duplicate
priority and physical acceptance. Official Windows/macOS acceptance of the Linux
archive extension remains unverified. Broader matrix still includes undo,
multi-root sharing, packs and remaining plugins/devices. Keep full goal active.

## Latest full-parity increment: Smart bindings in profile copies, 14 October 2026

Installed/tested SHA-256:
`47566065549fd95d0528f09325b2327893e347fc6a23c74994e45784c089fb38`.
Full parity remains active. Same-device Duplicate and Copy to device now carry an
independent Linux Smart binding. Destination enable/default/existing bindings
stay intact, including default-off for a new scope. Copy still activates on the
destination while retaining the source editor device and same-focus manual
override. Imported AppIdentifier metadata stays unchanged. Archive portability
of Linux associations remains open.

Host::commit_profile_change in library.rs shares transaction publication with
Smart automatic selection: validate and save graph/navigation/routing before
lifecycle refresh; restore all on persistence failure. A refresh failure reports
that the change was already saved. publish_profile_copy is shared by duplicate
and device_pool copy. No new framework or persistence format. Native copy dialog
explains binding and destination-preference behavior.

Passed:10 Smart tests (including independent rules, cross-device defaults/manual
activation and held-action-safe rollback),5 library tests,5 device_pool tests,
19 frontend Smart/multi-device tests, strict all-target Clippy, frontend/release
builds and whitespace/Python syntax checks. tests/native_smart_copy_interop.py
uses real duplicate/copy dialogs and private simulated devices; verifies restart,
independent rules and cropped artwork exported after deleting its original source.
Acceptance: target/smart-copy/native/native.json; screenshots visually inspected.
Initial native failure was test JSON property-order comparison, corrected to
compare binding fields. No product defect hidden by the correction.

Exact native-tested binary installed;2 real state/plugin JSON files unchanged;
.previous is Smart build2aa5e4e9. Running real app was not restarted. Save + full
Quit/reopen loads the update. Install proof: target/smart-copy/install.json.
No USB, personal rule changes, new distribution archive, commits, pushes, agents,
other-chat messages or memory edits. REA remains OPEN/HOT.

Next: Linux binding archive portability, application icons, non-KDE focus,
original duplicate-priority fidelity and physical acceptance; broader matrix
still includes undo, multi-root sharing, packs and remaining plugins/devices.

## Latest full-parity increment: KDE Smart Profiles, 14 October 2026

Full parity remains active. Smart Profiles are now connected to the native editor
and installed for KDE Plasma6, with real KDE Wayland acceptance on simulated
hardware. Profile options -> Smart Profiles opens per-device rules, optional
fallback, installed/recent application choices and Advanced identity matching.
Rules are off by default; no personal bindings were configured. Existing imported
AppIdentifier values remain intact. A unique native "*" marker is suggested as
the initial fallback. Linux rules remain private local preferences.

New src/host/smart_profiles.rs owns Config/Binding and runtime routing. Saved/Host
smart_profiles maps are keyed by stable device scope (not mutable serial/name).
Only current-device, compatible-layout profiles can activate; inactive-layout
bindings remain stored. Sidebar order wins multiple matches, with a UI warning
for exact duplicates. Native matching-list ordering is still unverified. A manual
selection stays until observed application identity changes; PID alone is ignored.
Layout changes reevaluate. Errors hold the current profile. Automatic selection
saves the replacement graph before lifecycle changes and rolls back on failure;
page/folder cursors are retained. Forget-device removes/restores rules alongside
its transaction. Config saves use an expected baseline and captured device owner.

The editor pause guard is atomic: hidden0, visible1, toolkit-minimized2, quitting3.
KWin reports own-window minimized state to handle Wayland correctly. Visible editor
or any plugin inspector pauses routing. Native Window show/restore holds immediately;
hidden/minimized routing is observed on the250ms host loop. Observer is lazy for
enabled rules or explicit dialog preview. Preview ends on actual hide/close.
Stopped workers retire without blocking input and are joined at shutdown.

Focus worker now uses a deterministic script name derived from canonical state
path under the session lock. A new owner unloads only that name, then replaces a
mode600 .smart-profile-focus.js file; normal exit removes script/file. Actual
SIGKILL/restart recovery of an owned simulated host passes. Current-user UTF-8
executable matching normalizes Linux's deleted-image annotation only if nlink=0;
a real filename ending in that suffix is preserved. Recent metadata stays in
memory, with no titles/content. Non-KDE focus remains unsupported.

A live fallback failure exposed an existing window bug: after Hide, KWin still
reported the supposedly closed test host as foreground. GTK destruction had been
queued after its loop stopped. GDK Display.flush after dropping the native view
now delivers destruction before background sleep. Real fallback then passed.
Historical failure-focus/state JSON under target/smart-profiles/native records
that diagnosis; only summary.json is current acceptance. Smart dialog footer
clipping was fixed; advanced fields are collapsed for a shorter normal workflow.

Installed/tested SHA-256:
`2aa5e4e93156a2c8095e3e505a010ce49d218a8dfe275b4b924ae101e02e6732`.
Exact current native-tested binary installed;2 real session/plugin JSON files
unchanged; .previous contains the file-drop buildab768783. Running real app remains
untouched, so Save + full Quit/reopen is required. Report: target/smart-profiles/install.json.

Passed:7 routing/persistence/ownership/cursor/model tests,2 focus receiver/kernel
identity tests,5 window-state tests,4 forget-device regressions,19 frontend
Smart/Profile ownership tests, strict all-target Clippy, frontend/release builds
and whitespace/syntax checks. tests/smart_profiles_interop.py configures real
WebKit, focuses an owned GTK fixture, proves visible pause, real minimize and hide
routing, manual override, fallback, restart, SIGKILL recovery and script/file
cleanup. Final summary/capture: target/smart-profiles/native/. Settings screenshot
visually checked. No USB; only test-owned windows/processes controlled.

Native file-open/draft regression passed in
 target/smart-profiles/file-open-regression/ before the final inactive-layout
binding-preservation adjustment; file/lifecycle code was unchanged afterward.
No new distribution archive generated. docs/SMART_PROFILES.md and PR-08 updated
with supported scope and remaining gaps. Existing system_app catalog now includes
desktop ID and declared StartupWMClass; installed choices prefer the declaration.
Native UI state retains Smart drafts across hide/show; successful saves clear the
matching retained draft even if the initiating WebView has gone away.

REA remains OPEN/HOT; last smart-profiles bundle142 records. Native parser/writer
0x1403d75b0/0x1403dc3a0, default/duplicate UI0x14029a670; original guide and binary
both suspend switches while editing. These are not Windows runtime acceptance.

Next: application icons and Linux binding export/copy behavior, exact native
priority, non-KDE focus backends and physical acceptance remain Smart gaps. Also
continue broader matrix: multi-root sharing, pack handling, undo and remaining
plugins/devices. Keep the full goal active. All existing work preserved; no
commits, pushes, other chats, agents or memory edits. A repeated-bug memory search
found only unrelated GPU/KWin freeze notes; none were used.

## Earlier foundation: Smart Profiles focus observer, 14 October 2026

Full parity remains active. Previous file-drop turn was progress. This turn starts
PR-08 rather than treating editing refinements as the whole goal. Smart Profiles
are NOT available yet: only the read-only focus observer and diagnostics are
implemented. No installation or automatic profile switching was performed.
Installed binary remains `ab7687835d5e7d3468677970aba63be97cd5a5077fe7a0346215efadb6f97e5c`.
Current source/release diagnostic build is
`1b5ee532c7be6421801b8a7dcd43cee3d178634db09345e0e0dafb24dec7e0e8`.

New src/focus_monitor.rs subscribes to actual KWin windowActivated and metadata
signals, reports the initial active window, and keeps32 recent app records in
memory. Data is PID, desktop ID, window class and current-user /proc/PID/exe;
no titles/content. Private DBus reports require a random nonce and bounded fields.
Bad owned reports set an error, untrusted nonce reports do not alter state.
Session calls/initial report bound2s; owner change causes reconnect. Graceful
Drop unloads the uniquely named temporary script. Snapshot reports unexpected
thread termination. Error snapshots can retain last-known focus: future routing
MUST check error before acting. Abrupt-death recovery is NOT implemented; add a
per-state owned-script lease/recovery before enabling this as a background service.

Main has hidden read-only focus-probe --seconds1..30 [--events]. It explicitly
starts this observer and emits JSON/JSONL; ordinary app startup does not create
it. No host/profiles/USB are involved. FocusProbe currently joins normally on its
bounded duration; forced signal termination is not the verified cleanup path.

Passed: focus receiver/validation/recent-list unit test, strict all-target Clippy,
release build, syntax and whitespace checks. Actual KDE Wayland probe received
foreground metadata and isScriptLoaded returnedfalse after Drop. Current desktop
is KDE/wayland. Desktop ID was empty for the foreground ChatGPT app while class
Chatgpt and executable /usr/lib/chatgpt/ChatGPT were available: bindings cannot
assume every window exposes desktop ID. Initial probe: target/smart-profiles/focus-probe.json.

New tests/focus_monitor_interop.py builds a short-lived owned GTK window, activates
ONLY its still-live PID through a temporary KWin test script, observes real focus
arrival/executable and departure, verifies both scripts unloaded, and tests a
missing private bus address failing closed. Final current-build run passes.
Reports: target/smart-profiles/focus-native/{summary.json,events.jsonl,observer.log}.
No profiles or real user state changed, no USB; the separate fixture changes focus
to its own window. The observer itself never activates windows.

Native evidence: AppExecutable SHA2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3.
0x1403d75b0 reads optional AppIdentifier;0x1403dc3a0 serializes it.
0x14029a670 identifies AppIdentifier="*" as default profile and compares the
current profile with the first matching UUID for duplicate warnings. Underlying
matching-list order still unresolved; do not claim a specific ordering yet.
0x14165f390 says switching applies when the app window is closed. Official Elgato
Smart Profiles guides also require editor minimized/closed to prevent edits being
interrupted. Links/evidence in docs/SMART_PROFILES.md. REA remains OPEN/HOT; bundle
../evidence/smart-profiles/rea-bundle.json142 records. Runtime symbol lookup for
GetForegroundWindow was unresolved; an empty string search was not absence proof.

Next is the actual switching engine + native settings UI, not completion:
- Add per-device config/bindings/default choice to Saved/Host/persist/snapshot;
  Host constructors are near2896 and3505. Keep imported AppIdentifier raw values
  intact and require explicit Linux desktop-ID/class/executable mappings.
- Route via with_device + select_profile, preserving device ownership, page/folder
  cursors, profile geometry and saved drafts. Avoid reasserting manual selection
  while the same application remains focused. Handle errors without fallback.
- Suspend while editor visible; minimized needs native Window::is_minimized()
  (Tao returnsbool) alongside Window hidden state. Worker refresh can report it.
  Consider open owned inspectors as editing. Headless/legacy behavior needs scope.
- Start observer only when configured/previewed; use host monitor interval250ms.
  Worker Drop joins, so take/drop retired workers outside host mutex (existing
  expired-update spawn_blocking pattern near3177) to avoid blocking input.
- Add captured-device/profile settings dialog using existing application_catalog
  and recent observed IDs; catalog currently has name, desktop path, executable,
  but no desktop ID. Test actual focus-driven profile changes before installing.

Other full-parity gaps and non-KDE focus backends remain open. Existing changes
preserved; no commits, pushes, other chats, agents or memory edits. Goal stays active.

## Latest full-parity increment: native file-drop routing, 14 October 2026

Full parity remains active. The main WebKit view now registers Wry's native file
DragDropEvent handler. A single action file dropped over a canvas key/dial/Infobar
imports into that control. Outside the canvas it loads the clipboard; profile
files import through the existing profile path. Hover outlines targets and Leave
clears it. Unsupported/multiple files are explicitly rejected, with no partial
import or default file navigation. Plugin inspector views are unchanged.

New src/desktop_file_drop.rs normalizes absolute bounded UTF-8 paths and GTK
widget-logical coordinates using WebKit page zoom (not display scale factor).
The Wry0.57 GTK implementation was inspected locally: its drag_motion/drop signal
coordinates are forwarded directly. Events carry a window generation and old
ones are ignored. No filesystem reads occur on hover.

New frontend/src/fileDrop.ts handles native notifications and DOM hit-testing via
data-drop-controller/index. Repeated motion within one control avoids redundant
state updates. App captures device/profile/page/target context, blocks readonly
or dirty targets, modal dialogs and concurrent edits, and reuses slot_import's
transaction. Profile/background drops call open_file with the captured device;
Controller::open_desktop_file_on exposes the existing operation for that scope.
Internal HTML action/slot drop handlers remain in place.

Installed/tested SHA-256:
`ab7687835d5e7d3468677970aba63be97cd5a5077fe7a0346215efadb6f97e5c`.
Exact native-UI-tested binary installed;2 real session/plugin JSON files unchanged;
.previous is the desktop-file-open build. Running real app untouched. Full restart
loads this update. Report: target/file-drop/install.json. File associations remain
Deckforge for action/profile types; URI default deckforge-2.desktop preserved.

Passed: Rust native event/path/zoom test;17 frontend multi-device/drop tests;
TypeScript/frontend and release builds, strict all-target Clippy, Python syntax
and whitespace checks. New tests/native_file_drop_interop.py drives the real
WebKit UI and host with SYNTHETIC native notifications and actual archive files:
hit-testing, hover/leave, target import without clipboard changes, unsaved target
rejection, bad/multiple files, modal blocking, background clipboard, profile
import, draft retention and restart after deleting the source files all pass.
Reports/captures: target/file-drop/native/; final screenshot visually inspected.
No USB or physical pointer injection. Actual OS/file-manager drag gesture and
compositor/DPI acceptance are NOT automated or claimed. Manual acceptance remains
open; do not mark UI-04 complete based on this test.

FILE_OPEN.md and FEATURE_PARITY.md updated with that evidence boundary. No new
REA queries this turn; REA remains OPEN/HOT, last file-open bundle134 records.
The distribution tar at target/file-open remains the prior file-opening milestone;
no new distribution archive was generated for this UI increment.

Next: multi-root sharing, plugin/asset-pack file handling, system clipboard,
undo and the broader matrix. Physical drop, original-app and hardware acceptance
remain open. Existing changes preserved; no commits, pushes, other chats or
agents. Keep the full goal active.

## Latest full-parity increment: desktop action/profile file opening, 14 October 2026

Full parity remains active. The desktop launcher now accepts local paths and
file:// URIs for .streamDeckAction and .streamDeckProfile. Actions load into the
clipboard, leaving existing assignments/profile/drafts alone until paste; profiles
use the existing importer. Files reopen the current process/window through private
IPC; cold launch also works. Each queued file result has a unique UI notice/error
so a prior transient message cannot conceal an import failure.

New src/file_open.rs validates local regular files, allowed extensions, encoded
paths and limits; remote schemes/hosts, queries/fragments/control characters are
rejected. Main accepts a bare URI/path or desktop --open, preserves plugin URI
routing, and requires the native editor. instance.rs adds FileMessage and an
optional file callback while keeping existing window/deep-link APIs. The native
callback only queues work; the existing64-job editor worker parses it. IPC's
one-second response means accepted/queued, not completed. Worker starts before
publishing the endpoint. No GUI wait occurs inside the IPC callback.

Shared-action staging now feeds either a destination paste or the clipboard.
Standalone encoder actions can be opened while a Mini is selected; folder cells
still must fit the staging geometry. Clipboard-only source archives carry private
ownership and are cleaned on replacement/clear and after successful shutdown
save; pasted graph references retain their archive. Invalid files retain the
previous clipboard. Source files are never modified. Generic archive errors no
longer incorrectly call an action file a .streamDeckProfile ZIP.

assets/deckforge-mime.xml declares action/profile ZIP subtypes. Installer writes
MIME data before rebuilding desktop caches, advertises both types and preserves
existing defaults. Distribution builder includes the XML. Installed MIME detection
and defaults were verified with GIO/xdg-mime: both new types resolve to
Deckforge; existing streamdeck URI default remains deckforge-2.desktop.

Installed/tested SHA-256:
`1ce902946cc32d7ea4f618d54ff6f120c87ddd58a3e0abfbabf59aeaa4d1b53c`.
Exact native-tested binary and distribution contents verified.2 real session/plugin
JSON files unchanged, rollback .previous retained, running real app untouched.
Full restart is REQUIRED before desktop files can reach an older running host.
Report: target/file-open/install.json. Archive: target/file-open/deckforge-linux.tar.gz
(local build only, not published; uses existing Discord release via --skip-build).

Passed:25 transfer tests (7 sharing),14 instance tests, desktop-file CLI/routing
and local URI/path tests,14 multi-device/file-notice frontend tests, strict
all-target Clippy, frontend/release builds, Python syntax/whitespace checks.
Isolated tests/file_mime_interop.py verifies actual GIO types, case-insensitive
extensions and mocked default preservation. Distribution MIME XML/installer and
binary hashes match current files/build. No full distribution runtime suite rerun.

New tests/native_file_open_interop.py drives gio launch through the actual %u
Exec shape in private state. It verifies same owner PID, reopening a hidden
window, unsaved draft retention, action clipboard/paste, invalid-archive error
visibility without replacing clipboard/draft, cold profile import and retained
pasted source archives after shutdown. Final report/captures: target/file-open/native/.
A setup-signal/hide ordering race was fixed in the fixture: Python now waits for
the result, requests Hide, and waits for window_open=false before reopening.
No production change was made for that harness race. No USB used.

REA remains OPEN/HOT. Native0x140181110 is explicitly
ESDProfileManager::importActionToClipboard, distinct from importActionToCanvas;
it queues the shared importer without a selected destination. Evidence bundle:
../evidence/file-open/rea-bundle.json134 records. Docs: FILE_OPEN.md, IN-04 matrix.

Next: file drops onto controls, multi-root sharing, plugin/asset-pack file
associations, system clipboard interoperability, undo and the broader matrix.
Original-app/hardware acceptance remains incomplete. Preserve existing changes;
no commits, pushes, other chats or agents. Keep the full goal active.

## Latest full-parity increment: read-only nested inspection/Copy, 14 October 2026

Full parity remains active. Read-only container profiles now allow selecting
children, switching Multi Action Switch branches, inspecting nested fields and
Copy. Mutation controls stay disabled: Cut/Paste, Add/Remove/reorder, plugin
settings and editable fields. Keyboard mutations and action drops are guarded.
Host read-only checks are unchanged and remain authoritative.

App's scrollable inspector fieldset now disables only during operations. Inner
inspector-edit-fields protect the readonly root settings; Sequence receives an
explicit readOnly prop and scopes sequence-edit-fields around its mutation
controls. Child selection, branch navigation and Copy stay outside those groups,
including recursively nested Sequence editors. CSS keeps the established layout.
The App clipboard callback also rejects non-Copy requests from readonly views.

Fixed a legacy selection bug found during inspection: runtimeChild previously
matched every absent ActionID to the first same-UUID child. It now resolves
missing IDs by row position plus UUID, while explicit IDs remain identity-based.
Blank positional slots return no runtime child. Source data is not normalized or
changed by selecting/copying it.

Installed/tested SHA-256:
`5f565ffdbe7d89b976c9edba9d2716b74226976394061373873ec75af65c5de9`.
Both native reports match the exact installed binary.2 real session/plugin JSON
files unchanged; `.previous` is the nested-Cut build. Live real app untouched.
Full restart loads the update. Report: `target/readonly-children/install.json`.

Targeted frontend checks pass:10 Sequence,13 multi-device and19 device tests;
TypeScript/frontend and release builds pass, as does whitespace verification.
This increment changes no Rust behavior, so the prior Rust/Clippy evidence is
reused. New `tests/native_readonly_children_interop.py` creates a private source
profile, marks it readonly while its owned process is stopped, then drives the
real WebKit editor: Key Logic -> Multi Action Switch -> branch2 -> second of two
same-UUID children without ActionID. Copy into an editable profile retains the
second child's duration34, source raw data stays identical, UI/drop/keyboard and
host mutations are blocked, and the pasted action survives restart. Report and
capture: `target/readonly-children/native/`; readonly screenshot visually checked.

The writable native Copy/Cut/Paste regression also passes, including3 immediate
Escape cycles, moving cropped artwork, restart and PNG re-export. Report:
`target/readonly-children/writable-regression/`. All fixtures private/no USB.
NESTED_CLIPBOARD.md and FEATURE_PARITY.md updated. No new REA queries this turn;
REA remains OPEN/HOT, last bundle action-clipboard/rea-bundle.json132 records.

Next editing gaps: multi-root sharing, file-manager drop/MIME integration,
system clipboard interoperability and general undo, plus the broader matrix.
Original-app/hardware acceptance remains incomplete. Existing changes preserved;
no commits, pushes, other chats or agents. Keep the full goal active.

## Latest full-parity increment: nested Cut and move integrity, 14 October 2026

Full parity remains active. Nested actions now offer Cut selected and Ctrl/Cmd X;
source data stays in place until paste commits, and Escape cancels. Moves work
within/between containers, onto canvas, and across devices. Occupied canvas
swaps return the old target to the source container only if compatible. Positional
Trigger/Key Logic sources become blank; lists remove the row. Ancestor swaps,
stale sources and incompatible destinations fail before publication.

Anchor now records root context, immediate parent/branch/index and a raw-record
fingerprint for legacy children without ActionID. Stable-ID cuts recapture latest
settings; positional records reject changed occupants. Clipboard snapshot exposes
root_context so unsaved parent drafts block moving their child from another view.
UI shows a dashed cut row, updates operation status and selects the actual moved
row after same-list index adjustment. Read-only nested controls remain a gap.

`transfer/nested.rs` rehydrates the edited child order while retaining each actual
instance's media/resources. This prevents shifted positional IDs from inheriting
a neighbor's private bindings. Selected Stack/Wheel child identity is maintained
through insertion/removal/reorder using the model parsers' normalized CurrentIdx.
`library::refresh_page` now cancels changed Multi Action runs against the old tree
and clears pending changed-stack gestures before disappear events. A held-child
test proves keyUp occurs before willDisappear; rollback retains source/clipboard.

Native testing caught intermittent immediate Escape cancellation: the global
shortcut listener used a passive effect and could retain the old busy state after
the controls became ready. It now uses useLayoutEffect. The final native fixture
waits for rendered readiness, performs3 immediate Cut/Escape cycles without an
added delay, then moves the cropped dial child to a second stack. The final
build passes restart and re-export with3 exact200×100 cropped PNGs after source
removal. Earlier failed reports are not acceptance evidence. The runner now
removes stale summary output before starting and records the exact tested hash.

Installed/tested SHA-256:
`ffd9d35188939e80b4aa5a2a1363a9377bf455263a2a37f767e2455cd69ce8c5`.
Exact native-tested binary installed;2 real session/plugin JSON files unchanged;
`.previous` contains the previous nested-copy build. Real running app untouched.
Full restart loads it. Reports: `target/nested-cut/install.json` and
`target/nested-cut/native/native.json`; final edit screenshot visually checked,
showing Copy selected/Cut selected and correct Pasted action status. No USB.

Passed:24 transfer tests (5 new Cut/move tests),5 library tests,21 frontend tests
(8 Sequence,13 multi-device/draft tests), strict all-target Clippy, frontend/release
builds and whitespace checks. Native test is the expanded
`tests/native_nested_clipboard_interop.py --output target/nested-cut/native`.
Coverage includes cross-device moves, occupied/ancestor/incompatible swaps,
legacy-ID media, positional blanks, current selection, rollback and held release.

Docs: NESTED_CLIPBOARD.md and FEATURE_PARITY.md updated. No new native reverse-
engineering claims for Cut; REA remains OPEN/HOT with132 records last exported to
`../evidence/action-clipboard/rea-bundle.json`. Earlier native action clipboard
is an in-process stash; portable vendor MIME interop remains unestablished.

Next: read-only child copy/inspection, multi-root sharing, file-manager drop/MIME
integration and undo remain editing gaps, plus the broader matrix. Full parity,
original-app acceptance and hardware acceptance are incomplete. Existing work
preserved; no commits, pushes, other chats or agents. Keep the goal active.

## Latest full-parity increment: nested clipboard, 14 October 2026

Full parity remains active. Saved children now support Copy selected and Paste
inside Multi/Switch/Random, Dial Stack, Action Wheel and positional Trigger/Key
Logic editors. A nested copy can also be pasted onto the canvas. Settings,
unknown fields, resource bindings, cropped artwork and referenced folder pages
are preserved with fresh identities. Save/revert pending container drafts first.

New `src/host/transfer/nested.rs` validates compatibility, declared support,
branches/indices, full raw destination baseline, nesting and folder geometry,
then uses the shared `publish_transfer` transaction. `capture` now finds nested
contexts through the owning root/page; nested Cut remains rejected. Root Cut
into a container is supported with rollback/ancestor checks. Canvas placement
clears child-only flags/event coordinates/IDs; Delay cannot escape to canvas.
Custom titles normalize before hydration to prevent losing owned image pixels.

Sequence has a compact clipboard row and handles Ctrl/Cmd C/V while preserving
text input shortcuts. Delete edits its selected child draft. Ctrl X cannot cut
the enclosing control accidentally; nested Cut remains an explicit gap. A native
test exposed paste completion reselecting its old destination after a later user
selection; removing the redundant setSelected fixes that, with a frontend test.
Read-only container controls remain disabled and are still an open UI limitation.

Installed SHA-256: `5d4a4fc3ce4d50355cfbeea8ceff006723db869b2321406da506efb48014672f`.
Exact installed hash verified;2 real session/plugin JSON files unchanged;
`.previous` contains the action-sharing build. Live real app untouched. Report:
`target/action-clipboard/install.json`. Full restart loads this update.

Passed:19 transfer tests (including6 sharing and4 new nested tests),5 child
hydration tests,8 Sequence and12 multi-device/clipboard frontend tests, frontend
and release builds, strict all-target Clippy, diff whitespace check. The Clippy
result preceded only a Rust error-message wording change; no later Rust behavior
changed. Native `tests/native_nested_clipboard_interop.py` drives actual WebKit
buttons and keyboard events for Multi/Stack copying, child-to-canvas paste,
restart and export of3 cropped200×100 PNGs after deleting the source. Final native
run passes after the selection fix. Captures/report:
`target/action-clipboard/native/`; final edit screenshot visually checked showing
Dial Stack and Copy selected/Paste after selected. No USB used.

Evidence changed the initial priority: original ESDActionClipboard vtable
0x140ebcec0 stores an in-process shared stash;0x140463450 chooses nested lookup
0x1403f71a0 for negative coordinates versus0x1403f7780 for canvas addresses.
0x140463180/0x140463580 delegate paste;0x140463610 clears the shared pointer.
No portable action MIME format is established. System clipboard interoperability
remains a Linux extension/gap, not proven absent vendor behavior. See
`docs/NESTED_CLIPBOARD.md`. REA is still OPEN/HOT on the same Windows executable;
`../evidence/action-clipboard/rea-bundle.json` has132 records. No reimport needed.

Next: remaining editing gaps (nested Cut, read-only child copy, multi-root sharing,
file-manager drop/MIME integration and undo), alongside other full-parity matrix
requirements. System clipboard interop remains open but should not be confused
with the traced in-process action clipboard. Original-app/hardware acceptance is
still incomplete. Existing work preserved; no commits, pushes, other chats or
agents. Goal must remain active.

## Latest full-parity increment: action-sharing files, 14 October 2026

The full-parity goal remains active. Edit now offers Import Action and Export
Action. Single-root `.streamDeckAction` archives preserve nested actions, folder
pages, custom artwork and resource bindings. Import uses the existing paste
transaction, renews identities, retains the clipboard and does not add a profile.
Export uses the traced `$storage` / `Fake/Storage` profile/resource ZIP format.

New `src/host/transfer/sharing.rs` is a child module of transfer, reusing its private
capture/paste helpers. `library::prepare_bundle` extracts existing import staging
without publishing a temporary profile. Export forces the wrapper for Fake/Storage.
The descriptor records actual Deckforge/Linux versions and all known nested plugin
dependencies. Native DeviceSettings is now identified (previously unknown field).
Neo infobar storage coordinate0,0 converts to managed1,0. Import checks the selected
device/profile/page/target, controller support and every folder coordinate before
replacement. Unsupported multi-root archives are explicitly rejected. Failed
unpublished imports remove their staged source; successful imports keep private
archive provenance for restart/re-export after deleting original files.

Installed SHA-256: `07dbebc9093d3ff45f0c58a110a95850a399cc35f77fbeae09fafced4a28e0c4`.
Installer verified exact binary hash;2 user session/plugin JSON files unchanged;
`.previous` contains the prior cropper build. Live real app remains untouched.
Full app restart loads this version. Report: `target/action-sharing/install.json`.
Both desktop launcher icons still resolve to a valid PNG.

Passed:6 new sharing tests,9 existing transfer tests,5 library tests,13 export
(2 artifact-gated tests ignored),11 frontend multi-device/cancellation tests,
frontend/release builds, strict all-target Clippy and diff whitespace check.
New `tests/native_action_sharing_interop.py` checks native menu availability/bounds,
real host import/export, clipboard, restart and re-exported200×100 PNG pixels after
removing the chosen files. Report/screenshots: `target/action-sharing/native/`;
share screenshot visually inspected. Native OS file dialogs are not automated;
frontend tests mock the picker to verify dispatch, cancellation and device/page
ownership. Initial native harness attempts to mock the bridge/IPC failed because
they are immutable; the corrected native test uses real host commands and only
claims menu/API checks. No USB or real user-state mutation in fixtures.

REA remains OPEN/HOT on the supplied Windows executable; exported bundle
`../evidence/action-sharing/rea-bundle.json` has127 records. Native functions and
limitations are documented in `docs/ACTION_SHARING.md`. Entry exports0x14085f280,
stashes0x1403c8170, common packer0x1403c8c50, reader0x1403bf260, descriptor fields
0x140436110/0x140436730. No cold import is needed. `Fake/Storage` root count1 is a
verified Linux subset; official Windows/macOS acceptance remains untested.

Next: continue UI-04 with native clipboard format/ownership evidence, multi-root
placement and undo. File-manager drops/MIME integration also remains open. Full
parity is not complete; other checklist gaps persist. Existing changes preserved;
nothing committed, pushed or published. No agents or other chats used.

## Latest full-parity increment: per-action Customize Image, 14 October 2026

The full-parity goal remains active. Selected encoder-display actions now offer
Customize background, Remove background and Show feedback icon. The cropper
implements pan (pointer/arrows/Shift), quadratic zoom, width/height alignment,
cover/reset, snapping and GIF/WebP still-frame selection. Only Set Image writes
the transparent200×100 PNG. The original per-action dialog disables animation
output; no Set Animation behavior is claimed for this scope.

`src/image_crop.rs` decodes/crops outside the engine lock. The native bridge's
`image_crop` load/save API validates action/device ownership, readonly state,
source hash and the prior background reference. Rechecks occur after decoding.
Regular-file reads are nonblocking; input16MiB/16MP/8192-side/1024-frame and256MP
frame-seek work limits are explicit. Scaled crop dimensions are capped4000.
Front-end snapping also stays within this bound for extreme aspect ratios.

Slot.encoder_overrides stores cropped PNG bytes in session state; native export
embeds them under Encoder.background, never exposing private override fields.
Copies retain the crop without its original source file. Child hydration retains
owned pixels only while the raw reference matches. Export's local-action test
now uses missing raw UUID rather than an empty raw map: new appearance metadata
must not prevent embedding default plugin artwork. A test verifies the default
plugin icon and cropped background both survive copy/export.

Crop drafts survive native window hide/show in the same host, not full process
restart. Saved images survive app restart. Completed saves clear retained dialog
state even if the old WebView was hidden. The dialog uses existing Modal focus/
Escape handling and blocks dismissal during Save. A narrow-layout check exposed
an offscreen Save button; tighter spacing, responsive preview sizing and a sticky
footer fixed it. The OS file picker itself is not automated.

Installed SHA-256: `c3d68f631aacbc6dd236ad1482717048a3979c2ba8168944fa16da5afb0aa81d`.
Session/plugin JSON unchanged; rollback `.previous` exists; real running app was
left alone. Full restart loads the update. Report: `target/cropper/install.json`.

Passed:3 crop/source/ownership/persistence Rust tests; encoder asset,13 export
(2 artifact gates not run),5 child-hydration tests; strict all-target Clippy;
11 frontend cropper/multi-device tests; frontend/release builds. Installed-binary
native test passes cancel, real GIF frame pixels, fit/reset, keyboard crop,
window draft restoration, saving, export and restart without source. Captures/
report: `target/cropper/native-installed/`. Chromium layout tests separately
verify reported1180×763 and760×533 viewports in `target/cropper/layout/`; screenshots
visually inspected. These do not prove manual pointer/hardware acceptance or
exact Qt rasterization. No USB used.

Frontend-developer skill was read/applied, preserving the established theme.
New files: frontend/src/ImageCropper.tsx, crop.ts, ImageCropper.test.tsx;
src/image_crop.rs, src/host/image_cropper.rs; tests/native_cropper_interop.py and
cropper_layout.py. Existing Modal gained an optional closeDisabled prop.
Native contracts are documented in ENCODER_APPEARANCE.md. REA remains OPEN/HOT;
`../evidence/galleon-display/rea-bundle.json` has112 records. Reset-cover uses
KeepAspectRatioByExpanding2; zoom side=40+(slider-40)^2/3960. No provider reset is
needed for later queries.

Next: return to the complete FEATURE_PARITY matrix and prioritize remaining
user-visible gaps such as system clipboard/action-sharing/undo (UI-04), broader
asset libraries/global Encoder.Icon editing (UI-06), and tutorial/onboarding
(AC-10). Exact snap/rounding, large-image timing, Windows acceptance and physical
checks remain open; the overall goal is not complete. Existing changes preserved;
nothing committed, pushed or published.

## Latest full-parity increment: original per-action Encoder artwork, 14 October 2026

The full-parity goal remains active. A new trace corrected the background scope:
Create New Background is a per-action200×100 cropper, separate from the page's
Controller.Background. Both ESDPropertyInspectorDialHelper and DialStackHelper
vtable slot53 point to0x140852170, which opens ESDBackgroundCropperDialog and
applies its result via0x1403a98b0/0x1403bb7c0. Constructor0x1405d46d0 names it
Customize Image; zoom slider range40–4000. UI/crop/output details remain to port.

The required rendering foundation is implemented. Imported action fields
`Encoder.background`, `Encoder.Icon` and `Encoder.IconVisible` are now honored.
The per-action image overlays its page background; IconVisible disables only the
feedback `icon` item, and the software dial icon remains separate. Cached assets
use action provenance, including copied/nested actions. Cross-profile export
rebases both Encoder paths when filenames collide, preserving bytes and unknown
metadata. Original references remain unchanged for untouched profile exports.
Per-action backdrops pass through Stack/Trigger/Wheel composition and draft
rendering. No new per-action editing controls are claimed yet.

Installed SHA-256: `02a4b66012ede645386488758ffe0b0211f18358aee695cd9279b7cfec9354e7`.
Session/plugin JSON unchanged; rollback `.previous` exists; running app untouched.
A full restart loads the update. Report: `target/encoder-assets/install.json`.

Passed: imported Encoder asset/visibility/alpha/draft/copy-conflict test,
7 display-target tests,10 Trigger tests,8 Wheel tests,13 export tests (2 artifact
gates not run), strict Clippy and release build. Native WebKit test imports the
untouched original Plus Default Profile, verifies custom software icon and
hidden feedback icon, exports original assets/metadata, then repeats after
restart. Transparent Swipe-left tip composition matches browser compositing
within1 channel value. Initial raw-PNG comparison correctly failed because it
omitted the underlying page artwork; the reference now includes that layer.
Capture visually inspected. Reports: `target/encoder-assets/native/`. No USB.

New module: `src/host/encoder_assets.rs`; contracts and evidence:
`docs/ENCODER_APPEARANCE.md`. Original profile source:
`../evidence/elgato-7.6/raw/flsysF8Fq6CoTq162E1zc6fyeIlKcw`. It contains
Encoder.Icon/Background examples in pageWIKRPB7W1T6CR20HJBRDODGG60Z; the tip
PNG35BC43FCJ120WB9GK09RWQT5LOZ is200×100, custom iconNHBD8ALK8T07P17OCJ7174QJWCZ
is48×48. Preserve the source archive; do not copy its assets into application
source. Similar fixtures exist in flsLUxNAo3HxRBo8yay7EbNtFyJC9c and
fls1.KgkvZ_osjTGmFQpIDeeaNhGPU.

Next: implement per-action Customize Image/cropper and native Encoder appearance
controls, with correct serialization/global-vs-state semantics. Existing generic
per-state appearance controls do not yet cover all these native Encoder fields.
Broader SDK precedence, animated action backgrounds, Windows edited-export
acceptance and physical output remain unverified. Then continue the full parity
matrix; onboarding/tutorial and other broader gaps remain explicit.

REA remains OPEN/HOT. Updated `../evidence/galleon-display/rea-bundle.json` has
109 records. UI/action RTTI tables are in background-menu-tables.json and
action-tables.json. The relevant base class is ESDAction0, not ESDAction. Native
options parser/serializer0x1403d4bd0/0x1403da550; background layer0x14042aa20;
IconVisible-to-icon.enabled mapping0x1403bc610. No close/reimport is needed.
Existing changes preserved; nothing committed or pushed.

## Latest full-parity increment: Galleon animated WebP backgrounds, 14 October 2026

The full-parity goal remains active. Galleon controller backgrounds now stream
WebP frames with encoded delays, alpha/disposal handling and finite/infinite
loop counts. Zero-delay media advances one frame per scheduler turn. Completed
finite movies retain the final frame; decode errors stop at the last good frame
and log once. Source data remains immutable and exports byte-identically.

New `src/background_movie.rs` uses `image-webp0.2.4`, already present transitively
and now an explicit dependency. It retains encoded bytes and a decoder canvas,
not all decoded frames. Input16MiB/source16MP limits remain explicit. Playback
renders full720×384 backgrounds through existing editor/hardware composition.
`controller_backgrounds` moved into the device runtime swap so each device owns
its movie/cache. The active page and source key control restarts; page departure
cancels playback. Scheduler polls the next encoded deadline. Other models retain
the still-image behavior recovered from the native Galleon-specific override.

Installed SHA-256: `90a5a02c37aa25ba2ed0d744267e38c9e47a0bead170fffcd39c324a3640c57d`.
Session/plugin JSON unchanged; rollback `.previous` exists; running app untouched.
A full restart loads the update. Report: `target/galleon/movie-install.json`.
Only test harness/fixture edits followed the release build; production code is
unchanged from the native-tested and installed executable.

Passed:2 decoder tests covering timing, finite/infinite loops, partial-frame
alpha/disposal, zero delay and corrupt later-frame stop;3 background host tests
(including identical editor/hardware frames, full-resolution output, page cancel/
return and independent device caches/clocks);5 device-pool tests; all-target
Clippy. A tiny4×4 fixture originally gave different corner pixels after Lanczos
upscaling; the host pixel test now uses a full720×384 fixture, while decoder/Qt
reference tests retain the small exact source fixture.

`tests/background_movie_reference.py` compares owned synthetic WebP frames with
installed Qt6.12.0: delays40/80/120ms, two cycles, maximum channel delta1. This is
not Windows-app execution or proof of its Qt build. Report:
`target/galleon/webp-reference/comparison.json`. The explicitly gated reference
test was run and passed. `tests/native_background_movie_interop.py` verifies
native red/blue animation, page departure/return, original-byte export and restart
without the original file; report/captures in `target/galleon/native-movie/`.
No USB used. Uncommon disposal inputs, large-source CPU-load timing and physical
Galleon acceptance remain unverified.

Next: trace vendor Create New Background, then return to the broader
FEATURE_PARITY checklist. Callback0x140853140 is not a Ghidra procedure; bounded
assembly shows dispatch to UI object's virtual slot0x1a8 (index53). Do not repeat
that failed decompile. Assembly saved as
`../evidence/galleon-display/create-background-thunk.asm`. The owning UI object's
vtable/implementation must be identified. REA remains OPEN/HOT; updated display
bundle has91 records. Do not close/reimport merely to continue queries. Qt6.8
primary-source reference links and implementation limits are in
GALLEON_REFERENCE.md. Existing changes preserved; nothing committed or pushed.

## Latest full-parity increment: background file selection/removal, 14 October 2026

The full-parity goal remains active. Page options now offers Set display
background and Remove display background for encoder-display models. The file
dialog captures device/profile/page ownership before awaiting selection; switching
devices or pages cannot redirect the edit. The backend command `page_background`
requires captured `profile`, page `id`, and `path` (null removes). It can update
an inactive captured page without navigating the current editor.

Page has a serde-default `background_source: Option<AssetSource>` separate from
action/page provenance. Selected image bytes are validated, copied into a unique
managed background ZIP, and exported byte-identically under the page's Images
folder. Existing key/state assets, action settings and other imported files are
preserved. Page duplication and folder transfer clone the source; retention lists
protect both live-library and clipboard sources. Unreferenced decoded caches are
pruned on edits. Invalid/corrupt images, wrong device, missing page and read-only
profile checks precede mutation; persistence failure restores prior metadata and
removes only the newly created archive. Legacy page-level Actions are converted
to a Keypad controller when adding an Encoder controller.

Installed SHA-256: `f5a347c69640dc2a02c0d275e8d503a51a60488cd53d0e559cb800d919d84173`.
Session/plugin JSON unchanged; rollback `.previous` exists; running app untouched.
A full restart loads the update. Report:
`target/galleon/background-edit-install.json`.

Passed:2 background Rust tests,13 export tests (2 installer-gated tests not run),
8 frontend multi-device tests including delayed file-picker ownership, strict
all-target Clippy, frontend and release builds. Native test in
`target/galleon/native-background-edit/` verifies adding a selected file through
the real host bridge, the real Remove button, changed pixels, export preserving
original and selected bytes, and restart after deleting both original source
files. The platform file dialog itself is not automated; frontend tests cover
its request and ownership, and the native test supplies the chosen path to the
host. Screenshot visually inspected. No USB used.

Next: animated WebP controller backgrounds. Existing `src/key_animation.rs`
only decodes GIF, reduces frames to256×256, clamps delay25ms and has icon-specific
budgets; do not reuse it unchanged for720×384 backgrounds. Galleon native override
0x140421130 installs `.webp` in a movie layer and starts it. The controller cache
currently contains still pixels/PNG/tiles; playback will need device-owned clocks
and page-change cancellation through the existing scheduler. Also investigate
vendor Create New Background handler0x140853140 before claiming that workflow
matches the new file-selection controls.

REA was left open/hot in the prior increment; no REA calls or close happened in
this increment. Display evidence bundle remains
`../evidence/galleon-display/rea-bundle.json` (90 records); last saved snapshot is
older transport evidence. Preserve the active provider when continuing. Other
FEATURE_PARITY gaps and all physical Galleon acceptance remain open. Existing
changes preserved; no commit, push or publication.

## Latest full-parity increment: Galleon row marker and hold preview, 14 October 2026

The full-parity goal remains active. The approximate widget underline is replaced
with the recovered external rounded border: canonical cell expanded6, radius5,
white border2 and outline1 using RGBA30,30,30,180. The shared CElementImp
constructor0x1405ad310 confirms the outline; factory argument0x14042fe53 confirms
that the marker element is a rectangle. SVG/resvg rasterization is independent
of Qt, so exact antialiasing equivalence remains unproven.

A deferred physical press arms250ms checks. After >500ms, the old marker fades
out and the next assigned row fades in over150ms, sampled through the existing
33ms scheduler. Preview never changes the routed action or persisted Routers;
held rotation still targets the selected row. Release commits the next row.
Explicit cancellation, page changes and disconnect remove pending ownership;
disconnect refreshes the cached marker used by the editor. Completed fades stop
scheduling. The runtime is already device-owned through encoder_routing.

Markers are composed outside widget content onto the physical display frame,
and the same cached transparent PNG is overlaid on the editor's dial grid.
The overlay cannot intercept pointer input. Widget previews themselves retain
only their feedback content, avoiding the old clipped-underline behavior.

Installed SHA-256: `2dcf48e99f2662c7da91aac271c9036979390ab1f817739abecc2a77c4d65b7c`.
Session/plugin JSON unchanged; rollback `.previous` exists; running app untouched.
A full restart loads the update. Report: `target/galleon/marker-install.json`.

Passed:3 routing/preview Rust tests (including midpoint opacity, unchanged SDK
routing and cancellation),7 display-target regression tests,19 frontend device
tests, strict all-target Clippy and release build. Process routing/queue/export/
restart tests pass in `target/galleon/routes-marker.json`. Native screenshot and
owned SDK probe tests pass in `target/galleon/native-marker/`: selected border,
held preview without early routing, cancelled preview, balanced clicks and
visible border pixels. Capture visually inspected. No USB used.

Next: background selection/removal UI and animated WebP controller backgrounds,
then continue the complete FEATURE_PARITY checklist. Physical Galleon operation,
Qt pixel equivalence, LED mapping and other model gaps remain unverified.
REA remains OPEN AND HOT on Windows AppExecutable. Do not close/reimport just
to continue queries. Updated evidence bundle is
`../evidence/galleon-display/rea-bundle.json` (90 records). Last on-disk snapshot
is older `../evidence/galleon-transport/rea-snapshot.json`; the current hot session
and display bundle retain newer evidence. Previous changes preserved; no commit,
push or publication.

## Latest full-parity increment: imported controller backgrounds, 14 October 2026

The full-parity goal remains active. Encoder Controller `Background` still images
now render behind feedback, through empty widgets and across Galleon gutters.
The same archive-backed path works for Plus. It respects page asset provenance,
including copied folders/pages, caches decoded images and preview crops, flattens
alpha over black and preserves original files/reference strings on export.
Page changes clear/restore the display. Missing images produce one cached
warning and a regular-canvas fallback. Restart uses the managed archive even if
the original import file is removed.

Ordinary layouts, Dial Stacks, Action Trigger and Action Wheel use the owning
widget's background crop. Trigger rendering now receives the existing Display
context rather than seven parallel arguments. The native editor receives a
full Galleon background and individual empty-widget crops. Existing key rendering
and no-background behavior remain covered by their regression tests.

Installed SHA-256: `a89003ef3c170051d3c802c92126afd647ddb2a0a5267cc121f7c1ef1268a47c`.
Session/plugin JSON unchanged; rollback `.previous` exists; running app untouched.
A full restart loads the increment. Report: `target/galleon/background-install.json`.

Passed: background/import/copied-provenance/export/alpha test for Galleon and Plus,
7 display-target tests, 10 Trigger tests, 8 Wheel tests, 16 renderer tests,
19 frontend device tests, strict all-target Clippy and release build. Installed
binary native test verifies an unchanged PNG from the original profile, assigned
and empty-widget crops, visible screenshot pixels, page changes, byte-identical
export and restart after deleting the original import file. Reports/captures:
`target/galleon/native-background-installed/`. No USB used.

Next: implement the actual Galleon route marker and pending preview. New native
findings show the current underline is only an approximation. Function
0x14042fc70 builds a rounded border around each canonical widget expanded by6,
radius5, border width2, white, outline width1. Function0x14042f3b0 fades the old
marker out and next marker in over150ms. The existing250ms timer calls it after
>500ms; actual row routing only changes on release. Verify outline defaults in
the element constructor before claiming precise artwork.

Also still open: background selection/removal UI, animated WebP controller
backgrounds (native0x140421130 uses the movie layer), exact vendor rasterization,
and physical Galleon acceptance. Still-image background application is recovered
at0x1404213f0; default pixmap stretch rendering at0x1405c1380. Contracts/evidence
IDs are recorded in GALLEON_REFERENCE.md.

REA IS STILL OPEN AND HOT on the Windows AppExecutable to avoid another lengthy
cold analysis during this active goal. Do not reopen with a snapshot or close it
before the next queries. Evidence bundle exported to
`../evidence/galleon-display/rea-bundle.json` (88 records). Last saved snapshot is
older `../evidence/galleon-transport/rea-snapshot.json`; new display evidence is
in the bundle and current session. Pixmap RTTI table in
`../evidence/galleon-display/pixmap-table.json`; local original-background.png is
verification material, not an app asset. Existing changes preserved; no commit,
push or publication.

## Latest full-parity increment: Galleon USB transport, 14 October 2026

The full-parity goal remains active. Native-contract Galleon USB support is now
implemented: Corsair `1b1c:2b18`/`2b4b` with usage-page 12, usage 1, interface 0;
identity-selected startup; initial `03 27` keepalive, 200 ms preheat, firmware
read and 500 ms heartbeat. Firmware minor component >=6 selects output `02 25`
instead of feature `03 27`. This is the recovered minor-field gate, not semver.
Identity probes remain read-only; only the selected handle receives startup.

The driver supports twelve keys, two physical dials, signed turns, nonzero button
states, partial-key preservation, and safe rejection of truncated reports.
Unrecognized controller reports are ignored without waking the software saver.
Key/display/full-LCD JPEG output has native dimensions, identity orientation,
quality90 and chunk framing. Brightness maps percentages to4–100. Background
clear runs on initial output, saver exit and reconnect. The shared worker now
accepts Galleon's non-touch encoder display; a regression test caught its former
touchscreen-only gate. Readback-unproven sleep controls remain unavailable.
No firmware update or reset is issued. Enumeration now prints actual vendor IDs.

Installed SHA-256: `d1ca37ad4a10a34876b86bf0ac448b39e3ef0e7e8be8afdf729138ae6af372f1`.
Session/plugin JSON remained unchanged; rollback `.previous` exists. The running
app was left alone, so a full restart loads the new binary. Install report:
`target/galleon/transport-install.json`.

Passed: 36 hardware tests (2 hardware-gated tests ignored), 31 device-session
tests (1 hardware-gated test ignored), 8 focused Galleon checks, 19 frontend
device tests, strict all-target Clippy, release build, all ten model/process/
profile/restart cases, and native Galleon/Plus canvases. Reports/captures:
`target/galleon/models-transport.json` and `native-transport/`. The final build
only adds actual-vendor CLI display beyond the process-tested build; read-only
`deckforge devices` still finds only the existing Stream Deck+.

No Galleon is attached. Hardware startup, keyboard coexistence, firmware variants,
image orientation/output and wake/reconnect acceptance remain unverified. Do not
claim those from codec or fake-transport tests. Source contracts and evidence
IDs are in `GALLEON_REFERENCE.md`; REA bundle/snapshot were saved to
`../evidence/galleon-transport/` and the session was closed. New code is mainly
`src/hardware/galleon.rs`, with shared HID/session integration.

Next: implement Galleon Controller Background rendering and the native pending
row-preview behavior, then continue the full FEATURE_PARITY checklist. LED-RGB
mapping, physical acceptance and broader USB/model limitations remain explicit.
Existing changes are preserved; nothing was committed, pushed or published.

## Latest full-parity increment: Galleon dial routing, 14 October 2026

The full-parity goal remains active. Physical Galleon dial events now map through
saved Encoder Controller `Routers`. Multiple assigned rows defer presses:
release at <=500 ms replays the measured press duration; release after >500 ms
cycles assigned rows without action activation. Single assigned rows dispatch
immediately. Held turns preserve physical pressed state; cancellation and page
changes clear owned gestures. Rapid clicks use a bounded FIFO to balance events.
Native simulator buttons submit complete operations, avoiding stranded UI holds.

Native composer evidence corrected widgets from 360×192 to 320×160 at origins
(24,24), (376,24), (24,200), (376,200), within the 720×384 display. Active-row
underlines are shown for columns with multiple assignments. A real screenshot
test exposed CSS clipping at the image bottom; the preview now fits its content
box and the captured underline passes pixel verification.

Installed SHA-256: `5fdc2614751440fb0ba46087ce74df5b2206d523329bfe06f9de97366f2f1a43`.
Session and plugin JSON were unchanged. The current app process was left alone;
a full restart loads the increment. Rollback binary is retained as `.previous`.
Installation proof: `target/galleon/routing-install.json`.

Passed: routing process tests (including saved imports, SDK coordinates, timing,
held turns, repeated clicks, cancellation, export and restart), all ten model
process cases, Galleon geometry/frame Rust checks, 19 frontend device tests,
strict all-target Clippy, production build, and native controls with SDK events
and visible screenshot pixels. Reports: `target/galleon/routes.json`,
`models-routed.json`, and `native-routes/native.json` with `routes.png`.

REA routing/geometry bundle and snapshot are saved under
`../evidence/galleon-routing/`; the session is closed. Source is the supplied
Windows executable, SHA-256 recorded in `GALLEON_REFERENCE.md`. Key addresses:
press classifier 0x140422440, clock 0x14017f9c0, route sync 0x140423e60,
preview timer 0x140426390, composer setup 0x140420420/0x140429f30.
The executable threshold differs from Corsair's documented three-second hold.

Next: continue USB initialization/recovery and extra-report contracts using the
retained hardware evidence in `../evidence/galleon/`. Galleon transport remains
unavailable and no Galleon is attached. Background images, native preview/marker
animation and physical output acceptance also remain open. Do not enable USB
from VID/PID knowledge alone. Other full-parity gaps remain in FEATURE_PARITY.md.
Existing changes are preserved; nothing was committed, pushed or published.

## Latest full-parity increment: Galleon profiles and editor, 14 October 2026

The full-parity goal is still active. Galleon is now a distinct profile/editor
model, `galleon`, with original `GRETSCH`/`GRETSCH1G` identities preserved,
SDK type 12, a 3×4 key grid, a 2×2 encoder widget grid, and two separate physical
dial markers. Widget images are 360×192 in a 720×384 frame. It has no touch input.
A new runtime-only child coordinate override preserves the owning encoder's 2D
coordinates for nested Keypad actions. Existing model coordinates remain intact.
See `GALLEON.md` for implementation and `GALLEON_REFERENCE.md` for sources.

Installed release SHA-256: `62af9564e03d96307633c19d128e1e9f1bd4771b2b3b0b4c219b443f1117edaf`.
Session and plugin JSON were unchanged by installation. The current app process
was left alone; a full restart loads this increment. Reports/captures are under
`target/galleon/`, including the installed-build native smoke check.

Verification passed: all ten model host/SDK/export/restart cases, native Galleon
and Plus canvases, actual four-widget frame composition, 18 frontend device tests,
shared children/Trigger Rust checks and strict all-target Clippy. The complete
Action Trigger suite now passes all eleven original adaptor records. The complete
soundboard suite now passes, including captured original embedded MP3 PCM.
Their old fixture code had to use each profile's declared model, since importing
a foreign-model profile intentionally leaves the active profile unchanged.

Physical Galleon transport remains unavailable. Static investigation found
Corsair 1b1c:2b18 and 1b1c:2b4b, usage-page 0x0c, usage 1, interface 0; both use
factory 0x1405f16f0 and hardware table 0x1416a3320. Modern key/full-display/region
JPEG writers are shared. Base normal polling retains keypad 0x100, and Galleon
adds encoders 0x103; disabled callbacks are identifiers 0 and 0x112, NOT keypad.
An early interpretation to the contrary was corrected. Only Stream Deck+ is
currently connected. Initialization, extra-report handling, transforms and
physical dial-row switching need further work before enabling USB support.

REA evidence, assembly and vtable inventories are in `../evidence/galleon/`.
The session was saved to rea-snapshot.json and closed. Adapter secondary vtable
0x141672990 is important; its primary QObject table omits the rendering methods.
Next trace the physical dial-row selection and initialization/input path, using
this retained evidence. Other full-parity gaps remain in FEATURE_PARITY.md;
none of the physical unknowns means the overall goal is complete or blocked.

## Latest full-parity increment: Key Logic, 14 October 2026

The full-parity goal remains active. Key Logic is implemented using the recovered
300/500 ms classifier, three fixed child positions, the existing pulse/key ownership
system, and the independent SupportedInKeyLogicActions manifest flag. Native
right-click creation, nested settings, library drop, save/restart, ordinary SDK
children, unchanged Counter execution and Linux export/import passed.
See `KEY_LOGIC.md` and `KEY_LOGIC_REFERENCE.md` for evidence and limits.

Installed release SHA-256: `6500cea0fa5702058a657f351068b0674bcc8799953c55211e56e634d00abf34`.
Installation retained the saved session/plugin JSON byte-for-byte. The running app
was left alone; a full app restart is needed to load this increment. Reports and
native captures are under `target/key-logic/`. Prior local changes remain intact.

The shared Trigger/container Rust checks, Key Logic timing/ownership/opt-out tests,
frontend sequence/model tests and all-target Clippy pass. The existing broad
`tests/trigger_interop.py` passed its generated workflows but still failed at
`GRETSCH` import. Do not report that full suite as passing. This also blocks a
previous soundboard original-fixture run.

Next: implement Galleon support properly, starting with its profile/layout path.
`GALLEON_REFERENCE.md` records the authoritative original archive, retained native
geometry and official SDK type 12. It has a 3-column × 4-row key grid and four
logical encoder widgets in a 2 × 2 grid, controlled by two physical dials. It is
not a Plus alias. No Galleon source changes have been made yet. Preserve the
2D encoder coordinates, model identity and transport-unverified boundary.

## Active full-parity goal, 14 October 2026

The user explicitly requested full feature parity. The goal is active and must
remain active while required work remains. This is no longer a one-feature goal.
The original priority order still guides sequencing; deferred items remain visible
in the full checklist. No subagents were newly authorized.

This increment implements Open Application with native installed-app search,
literal arguments, KDE activation, 500 ms hold close/force-close/none, cancellation,
and restart preservation. Desktop IPC has its own bounded worker and cannot block
keyboard release. Launcher fixed arguments distinguish applications sharing one
interpreter. See `OPEN_APPLICATION.md` for platform and visual limitations.

Tests passed: 52 system Rust tests plus the host hold-cancellation test, 14 frontend
field tests, production frontend build, strict all-target Clippy, real owned Qt
window activation/close via KWin, force-close timing, process isolation, native
catalog/search/form saves, restart and system-action regressions. Reports are in
`target/open-application/`. Installer verification is in `install.json`; the current
running app was left alone. The new build takes effect after a full app restart.

Next implement Key Logic. `KEY_LOGIC_REFERENCE.md` records the recovered timing,
child dispatch and remaining serialization/runtime questions. REA bundle/snapshot
are in `../evidence/open-application/`; the session was saved and closed. Existing
uncommitted work remains preserved; nothing was pushed or published.

Installed release SHA-256: `ca9bcdab6a7e1084b7577babbed707d7742ee9de2766f3fcc54a3fb3032700eb`.

## Latest continuation, dial hotkeys and app icon, 14 October 2026

The user requested a new goal to continue the Linux port, then asked to fix the
missing app icon. This pass adds the original dial Hotkey queue behavior and
its Coalesce control. See `HOTKEY_COALESCE.md` for contracts and evidence.
The original Windows binary was analyzed statically through REA/Ghidra.

The installed and running release is
`7baa982e9ff2636a7637925a6ec91563604011681345e3b832cb700e91cc437b`.
It reopened the physical Plus and reconnected Discord, Codex, Spotify, Steam Hub
and Bluetooth. Rollback from before this pass is `~/.local/bin/deckforge.before-coalesce`.
No physical hotkey injection was performed. The complete port remains unfinished.

The existing SVG artwork now also ships as PNG. All three native WebKit window
builders embed it; the installer writes an absolute PNG icon path, refreshes icon
and KDE service caches, and the distribution includes both formats. The local
`deckforge-2.desktop` alias had the same old named icon and was updated with a
backup. Desktop identity already matched; do not infer icon emptiness from
KWin JavaScript's `QIcon()` string alone. Taskbar optical acceptance remains for
the user, since the panel was hidden in the available desktop captures.

Validation: 49 targeted Rust tests, 13 frontend field tests, frontend production
build, strict all-target Clippy, two-launch native Coalesce UI checks, system-action
process regressions, and the packaged distribution/isolated install/native plugin
check passed. Reports are under `target/hotkey-coalesce/`. Existing uncommitted
changes were retained; nothing was published.

## Latest continuation, 14 October 2026

The user resumed built-in/system and dial porting in **Investigate broken plugins**.
No new automatic goal was created. The installed release is now
`5e2f18640dc37a7b41713a03ac14e807f19fd37f70b417ed47b89120b87a24d2`.
The native editor is running this build. Saved profile data and plugin auth files
matched across installation; executable rollback is `~/.local/bin/deckforge.previous`.

Added Hotkey Switch, Close with optional force, Unicode clipboard Text, Timer,
and Mouse Wheel with held horizontal scrolling. Categories now match Stream Deck
and System. See `SYSTEM_ACTIONS_NEXT.md` for original-binary contracts and limits.
Tests cover native forms, host switch/close/timer behavior, and actual timer PCM
on an owned virtual output. Strict all-target Clippy passed before final packaging.
The broad soundboard suite reached its unchanged unsupported GRETSCH import
fixture after audio checks; the focused timer/output checks passed independently.

Bluetooth was independently remade for BlueZ; Spotify gained a local MPRIS
controller. ChatGPT & Codex now uses the Linux IPC socket and verified CLI discovery.
The live installed Codex plugin reports connected with a complete chat catalogue.
Its read-only validation sent no prompts, approvals or task changes. All original
plugin folders are retained under Deckforge's `plugin-backups`.

A Plus LCD background-clear fix also removes untouched boot/screensaver pixels
before normal key/strip replay. HID acceptance was verified; optical confirmation
was requested from the user. See `SCREEN_SAVER_PROTOCOL.md`.

Remaining priorities: direct Unicode typing, Coalesce, richer Open Application
activation/held-close behavior, Key Logic, further dial variants, and individual
Codex action acceptance. Voice/dictation observation remains unavailable on Linux.
New files and edits are still local/uncommitted in this checkout. Preserve them.
Private evidence is in `../evidence/system-actions-next/`; recent live/native/audio
reports are in `target/system-actions-next/` and `target/codex-plugin-linux/`.

## Previous handoff, animated-image phase

The user asked to finish the animated-image work and stop the active goal because
this chat was too long. Do not resume automatic goal work without a new request.
The full Stream Deck desktop port is unfinished. The last rough engineering
estimate was 60–70% overall, with substantial uncertainty around native plugins.

## Project and user constraints

- The repository root contains the app in `streamdeck-linux`; run app commands there.
- Public repository: https://github.com/JackSkellet/deckforge
- Local branch: `codex/streamdeck-linux`; publish to verified origin `main`.
  Inspect/fetch current history before continuing; preserve user changes.
- Native Rust host and Wry/WebKit desktop shell, with an embedded TypeScript UI.
  Dark theme. Build independently; no existing Linux Stream Deck implementation
  as a starting point. No Wine. Rust ports for platform-only plugin backends.
- Use the installed `i-have-adhd` skill. Prefer simple solutions without losing
  functionality. Do not spawn agents unless newly authorized.
- GitHub CI stays manual-only. The user handles long reliability tests.
- Studio LED is skipped. Effects and screensaver completion are deferred.
  Sharing and Smart Profiles go last. Larger plugin and built-in action gaps
  are the next priority. Audio routing was physically accepted by the user.

## Latest delivered work

Folder auto-exit defaults/overrides, a countdown ring, confirmed override reset
and Back-key long-press to the root page are implemented. Key ownership guards
prevent stray releases from navigating. Native/process checks cover Counter,
export/restart, held input and adapted Back through an Action Trigger.
See `FOLDER_EXIT.md` for contracts and limits.

Animated GIF key artwork now works from user images, imported profiles, manifest
images and SDK file/data-URL writes. Animated `setImage` is explicitly requested
as a Deckforge extension even though the official SDK excludes animation.
Encoder `setImage` animates the editor icon separately from LCD feedback.
Targets, user precedence, state changes, original bytes and restart are covered.
See `ANIMATED_IMAGES.md` for timing, budgets and remaining boundaries.

The installed release SHA-256 is `173e85ee2fc937884f2242f430bac91fc2fd4a3dfabf9d4ce946485402301cd4`.
Installation preserves saved state, Discord credentials and the running app.
The user can reopen Deckforge when convenient to load it.

## Evidence and verification

Private evidence is under `../evidence/key-animation/`:
- `sdk-final.json`: unchanged SDK 2.0.4, animated files/data URLs, state/target
  isolation, encoder icons, original-byte export/import and restart.
- `native-final/report.json`: actual WebKit pixels change, Counter works while
  the image animates, and restart works after deleting the original image.
- `static-regression.json`: static PNG/JPEG/WebP and dial behavior still pass.
- `rea-bundle.json` and `rea-snapshot.json`: native decoder timing/loop evidence.

Additional targeted Rust checks cover timing, finite loops, GIF disposal,
frame-count rejection, hardware/editor frames, user priority and page cleanup.
Appearance, display-target, Wheel, Trigger and update-asset checks passed.
Strict all-target Clippy passed. These are simulated-device/native-window checks;
no USB acceptance was newly claimed. REA was exported and closed.

## Remaining scope

Read `FEATURE_PARITY.md`, but distinguish current rows from its historical
baseline sections. Some old OS/account/audio baseline wording is stale; Discord
and Marketplace auth and the native mixer have since been implemented.

1. Broader real plugin coverage and native ports for Windows/macOS-only backends.
2. Text entry, Hotkey Switch, application closing/activation, Key Logic and
   remaining dial/display behavior. Animated WebP/APNG and feedback pixmaps are
   still open, as is exact Windows image/state-reentry parity.
3. Full backup/restore, migrations, undo, system clipboard, icon packs and file
   associations. Windows acceptance of exported profiles still needs evidence.
4. Remaining transports, Mobile/virtual/MCP support, Network Dock, firmware
   updating and physical device checks beyond the tested Plus.
5. User-deferred sharing, Smart Profiles, effects and screensaver completion.

## Operational safeguards

Do not print or publish credentials. User state is in
`~/.local/share/deckforge/session.json`; Discord private auth is under
`~/.local/share/deckforge/plugin-data/`. Public Discord packaging uses an
allowlist and excludes connection tokens. Do not rerun live auth casually.

For an app update, build frontend before Rust because the frontend is embedded.
Use the existing targeted checks and reuse valid results. Install atomically with
`scripts/install-desktop.py --without-bundled-plugins`, compare state/auth hashes
and modes before/after, verify the installed binary hash and leave the running
app alone. `~/.local/bin/deckforge.previous` is the executable rollback.
