# Drag actions into sequences

Drag an action from the right-hand library onto **Drag actions here** in a Multi
Action. Drop on the upper or lower half of a step to insert before or after it.
Use the dotted handle to drag an existing step into a new position. **Save changes**
publishes the edited sequence; dropping alone does not run or persist its actions.

The same list interaction applies to Multi Action Switch's selected branch,
Random Action, Action Wheel and Dial Stack, with their existing compatibility
rules. Fixed Action Trigger and Key Logic slots keep replacement behavior. Nested
editors consume their own drops; an invalid drop on a gesture editor does not
append into its parent. Cross-container step moves use the existing Copy/Cut/Paste
controls; drag reorder is confined to the same unchanged sequence and branch.

Insertion feedback, an empty drop area, an end target and inspector edge scrolling
make destinations visible. The dropdown/Add and Move up/down buttons remain
available for keyboard operation. Read-only or busy editors reject drag edits.
Stale reorders are rejected rather than moving an action that changed during drag.
Existing action identities, settings, assets and unknown record fields are retained
when reordering; library drops create new independent children.

Verification: four focused drag tests plus existing Sequence and command-result
suites cover append/insertion/reorder, incompatibility, stale data, disabled
editors, switch branches and nested drop isolation. Native WebKit testing exercises
the real library dragstart payload, DataTransfer/DragEvent handlers, explicit Save,
exact child-record preservation and restart. It does not execute actions or open
USB. These are synthetic native drag events, not a recorded physical pointer
session. Reports and empty/populated screenshots: target/sequence-drag/native/.
