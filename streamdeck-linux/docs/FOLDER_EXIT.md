# Folder auto-exit

Select the Back from folder key and open **Folder auto-exit** to enable the
folder default. Other actions can follow that default, disable auto-exit, or
choose their own whole-second delay from 0 to 60. Save applies the draft.

A key release starts the delay. Another press cancels it; releasing that action
can start a new delay. Zero returns immediately after dispatching the action's
release. Nested folders return to their immediate parent. Navigation actions
are excluded, so opening another folder does not start the old folder's timer.
Timers belong to the device that received the input, not the device selected in
the editor. Leaving the page, removing its Back action, disconnecting the device
or editing auto-exit settings retires the pending countdown.

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

Unknown fields remain in the raw record. The editor changes only `Enabled` and
`Delay`, or removes `AutoExit` to restore inheritance. Plugin settings and live
values are not rewritten. Imported out-of-range/malformed delays are preserved
but inactive; new edits reject them instead of producing an unintended exit.

## Verification and remaining work

Rust tests cover inheritance/override, immediate and delayed exit, repeated and
unowned releases, press cancellation, navigation/disconnection cleanup, nested
return, per-device isolation and invalid-edit atomicity. Frontend tests cover
inherited controls, draft changes and saving without reverting live plugin data.

`tests/native_folder_exit_interop.py` opens the actual WebKit desktop window with
an unchanged official Counter plugin. It edits both forms, verifies Counter
execution and timer cancellation, measures the delay, exports original-format
records and checks persistence after restart. It uses a simulated device and
private temporary state. No physical USB or Windows runtime acceptance is claimed.

The original Back-key countdown artwork and its reset-all-overrides button are
not implemented yet. Nested child-action override semantics, timing relative to
long Multi Actions, and original dial/touch dispatch need further reference
comparison. Top-level dial/touch actions use the same device-owned deadline.
