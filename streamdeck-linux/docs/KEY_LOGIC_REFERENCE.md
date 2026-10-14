# Key Logic reference for the next implementation

Status: native behavior recovered and Linux implementation added; see `KEY_LOGIC.md`.
UUID: `com.elgato.streamdeck.keys.logic`.
The original Windows executable SHA-256 is
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
REA/Ghidra records are in `../evidence/open-application/rea-bundle.json` relative
to the repository root, with the matching snapshot. The session was exported and
closed after the Open Application pass.

- Constructor `0x1402c2700` creates a 300 ms single-shot timer at offset 0x8c8 and
  a 500 ms single-shot timer at 0x8d0. Its pulse queue uses a cap of three, a 60 ms
  press and a 15 ms gap, matching the existing Action Trigger repeater.
- `0x1402c31f0` creates exactly three positional child slots from its serialized
  action vector; `0x1402c39d0` retrieves slots 0, 1 or 2. A shipped profile fixture
  for this action has not yet been located. Confirm serialization before import
  support rather than claiming an observed profile round trip.
- Key-down `0x1402c3100` marks held and invokes `0x1402c2f60` with event 0. When
  both timers are idle, both start. A second press while both timers run stops
  both and dispatches child 1 immediately as a held action.
- Callback `0x1402c3c30` dispatches child 0 only when the 300 ms timer fires while
  the physical key is released and the 500 ms timer is still active. It cancels
  both timers and sends the child through the 60/15 ms pulse queue.
- Callback `0x1402c3cc0` dispatches child 2 only when the 500 ms timer fires while
  the key remains held. It cancels both timers.
- `0x1402c2dc0` handles child selection. Missing children show an alert. Child 0
  uses the queue; children 1 and 2 receive direct key-down and become the owned
  current child. Key-up `0x1402c3120` releases that child and clears ownership.
- A first release between 300 and 500 ms appears to dispatch no child in this
  recovered path. This is static evidence, not Windows execution. Preserve the
  uncertainty explicitly and inspect related paths before claiming exact timing.

The implementation should reuse existing child lifecycle, key ownership,
resource/export and keyboard-preparation mechanisms. Add native editing for the
three positions and simulated event tests with real sample plugins. Cancel all
owned child input on replacement, departure and device loss. Do not introduce
an independent plugin runtime.
