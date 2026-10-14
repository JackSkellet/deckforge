# Editor undo and redo

The native editor has session Undo and Redo controls beside the page tabs.
Ctrl+Z undoes, Ctrl+Shift+Z or Ctrl+Y redoes. Text fields keep their native text
undo. Existing dirty action/advanced-text drafts, modal dialogs and busy edits
block graph undo; drafts belonging to deleted actions do not prevent restoring
those actions. Current canvas selection is retained. Saved undo/redo results
persist, but the history itself is cleared on full application restart.

## Recorded edits

History covers assignment/removal, settings and appearance saves, cropped and
page artwork, nested sequence edits, pin/folder operations, copy/move/paste,
profile/page create/rename/duplicate/delete/reorder, device profile copy, explicit
profile file import and Smart bindings. One native action Save groups its
settings and appearance requests. Property-inspector setSettings/setResources
writes are recorded too. Clipboard contents themselves are not rewound.

History is chronological across devices, and cross-device edits reverse together.
It holds up to50 operations and32MiB of serialized history. Old entries are evicted
with a visible note. New edits clear redo; no-op edits leave it intact. A revision
check rejects stale requests. Device identity/layout changes and device/plugin
configuration operations clear incompatible history. Normal input, plugin replies,
navigation, file export, process/audio effects, mixer/device preferences and
plugin global settings are not recorded or replayed.

## Changes, persistence and lifecycle

`src/host/history/patch.rs` tracks changed object fields and stable array identities
(context, profile/page id, native ActionID). Unrelated newer plugin data survives
undo. Ambiguous/unkeyed arrays remain atomic; conflicts fail without changing the
input graph or advancing history. Known container branches use their semantic
branch numbers in the private history representation; source/native records are
reconstructed exactly. Opaque imported container payloads remain untouched.

Reversing structural creation may remove the same identified action/profile even
if it has received normal plugin updates; its current payload is captured for
redo. Live inputs, transports, timers, audio and processes are never restored from
snapshots. Newer navigation is retained when valid; undo repairs missing active
pages/profiles and conditionally restores cursors and bundle references.

The replacement graph, navigation and Smart rules are validated and saved before
lifecycle changes. Initial-save failure restores prior graph/rules/navigation,
keeps history unchanged and emits no releases. After durable save, history advances
even if a later refresh fails; that error explicitly says the change was saved.
Affected devices refresh their projections and balance action lifecycle events.
Surviving contexts receive restored settings/resource notifications; appearance
notifications use the edited state, including non-visible states. Smart rule
changes invalidate only affected routing observations.

Refresh can materialize lazy resource maps or child records. The opposite history
operation is rebased onto that actual graph while the host mutex excludes plugin
updates; otherwise a valid redo could conflict with its own hydration. Pending
captures and history entries retain owned media/archive sources through the
existing asset-retention mechanism. No separate asset store is introduced.

## Verification and remaining acceptance

Seventeen engine/host tests cover field preservation, conflicts, generated edit
sequences, stable identities, grouped saves, live plugin data, held controls on
failed saves, cross-device cut/paste, navigation/Smart rules, stale requests,
artwork retention/export after deleting originals, inspector resource events,
nested reorder with later child settings/resources, opaque imported data,
bounded history/no-op edits and non-visible-state appearance notifications.
Related transfer, appearance, crop, background, inspector and Smart tests pass.
Frontend tests cover captured device/revision, text undo and draft guards.

`tests/native_history_interop.py` uses the unchanged official Counter sample in
private simulated state. It exercises actual Undo/Redo controls and shortcut
handlers, assignment after live plugin updates, settings undo preserving the
current count, grouped title/text Save, dirty/text guards and restart. Reports
and screenshots: `target/editor-history/native/`. These fixture key events test
the native WebKit handlers, not physical keyboard or USB acceptance.

Undo is a documented Linux usability requirement in UI-04. The supplied Windows
AppExecutable string search did not establish a general native undo contract.
One Auto Exit reset tooltip says it cannot be undone; unrelated embedded-data
matches are not evidence of editor history. Compact source evidence:
`../evidence/editor-history/string-scan.json`. Exact Windows semantics, legacy
ambiguous-record conflict behavior, long-running history load, physical-device
acceptance and fault injection after durable commit remain unverified. Full
UI-04 and overall parity are not complete.
