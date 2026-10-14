# Nested action clipboard

Select a saved child in a container and choose Copy selected or Cut selected.
Cut retains the source until a paste commits; Escape cancels it. Paste at end or
Paste after selected inserts into Multi Action, Switch/Random, Dial Stack or
Action Wheel lists. Trigger/Key Logic uses Paste into selected to replace one
fixed position. Copied children can also be pasted onto the device canvas or
into another compatible container. Settings, unknown action fields, resource
bindings, artwork and referenced folder pages travel with the copied action.

Copy/Cut/Paste inside the sequence handles Ctrl/Cmd C/X/V when focus is on its controls;
text fields keep their normal editing shortcuts. Delete removes the selected
child in the existing unsaved draft rather than removing the device control.
Copy/Paste requires saving or reverting the container's pending edits. Clipboard
operations themselves publish immediately through the existing transaction.
The clipboard stays available when switching profiles/devices.

## Reference evidence

Read-only analysis of the supplied Windows7.6 AppExecutable,
SHA-256 `2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`:

- ESDActionClipboard vtable `0x140ebcec0` has12 entries. The implementation stores
  a shared stash at object+8/+0x10 and checks its valid byte at stash+0x450.
- `0x140463450` copies a selected action. Nonnegative row/column uses
  `0x1403f7780`; negative coordinates select `0x1403f71a0` for nested addresses.
- `0x140463180` pastes through `0x1403a7ad0`, passing the stash and destination.
  `0x140463580` similarly delegates placement to `0x1403a7c50`.
- `0x140463610` clears the shared stash/reference count. These methods do not
  serialize an action into QClipboard MIME data.

The traced action clipboard is in process. No portable vendor action clipboard
MIME contract has been established. System clipboard integration remains an open
Linux interoperability feature; it is not proven missing vendor functionality.
This distinction corrected the implementation priority from OS clipboard work
to nested action copying. Evidence: `../evidence/action-clipboard/rea-bundle.json`.

## Implementation

`transfer::capture` locates a child through its owning root/page and stamps source
provenance. Nested copies use the same Clipboard as canvas copies. `position`
clears parent-only context fields when a child becomes a canvas action; Delay
cannot be pasted onto the canvas.

`transfer/nested.rs` builds a replacement graph before publication. It validates
the live destination/raw baseline, branch, position, controller support, declared
Multi/Key Logic support, container parser limits and complete nesting depth. It
renews instance/action/page IDs, hydrates the destination, and retains runtime
media/resource bindings. Custom titles are normalized before hydration so raw
updates do not discard the copied artwork. Positional blanks remain positional.

`publish_transfer` is shared with canvas paste. Persistence failure restores the
old graph and leaves a cut source intact. Successful cuts from either canvas or nested controls remove the source only
after persistence and clear the clipboard. List moves insert at the chosen
position; Trigger/Key Logic positions replace that slot. A nested cut onto an
occupied canvas swaps its old occupant back into the source container only when
compatible. Ancestor swaps and moves into oneself are rejected. Lifecycle refresh
releases a running Multi child against the old tree before it disappears and
resets pending stack gestures after structural changes.

## Verified scope and gaps

Rust tests cover nested-to-canvas flags/coordinates, Switch branches, independent
identities, custom titles/artwork, resource export, nested Multi inside Trigger,
Trigger positions, Dial Stack conversion, stale/unsupported targets and failed
persistence retaining cut data. Existing transfer and child-hydration suites pass.
Frontend tests cover live child context, insertion index, keyboard scope, text
shortcuts, dirty drafts and the paste-completion selection race.

`tests/native_nested_clipboard_interop.py` drives actual WebKit buttons/keyboard
events and the real host. Multi children copy to the canvas; cropped dial actions
copy into stacks and back out. Restart and profile export preserve all three
200×100 cropped PNGs after deleting the source. No USB or user state is used.

Cut anchors include the owning root, immediate parent, branch and position.
Reordered explicit-ID sources are rejected; legacy records without ActionID also
require an unchanged raw record. Latest settings remain eligible for stable-ID
cuts. Canvas and container paste block unsaved edits in the cut source's owning
container, including when the target is on another control/device.

A parser rebinds the already-edited child order while preserving each actual
instance and its private media/resources. This avoids assigning a neighbor's
bindings to a shifted legacy row with a positional ID. Stack/Wheel selection
tracks its existing selected child through insert/remove, including moving the
selected child within the same stack. Tests cover these cases plus cross-device
moves, occupied swaps, ancestor/incompatible rejection, persistence failure and
keyUp-before-willDisappear for a held Multi child. The native test now also
verifies nested Ctrl X, source retention, repeated immediate Escape cancellation,
stack-to-stack paste and restart with all cropped images intact. Shortcut handlers
update in a layout effect so their clipboard/busy state matches the rendered
controls; a passive-effect lag previously could ignore a fast Escape. Successful
nested operations also replace stale status notices.

Read-only container editors now allow branch/child selection, inspection and
Copy. Their edit fields, Add/Remove/reorder, plugin settings, Cut and Paste remain
disabled. Keyboard mutations and action drops are also blocked; host read-only
checks remain authoritative. Editable fieldsets are scoped around mutation
controls, leaving recursive child selection and Copy outside them.

Legacy child records without ActionID resolve their runtime context by position
and UUID, rather than matching the first same-UUID sibling. Explicit ActionIDs
still resolve by identity after draft reorder. Tests exercise the second legacy
child inside a read-only Multi Action Switch nested in Key Logic, copy it into an
editable profile, compare unchanged source data and verify restart. Native report:
`target/readonly-children/native/native.json`.

System clipboard interoperability, file-manager drops, general undo and official-app runtime
acceptance remain open. No full UI-04 completion is claimed.
