# Multi Action feedback

## Child images in the native editor

Each child action now has an independently rendered preview in the sequence
editor. Its live title, image, state and temporary warning/success badge belong
to that child. Preview lookup follows the child ActionID and action UUID through
draft reordering, and does not reuse artwork when a draft replaces the action.
Saving a sequence renders the new child images immediately, even when a plugin
has not sent an initial display update.

Rendering these editor previews does not forward child feedback to the physical
parent key. The original shared child model already has a temporary image layer:
Windows 7.6 callback `0x1402493e0` substitutes 3000 ms for a negative duration,
updates the child's renderer and notifies model flag `0x10`. Forwarding a child
alert to the parent remains unproven and is not introduced by this change.

Verification:

- `host::feedback::tests::multi_child_feedback_renders_in_editor_without_changing_parent_frames`
  checks immediate previews, child-only alerts, sibling isolation, expiry and
  unchanged generated parent hardware frames.
- `frontend/src/Sequence.test.tsx` checks stable thumbnail ownership when draft
  rows move or change action identity.
- `tests/native_sequence_feedback_interop.py` opens the native application with
  a generated plugin and simulated device. It inspects decoded DOM pixels for
  warning/success, current-title restoration, and unchanged parent/sibling images.

No physical device is opened by these tests. Native transient effects were not
manually compared against a running Windows application.

## Recovered parent completion behavior, awaiting implementation

The parent has its own feedback path, independent of child `showAlert`/`showOk`.
The original Windows 7.6.0.23012 executable is SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
REA/Ghidra static analysis establishes:

| Native location | Observation |
| --- | --- |
| `0x14011ddc0` | Initializes the global preference key `sw_feedback_suppress_ok_ma`. |
| `0x1402c89f0` | Connects the routine manager's started, finished and progress signals to callbacks `0x1402ca590`, `0x1402ca620` and `0x1402ca720`. |
| `0x1402ca590` | Validates the action address, sets execution state 2, and starts progress at zero when the suppression preference is false. The preference getter's fallback is false. |
| `0x1402ca720` | Validates the address and, when unsuppressed, stores `ceil(current*100/total)` and emits image-change signal 2. The exact meaning of current/total still needs the producer traced. |
| `0x1402ca620` | Clears progress to -1. Normal completion reason 0 calls the existing success-image helper `0x1403bc0f0(parent, false)` when unsuppressed. Cancellation does not request a success image. |
| `0x1402a17d0` | Initializes the preference checkbox to the inverse of that suppression flag. |

The manager completion callback `0x1401b1420` invokes the Switch state-increment
callback only for completion reason zero, then emits the routine-finished signal.
This continues to support the existing Switch cancellation behavior. Completion
here concerns the dispatch sequence; it does not establish success of every
asynchronous operation performed by a plugin.

Relevant retained Evidence IDs:

- Preference key: `ev_19c24bb36e01722a867be35ad573fc555a24805110e154f71adeb8cec574bd08`
- Signal connections: `ev_ed7cf57e47c7b84c2306e825d27b55ae28f764af312bc711730f26426d8edcbc`
- Parent completion: `ev_9eca0468b4d72566c537d010fb5be568c6c12db883193fe9155942e4f897a836`
- Parent progress: `ev_b6d94fee24727d12c5cf95b0af283e8d4c0b5e4abe26c79d79e49bb7dd5c1d33`
- Checkbox: `ev_854ebb309d3cf46917055ab1e799ab3ed3dccdc61ab89c59b850c80e18b7829e`

The private Evidence bundle and snapshot are retained in
`../../evidence/multi-child-feedback/`. Native pseudocode and vendor binaries
are not distributed. This is static evidence, not an observed Windows run.

Next: trace the progress producer and parent overlay renderer, then implement
parent progress/completion with its global preference, cancellation and Switch
coverage. Child-to-parent alert forwarding remains a separate unresolved path.
