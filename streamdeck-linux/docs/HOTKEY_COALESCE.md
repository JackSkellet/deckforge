# Paced dial hotkeys

Dial Hotkey turns and touch taps now run through a per-action queue. The editor's
**Limit queued dial shortcuts** checkbox writes the original `Coalesce` field.
Enabled allows three entries including the active stroke; disabled allows five.
An absent field follows the original true default. Existing explicit false values
are preserved. Reversing direction or switching to a tap discards unstarted
entries from the previous gesture. The active key always receives its release.

Plain shortcuts have a 30 ms press and 30 ms release gap. Shortcuts with modifiers
use 50 ms for each. Dial button presses and keypad shortcuts keep their existing
held-until-release behavior. Queues are independent across actions and devices.
Leaving a page, editing settings, disconnecting the device or shutting down
cancels the applicable queue and releases its owned keys. Shared modifiers stay
held until their remaining owners release them.

## Reference evidence

The Windows 7.6.0.23012 executable from the supplied MSI has SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
REA/Ghidra evidence is saved in `../evidence/hotkey-coalesce/` relative to the
repository root. The vendor executable was analyzed statically, not executed.

- `0x1402bc3b0` assigns a queue limit of 3 or 5 from the saved Coalesce flag.
  `0x1402bb4e0` from the preceding investigation supplies its true default.
- `0x1402bc960` submits each signed dial tick under gesture 0 or 1;
  `0x1402bc7a0` submits touch taps under gesture 2.
- `0x1402e00b0` removes differing unstarted tail entries, applies the cap and
  starts processing immediately when idle.
- `0x1402e03d0` runs press, wait, release, wait, remove. `0x14059da60` returns
  30 ms without modifiers and 50 ms when `KeyModifiers & 0x1fff` is nonzero.
- `0x1402e02d0` clears the queue and releases an active stroke.

The new evidence records are
`ev_0cc9252bf36a46c199db057bdf771c3d290a3fa63c839bdb7c7159e7a95f64ea`
and `ev_28b3891c663b0b5527cc773dad24cd82281256679c69b0eb965c675c91542405`.

## Verification and limits

Rust tests cover native queue caps, gesture reversal, separate press/release
intervals, delayed polling, cancellation, modifier ownership, signed tick bounds,
and the real worker's cancellation/shutdown path. Keyboard events are captured
into owned files, without desktop input. Frontend tests preserve imported fields.
`tests/native_hotkey_coalesce_interop.py` exercises the real WebKit checkbox,
Save, unchanged shortcuts/unknown settings and a second process restoring them.

These checks do not establish physical dial acceptance or how each focused
application handles injected shortcuts. Linux event-loop scheduling can lengthen
intervals; missed deadlines never compress a press or release gap. Keypad auto-repeat and logical active-layout mapping remain separate work.
Left/right modifiers and focused US/German/Norwegian checks are covered in
[Hotkey modifiers](HOTKEY_MODIFIERS.md).
