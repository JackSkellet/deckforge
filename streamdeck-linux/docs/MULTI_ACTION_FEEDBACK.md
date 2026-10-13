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

## Parent progress and completion

The parent has its own feedback path, independent of child `showAlert`/`showOk`.
Deckforge now implements this path: completed steps reveal the success badge
clockwise from the top, and normal completion shows the full timed check mark.
Progress advances after each step's final gap, including an explicit Delay; it
measures completed steps, not elapsed time. Fractional percentages round upward.
Cancelling removes progress without creating completion feedback. Empty sequences
do not run, show success, or advance a Switch state.

**Preferences → Startup & about → Show Multi Action progress and completion
check marks** controls these automatic overlays globally. It defaults on,
persists across restart and leaves sequence execution/state changes intact when
off. It does not disable explicit plugin alerts or erase an already displayed
temporary badge. Existing temporary feedback takes precedence over progress.
The Linux setting is saved as `suppress_multi_feedback`; stale writes and failed
persistence leave the previous preference active.

The badge uses Deckforge's independent geometry and colours. Its sweep direction,
step timing, completion policy and preference polarity follow the native trace;
pixel-identical artwork and physical display acceptance remain unverified.
The original Windows 7.6.0.23012 executable is SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
REA/Ghidra static analysis establishes:

| Native location | Observation |
| --- | --- |
| `0x14011ddc0` | Initializes the global preference key `sw_feedback_suppress_ok_ma`. |
| `0x1402c89f0` | Connects the routine manager's started, finished and progress signals to callbacks `0x1402ca590`, `0x1402ca620` and `0x1402ca720`. |
| `0x1402ca590` | Validates the action address, sets execution state 2, and starts progress at zero when the suppression preference is false. The preference getter's fallback is false. |
| `0x1402ca720` | Validates the address and, when unsuppressed, stores `ceil(current*100/total)` and emits image-change signal 2. Current is the number of completed children; total is the child list length, as established by `0x14024cb90`. |
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

The follow-up trace resolves the remaining progress path:

- `0x14024cb90` advances the child cursor in phase 2, after the up/gap timer,
  and emits completed-child count and list length. This includes explicit Delay.
- `0x1402c9170` draws a transparent 72-pixel base image (scaled by pixel ratio),
  clips with an arc from 90 degrees through `-percent*360/100`, and draws the
  success artwork into that sector. This is a clockwise sweep from the top.
- `0x14079b540` selects the normal/high-resolution `ok_2_green_symbol` resource.
  Deckforge reuses its own success badge instead of distributing that artwork.

REA Evidence: `ev_c3a2a8fdf367dae9572cc3a5dae379772888b13b72885c69d5280909fd1c6bb4`
(timer), `ev_e890026faa9d930e8af33316fa57f8d4726029d1ba8a09aabec55a48a18429b1`
(clip renderer), and `ev_11e91243a9799bd6a4eb8d0760d7f40b7b6d8ce445f661f33ba8234a188d78e9`
(asset helper). The follow-up bundle is in `../../evidence/multi-completion/`.

Scheduler tests check gap/delay boundaries and upward rounding. Renderer tests
check clockwise clipping and unchanged pixels outside the sector. Host tests
cover generated hardware frames, cancellation restoration, all three container
variants, suppressed feedback, empty Switch branches and preference persistence.
`tests/native_multi_completion_interop.py` verifies the visible native key,
completion expiry, the actual preference checkbox, suppressed Switch execution
and restart. It uses an isolated simulated device and no external actions.

The full `tests/multi_interop.py` workflow passes with the original profile
imported on its actual XL model (`20GAT9902`). Its old Plus-host import assertion
also failed on the previous installed build: the importer correctly retained
that foreign-model profile without activating it. No imported hotkeys execute
in this acceptance test.

Child-to-parent alert forwarding remains a separate unresolved path.
