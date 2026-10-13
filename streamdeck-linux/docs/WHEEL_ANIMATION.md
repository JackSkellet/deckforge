# Action Wheel carousel animation

The wheel renderer now animates its cards independently of input selection.
The selected child changes when the navigation step is accepted; drawing an
intermediate frame never sends key events or changes that selection.

## Recovered native behavior

The reference is Stream Deck 7.6.0.23012, executable SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
REA/Ghidra static analysis and bounded disassembly establish these contracts:

| Native routine | Observation |
| --- | --- |
| `0x1402dcfc0` | Two-card parallel property animations last 160 ms. They exchange front/back size, position, opacity and depth. |
| `0x1402dd690` | Larger wheels use 320 ms transitions with four moving cards drawn from the new selection's neighboring images. |
| `0x1402dc810` | Rectangle, opacity and depth use the same cubic Bézier: `(0,0), (.3,.8), (.3,.8), (1,1)`. |
| `0x1402ddfa0` | Carousel repaint notifications use a 40 ms timer. |
| `0x1402de290` | Copies child images into the animator. Two-card motion uses the previous selection; larger motion uses the new selection and direction. |
| `0x1402dd440` | Two-card animation flips from the previous selection even if a multi-step batch wrapped back to that same logical index. Settled rendering still uses the actual selected index. |
| `0x1402ddc50` | Stops an interrupted larger-wheel animation at its endpoint before the next transition starts. |
| `0x1402dcc80` | Stable depth ordering and rounded subtraction masks prevent lower artwork from bleeding through translucent higher cards. Bounds align outward to integer pixels. |
| `0x1402dc420` | Card borders are `#262626`, with a one-pixel image inset; animated corner radius is aligned width divided by six. |

The positive-direction transition moves cards through these rectangle anchors:
`[10,20,44,44]`, `[20,14,56,56]`, `[64,6,72,72]`,
`[124,14,56,56]`, `[146,20,44,44]`. Incoming/outgoing cards fade at the ends.
Reverse motion uses its own opposite templates. For larger batches, the original
animator uses the new target's neighbors rather than interpolating every skipped
logical action. The Rust model retains that distinction.

The direction flag was independently checked in the timer's instructions:
`0x1402c6e58` derives it from the pending count's sign; `0x1402c6f10` passes it
to the animation dispatcher. The complete private records are retained in
`../../evidence/wheel-animation/rea-bundle.json` and the accompanying snapshot.
No original executable, graphics or decompiled implementation is distributed.

## Rust rendering and ownership

`src/wheel_animation.rs` owns timing, easing and rectangle/opacity/depth
interpolation. `src/host/wheel.rs` owns transient frame snapshots and composition.
At most five small key images are captured for each display plane. A transition
ends at its deadline and restores the current live artwork, rather than writing
animated frames into profile state.

Hardware and editor snapshots are separate. If both targets initially share an
image, both retain that initial snapshot even if an SDK command later splits
them. If they initially differ, the host continues rendering separate planes
until the transition ends, even if the current action state becomes shared.
This prevents target-specific artwork from crossing surfaces during animation.

Page changes, structural replacement, explicit Show selection and device
shutdown discard obsolete motion. Changing title-overlay mode also discards an
incompatible visual snapshot. Animation timers exist only while motion is active;
the normal input and profile-persistence paths remain independent.

## Verification and remaining boundaries

- Model tests check durations, the Bézier time inversion, forward/reverse
  templates, larger rotation batches, draw order and deadline termination.
- Renderer tests inspect intermediate device frames, rounded overlap clipping,
  page cleanup and software/hardware isolation when targets change mid-motion.
- The native WebKit workflow records multiple distinct strip images during a turn,
  checks that they settle, and still exercises Counter, drag/drop, settings and
  restart. It uses an isolated simulated device and opens no USB transport.
- Existing display-target and wheel/profile process checks cover affected
  integration paths. This is not live Windows or physical-device acceptance.

Raster resampling, font rendering and rounded hit-test edges are not claimed
pixel-identical. Touch hit testing still uses the recovered settled card regions;
reference hit behavior during motion needs further verification. Native backdrop
blur is separate from card interpolation.

The separate named touch `press` animation has also been located at
`0x1405b8a90`: defaults are duration 0.2 seconds, peak scale 0.8 and update interval
20 ms, with a half-duration first timeline and completion callbacks. That return
phase still needs tracing and implementation. Physical down/up retains the
already implemented immediate 0.8/1.0 scale.

Selected Evidence IDs:

- Two-card setup: `ev_7117d4b4daceed236e918ecff1bd412be15dc4f76278f68ce0c093cd1316b996`
- Larger setup: `ev_5091c2ba5f673e9884586db88f35dbb4d6f1021d1bc7352a11474004f77ef257`
- Property/easing factory: `ev_b6e5828392d157ede37c230202d7c4acba6e8e1a1a6c767776622cc83b93dc12`
- Occluding composition: `ev_23327f5bcbee0e15b348722a3bf761397e7f7050deba5bd56f17e5a8786f06e3`
- Card painting: `ev_3d38b270c24b36054b1b8a31877b1e419887cd60d698ba62f8548de8452f01ff`
- Touch-press factory: `ev_770d216508272bdbc32a94c6fdbcbe1824b4d93d74c31f7a60f725ffa14a0fe1`
