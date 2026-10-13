# Folder auto-exit

Select the Back from folder key and open **Folder auto-exit** to enable the
folder default. Other actions can follow that default, disable auto-exit, or
choose their own whole-second delay from 0 to 60. Save applies the draft.

A key release starts the delay. Another press cancels it; releasing that action
can start a new delay. Zero returns immediately after dispatching the action's
release. Nested folders return to their immediate parent on a short Back press. Holding
Back for more than 500 ms returns to the root of the current folder path on
release. This also works when Back is assigned through an Action Trigger or
Action Wheel. Navigation actions
are excluded, so opening another folder does not start the old folder's timer.
Timers belong to the device that received the input. Selecting another device in
the editor does not move a running countdown. Leaving the page, removing its Back action, disconnecting the device
or editing auto-exit settings retires the pending countdown.

The Back key shows a grey ring and a blue clockwise sweep while the timer runs.
Cancelling the timer restores its normal artwork. The ring is drawn into both
the editor preview and generated device frames without changing saved images.

The Back action also offers **Reset action overrides** with an explicit
confirmation. It removes overrides belonging to that folder while preserving the
selected Back action's default, other pages, shared pinned actions and plugin
settings. Unsaved action drafts must be saved or reverted first. Stale-folder and
read-only requests are rejected; a failed save restores the changed overrides.

## Original format and reference evidence

Elgato's [Auto Folder Exit guide](https://help.elgato.com/hc/en-us/articles/22589649057933-Elgato-Stream-Deck-Auto-Folder-Exit)
describes the folder default, per-action overrides and 0–60 second range.

The shipped Stream Deck 7.6.0.23012 Galleon profile contains a Parent Folder
record with `"AutoExit":{"Delay":10,"Enabled":true}`. This field sits on the
action record, outside plugin `Settings`. Its archive SHA-256 and exact member
are retained privately in `evidence/folder-exit/original-records.json`.
Galleon remains an unsupported device model; finding its record does not prove
that the entire profile or that hardware works.

REA/Ghidra inspection of the original executable
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`
establishes:

- `0x1403da2e0` serializes `Enabled` and `Delay`; `0x1403dd420` reads the record.
- `0x1402e2ce0` uses absence of the optional action record for inheritance from
  the Back action. Its unconfigured UI default is disabled with a one-second delay.
- `0x1402e26e0` limits the editor to 0–60 seconds.
- `0x140389ec0` cancels the animation on press and starts the exit check on release.
- `0x14038a120` excludes folder/page navigation, selects action override or folder
  default, and multiplies seconds by 1000 for the timer.
- `0x14038a3a0` stops the active timer; `0x14038a600` resolves the parent on completion.
- `0x140388c00` requests countdown artwork from the active integer 0–100 animation.
  `0x14079c390` draws a 75% diameter circle, a 6/72-width grey pen, a blue clockwise
  arc from twelve o'clock with round caps, and a black alpha-128 background. Rust
  reproduces this geometry independently. Updates are bounded to 16 ms or the next
  integer percentage, whichever is later.
- `0x1402f6110` clears the optional setting on other actions with the matching
  device/page identities. The selected Back action retains its default.

Unknown fields remain in the raw record. The editor changes only `Enabled` and
`Delay`, or removes `AutoExit` to restore inheritance. Plugin settings and live
values are not rewritten. Imported out-of-range/malformed delays are preserved
but inactive; new edits reject them instead of producing an unintended exit.

## Verification and remaining work

Rust tests cover inheritance/override, immediate and delayed exit, repeated and
unowned releases, press cancellation, navigation/disconnection cleanup, nested
return, per-device isolation and invalid-edit atomicity. Frontend tests cover
inherited controls, rapid field edits before props refresh, reset confirmation,
and saving without reverting live plugin data.

`tests/native_folder_exit_interop.py` opens the actual WebKit desktop window with
an unchanged official Counter plugin. It edits both forms, verifies Counter
execution and timer cancellation, measures the delay, exports original-format
records and checks persistence after restart. It also checks countdown restoration, reset
cancellation/confirmation and exported inheritance. It uses a simulated device and
private temporary state. No physical USB or Windows runtime acceptance is claimed.

Physical display acceptance and pixel-exact Qt raster/compositing equivalence
remain unverified. Nested child-action override semantics, timing relative to long
Multi Actions, and original dial/touch dispatch need further reference comparison.
Top-level dial/touch actions use the same device-owned deadline.

The Back long-press contract comes from `0x140389230`: strictly more than 500 ms
resolves the root page; a short release returns one level. Press ownership and
start time are retained per device and action. Repeated down packets do not reset
the clock. Page/profile/path changes, hidden actions and device disconnection
retire the gesture. A release with no matching press does not navigate, including
when the root slot now contains an Open folder action.

Tests cover the exact threshold, nested return to a non-first root page, pinned
Back actions across path changes, device isolation/disconnection and held Back
through Action Trigger. The native workflow checks short and long key presses,
duplicate release rejection and the adapted dial gesture. The original Windows
Plus whole-profile navigation/export/reimport process check also passes. Physical
long-press acceptance and the Windows execution path remain unverified.
The retained reference is `evidence/folder-exit/finish-rea-bundle.json`.
