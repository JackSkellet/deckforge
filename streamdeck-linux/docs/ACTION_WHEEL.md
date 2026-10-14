# Action Wheel contract and implementation plan

Action Wheel is connected to the Rust host and native editor. Add **Action Wheel**
to a dial, then drop key actions onto its editor or use the action selector.
**Show** selects a saved child for preview; **Wheel options** contains reverse
rotation and the selected-title overlay. Child plugin inspectors remain available.
Key Logic is a separate action and has not been implemented by this work.

Rotation uses the recovered paced selection model. Dial presses send ordinary
keyDown/keyUp; centre taps use owned 100 ms pulses. Every wheel child stays live,
including those not among the currently drawn cards. The wheel can also be used
inside a Dial Stack. User artwork on its software dial icon retains precedence.

Option saves preserve live selection and child settings. Flat child edits merge
by ActionID/UUID against their captured baseline, so a plugin count update is not
replaced merely because a row moved or another setting changed. Native editor
raw-record edits also merge against the latest draft if a snapshot arrives before
the click callback runs. Structural/action-setting edits release held inputs;
changing only wheel options preserves an owned press. If selection changes during
a press, Linux releases the original child, preventing a stuck key. This ownership
rule is not a claim that every overlapping Windows gesture follows the same path.

Elgato's [Action Wheel guide](https://help.elgato.com/hc/en-us/articles/28742375481997-Elgato-Stream-Deck-Action-Wheel)
and [usage overview](https://www.elgato.com/us/en/explorer/products/stream-deck/what-is-an-action-wheel-for-stream-deck/)
establish the interaction: rotate or tap a side card to select a key action,
then press the dial or tap the selected card to activate it. Dial Stack instead
groups encoder actions. No existing Linux implementation was used.

## Source identity and profile format

Static analysis targets Windows Stream Deck 7.6.0.23012, executable SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
The original application was not executed.

The shipped Plus XL profile archive has SHA-256
`e4ce1ac8947e28266b852a0860c03c9437b7ed2a2e2bd5044e1b94a502a72675`.
Its page `6B53D0C1-DD39-4FBA-A38E-87021FEDEC39` contains an Encoder action at
`1,0` named Action Wheel, UUID `com.elgato.streamdeck.keys.stack`. It has six
flat Clocks children in `Actions`, with independent ActionIDs and settings.
The parent settings are `CurrentIdx: 1`, `ControlOffset: 0`, `ExtTitles: false`
and `InvertRotation: false`. The model retains the full source object and each
child record, including unknown fields and null settings.

Factory `0x140409a50` constructs `KA_KeyStack` through `0x1402c3e00`; its vtable
is `0x141662a10`. Settings loader `0x1402c4640` reads CurrentIdx with default zero
and clamps to the last child, not to the first. Native numeric conversion
`0x1401e1570` preserves signed integer bits in the unsigned result and truncates
floating numbers, so a negative integer clamps to the last child. Boolean
settings default false. Negative ControlOffset is ignored. The Linux model
bounds oversized offsets and child counts to avoid unbounded work; its child
limit is 512. Nested records remain raw until the host applies its ownership,
controller compatibility and recursion rules.

## Navigation and activation

| Path | Recovered behavior |
| --- | --- |
| Rotate, `0x1402c50b0` | Ignore new turns while pressed. With two or more children, optionally invert ticks and clamp to -5…5. Replace the pending amount, rather than adding to it. Start an inactive timer at zero delay. |
| Rotation timer, `0x1402c6e10` | Consume `ceil(abs(pending)/1.5)` steps per callback. Wrap selected index modulo child count. Refresh neighbor images and settings, then rearm at 61 ms, even when this callback consumed the last pending step. The following empty callback stops. |
| Touch, `0x1402c4c30` | Left/right card tags call the same rotation handler with -1/+1 and pressed=false, so the inversion setting also affects that route. Centre tags queue one key activation on the selected child. Other tags do nothing. |
| Dial button, `0x1402c4ef0` | Dispatch ordinary keyDown/keyUp to the selected key child. A successful release may advance the child's automatic state. A rejected down requests child warning feedback. |
| Tap helper configuration, `0x1402c3e00` | The reusable key pulse helper gets 100 ms hold and gap intervals and a pending limit of one. This differs from Action Trigger's 60/15 ms configuration. |
| Press visual, `0x1402c6c20` | Front-card scale is 0.8 on down and 1.0 on up. Touch requests the named `press` animation. |

Ignoring a new held turn does not cancel a rotation batch queued before the
press. The shared key-adapter ownership tracks physical and pulse inputs independently.
Page changes and structural replacements cancel obsolete owners; disconnect
releases physical ownership and stops queued rotation. Physical USB acceptance
remains unverified.

## Rendering and indicators

The private Qt resource `:/layouts/layout-y1.json`, SHA-256
`0967c1cd94b7aeb56c3f38ef34a21684470acfe8d77494f1c03079624252fb8a`,
defines a 200 × 100 `$Y1` layout. The Rust model uses these observed coordinates,
without distributing vendor graphics.

| Child count | Back cards | Front card |
| --- | --- | --- |
| 1 | None | `[64,6,72,72]` |
| 2, selected 0 | Next at `[102,14,56,56]` | `[42,6,72,72]` |
| 2, selected 1 | Previous at `[42,14,56,56]` | `[86,6,72,72]` |
| 3+ | Previous at `[20,14,56,56]`, next at `[124,14,56,56]` | `[64,6,72,72]` |

Side cards have opacity 0.6 and radius 9; the front has radius 12 and paints
above them. The model hit-tests front-first with half-open rectangle edges;
exact native rounded-corner/edge inclusion is still unverified. Native side
backdrops also have blur radius 16, and a separate animation layer sits above
the cards. Carousel frame notifications run every 40 ms. The Rust renderer now uses the
recovered 160 ms two-card and 320 ms multi-card transitions, including the native
cubic Bézier, changing depth, opacity, size and rounded occlusion. See the
[carousel implementation and evidence](WHEEL_ANIMATION.md). The separate touch
press-animation curve still needs implementation.

`ExtTitles` configures the child renderer with additional flags `0x600`
(`0x4b` becomes `0x64b`) and enables a separate selected-title item over
`[16,10,168,72]`, using the child's font size, title alignment and colour at
weight 600. The layered title fallback branches have been retained as evidence;
the implementation uses the selected child title and style, not the parent action name.

`ControlOffset` belongs to the position-indicator viewport. Its model has a
nine-item capacity, current index, offset and up to two small markers on each
side. `0x140376450` initializes marker counts and `0x1403764e0` adjusts the
viewport to keep the selected item visible. The indicator strip is centred
inside `[16,84,168,10]`: inactive dots are 8 × 8, the active white pill is
12 × 8, and collapsed-edge markers are 6 × 6 and 4 × 4, separated by 4 pixels.
It is internal viewport state, not a user-facing position or audio control.

`0x1402c53a0` creates an ESDGenericActionsSet with controller kind 1 (Keypad),
instantiates the child actions and binds their parent address. Activation and
deactivation loops visit the full child collection (`0x1402c4350` and
`0x1402c42f0`). A wheel therefore needs live child contexts beyond the three
currently drawn cards. Imported unavailable children must retain their records.
Exact editor restrictions on nested action types remain a separate question.

## Evidence and verification

The private fixture, layout extraction, REA bundle and source-bound snapshot are
under `../../evidence/action-wheel/`. Native decompilation and original binaries
are not included in the public repository.

Selected REA Evidence IDs:

- Settings: `ev_bfea891604b3d0851d3c8937b3373f6b873dd21e30328c438a35f06afa5349b7`
- Rotation handler: `ev_f9086798e906e70ac4fe8a14d1f1e712c908a3805eb921dbb2921d3900f1cb77`
- Rotation timer: `ev_4dd285ed5861f2d475eea8876acbdcbf91e02c4b4a15a6ec01143b959d8a330b`
- Touch routing: `ev_71f50b5410bf9dfdfc64a1e84e9ecc198144a0d94a65486aab4ecb8de3f5371d`
- Button routing: `ev_09dc32db7cddeeea24e82dd0b9c5cd8d7532088790bec50b141ccfaf219ec8e8`
- Visible cards/title properties: `ev_f0302edb8e2c4ca44e8fc593dd9192fa270ba2ec6eb7e1f1b28b41fe19fa9d01`
- Indicators: `ev_9458710e23f25ddad894136225e4696f1979ed201e1b56423cecedd613ae3aa9`
- Child model: `ev_1f7a497c326e0370a6a4f98ae0d94448fbe4cd40d9b09859636c00f7cd5a2463`

`cargo test --test action_wheel_model` checks saved-field preservation, selection
normalization, capped/replaced rotation batches, timer rearming, two-item hit
priority and indicator visibility across all 512 supported child counts.
The explicitly gated original-profile test also passes with `--include-ignored`:
all six Clocks records survive parsing unchanged.

Host tests check empty/two-card rendering, selected-card layering, owned releases,
option updates during a press, per-device state and merging live child settings.
`tests/action_wheel_interop.py` verifies six live key contexts, capped selection,
inversion, 100 ms tap timing, page-hide release, nested Dial Stack pulse isolation,
unchanged Counter execution, restart and generated/original profile round trips.
`tests/native_action_wheel_interop.py` exercises actual WebKit card decoding,
library drag/drop, Show, option saves and Counter updates across restart. A
controlled repeated run checks the snapshot/edit race found by that fixture.
All of these use simulated devices and temporary state; no physical acceptance
is inferred.

The existing Dial Stack workflow passes. The complete Action Trigger script now
passes after adding the distinct Galleon profile model and testing each original
fixture on its declared device.

Remaining work includes native backdrop-blur and touch press-animation
fidelity, the full native title-fallback matrix, rounded hit-test edge comparison,
manual hardware acceptance, and broader nested-action/editor parity. The current
renderer shows the recovered carousel transitions, static layouts and physical
pressed-card scale; it does not claim pixel-identical Windows rendering. Existing configured-slot drag/drop
into the wheel and child appearance editing need further native UX coverage.

Verification update, 14 October 2026: the Galleon profile/model gap is resolved.
The complete Action Trigger and soundboard suites now pass on their fixtures'
declared device models. See [Galleon support and remaining USB work](GALLEON.md).
