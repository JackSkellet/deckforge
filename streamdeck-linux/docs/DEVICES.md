# Device models

Deckforge has model-specific HID transports for the documented Mini, 15-key,
XL, Plus, Plus XL and Neo families, plus original 2017, Studio, Galleon and input-only
Pedal transports. The
implementations use Elgato's published protocols and static evidence from the
original application. Only the attached Stream Deck+ has physical
acceptance evidence. Packet tests, simulated input and native editor tests do
not establish that another model works on hardware.

| Layout | Keys | Dials | USB product IDs (`0fd9` unless noted) | Evidence boundary |
| --- | --- | --- | --- | --- |
| Mini | 3 × 2 | — | `0063`, `0090`, `00b3`, `00b8` | Protocol and packet tests; physical validation pending |
| Stream Deck 2017 | 5 × 3 | — | `0060` | Native protocol evidence and codec tests; physical validation pending |
| Stream Deck 2019 / MK.2 / Scissor | 5 × 3 | — | `006d`, `0080`, `00a5`, `00b9` | Protocol and packet tests; physical validation pending |
| XL / XL 2022 | 8 × 4 | — | `006c`, `008f`, `00ba` | Protocol and packet tests; physical validation pending |
| Plus | 4 × 2 | 4 | `0084` | Existing physical display/control confirmation; new model generalization has software regression coverage |
| Plus XL | 9 × 4 | 6 | `00c6` | Protocol and packet tests; physical validation pending |
| Neo | 4 × 2 | — | `009a` | Protocol and packet tests; physical validation pending |
| Studio | 16 × 2 | 2 | `00aa` | Native USB/SDK contracts, packet and native editor tests; physical validation pending |
| Galleon 100 SD | 3 × 4 | 2 physical, 4 widgets | Corsair `1b1c:2b18`, `1b1c:2b4b` | Native USB contracts, packet/heartbeat tests and verified profile/editor/simulation; physical validation pending |
| Pedal | 3 × 1 | — | `0086` | Input/metadata protocol recovered from the original executable; physical validation pending |

The module products in the table use the matching documented family. The
original 2017 Stream Deck uses its own firmware-aware BMP transport, documented
in [Original HID](LEGACY_HID.md). Studio supports key images, encoder input and
brightness; LED/ring feedback, sleep readback and full-panel output remain
unsupported. Mobile, virtual decks and Network Dock remain separate work. The [multi-device manager](MULTI_DEVICE.md) maintains independent device runtimes
and shared plugin processes. Two simultaneous physical devices have not yet been
validated.

## Selection

`deckforge devices` enumerates supported USB devices without changing them.
Hardware discovery chooses the actual model. `--serial` selects one physical
device; `--model` can additionally restrict the family. An ambiguous selection
is rejected rather than choosing the first device.

```sh
deckforge desktop --model xl --serial DEVICE_SERIAL
# Isolated editor/profile development; never opens USB:
deckforge desktop --simulate --model plus-xl --state /tmp/deckforge-plus-xl
```

Model names are `mini`, `classic`, `classic-2017`, `xl`, `plus`, `plus-xl`, `neo`,
`studio`, `galleon` and `pedal`. The two 15-key generations can use each other's profiles while
keeping their original device metadata and distinct USB protocols.
The temporary `--legacy-editor` fallback supports Plus only; use the default
TypeScript editor for other models. Saved sessions retain their model across
restarts without repeating the flag.
Legacy sessions default to Plus. If the device is absent and its family has never
been discovered, specify `--model` for the intended layout. Reconnect stays bound
to that family so queued frames cannot reach a different model; relaunch with the
device present to discover its model automatically. Profiles retain their own
model and coordinates;
a profile for another model must not silently truncate to the active device.

## Display and input policies

- Mini uses its documented 80 × 80 BMP transport and separate feature/input
  report formats. The original Windows 7.6 Mini key writer at `0x14062e920`
  proves zero-based chunks and sets ShowImage to `01` only on the final chunk
  (`0x14062e9b0`–`0x14062e9c3`). Local static evidence is retained in
  `evidence/device-protocols/mini-direct-key.asm`; physical Mini validation is
  still pending.
- JPEG key sizes and orientation come from each model's HID page. Per-key
  rendering is resized for the target display at the transport boundary.
- Plus XL uses a 1200 × 100 strip, six 200 × 100 action areas and 36 keys.
  Its profile model `20GBX9901` comes from the shipped 7.6 profile artifact;
  the HID page currently repeats the Plus identifier. The two are not conflated.
- Neo has a separate, display-only SDK `Neo` controller. Its custom layout is
  232 × 50. Deckforge centers that logical area in the physical 248 × 58 infobar
  with an 8-pixel horizontal and 4-pixel vertical gutter. This placement is a
  Linux rendering policy inferred from the documented dimensions, not recovered
  vendor pixel placement. The two touch points navigate pages; they are not
  assignable plugin keys. Their 96 × 16 indicator displays use hardware indices 8 and 9, separate from
  the eight SDK keys. Deckforge lights an available page control in the app
  accent colour and turns it off at a boundary. The editor uses the same page
  availability, including exit paths from folders; these colours are a Linux
  policy, not a claim of vendor theme equivalence.
- Pedal uses the recovered `20GBF9901` identity and shared input/serial/firmware
  methods from the original executable. Its editor shows software action labels.
  Image, brightness and sleep operations are disabled before USB I/O. The current
  3-column, 1-row SDK layout is now confirmed through the original device-data
  service and controller-description table. The raw USB descriptor stores 1 × 3,
  but it is not the SDK size source. See [recovered contracts](DEVICE_CONTRACTS.md)
  for the complete path and the corrected controller-kind argument interpretation.
  An original exported Pedal profile and physical acceptance remain unverified.
- Studio has 32 rectangular 144 × 112 keys and two encoders without touchscreens.
  The editor provides dial assignment labels, settings and push/turn controls. SDK
  feedback metadata is retained, but no LCD layout is loaded or sent for a Studio
  encoder. LED/ring feedback is not implemented. Brightness maps 0–100% to native
  values 1–100, with integer truncation, matching the recovered factory and mapper.
- The recovered Plus brightness curve stays Plus-only. Other documented
  transports receive the requested percentage directly, except original 2017,
  which uses its independently recovered firmware-specific curve, and Studio,
  which uses the 1–100 range above; physical luminance
  equivalence has not been measured.

No reset, bootloader command or firmware write is part of these tests.
Screensaver completion and prolonged hardware reliability testing remain deferred.

## Sources and checks

The transport sources are Elgato's [general HID reference](https://docs.elgato.com/streamdeck/hid/general/),
[Mini](https://docs.elgato.com/streamdeck/hid/mini/),
[15-key](https://docs.elgato.com/streamdeck/hid/stream-deck-classic/),
[XL](https://docs.elgato.com/streamdeck/hid/stream-deck-xl/),
[Plus](https://docs.elgato.com/streamdeck/hid/stream-deck-plus/),
[Plus XL](https://docs.elgato.com/streamdeck/hid/stream-deck-plus-xl/) and
[Neo](https://docs.elgato.com/streamdeck/hid/stream-deck-neo/) pages.
The [SDK device guide](https://docs.elgato.com/streamdeck/sdk/guides/devices/),
[Neo infobar guide](https://docs.elgato.com/streamdeck/sdk/guides/neo-infobar/) and
[Neo quick start](https://www.elgato.com/us/en/explorer/products/neo/elgato-stream-deck-neo-quick-start-guide/)
provide SDK geometry and navigation semantics.

`cargo test` includes packet reconstruction, input bounds, model-specific
features and session-frame/reconnect tests. `tests/models_interop.py` exercises
the real host and plugin WebSocket protocol with simulated devices, checking the
last key/dial, SDK geometry, profile export/import and persisted-model restart.
`tests/native_models_interop.py` captures all eight layouts in the actual native
WebKit window and checks visible controls and final-key selection. The unchanged
Counter, Layouts and Lights Out samples, whole Plus profiles, clipboard transfers
and SDK device-lifecycle regressions also pass.

Static Pedal and Mini findings are pinned to original Stream Deck 7.6.0.23012
`AppExecutable`, SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
Pedal registry `0x140135a22`/`0x140135a65` binds model and USB identity; its
vtable shares serial `0x14061aa40`, firmware `0x14061ab50` and input
`0x14061abe0` with the documented 2019 family. Mini image loop
`0x14062e920` establishes zero-based chunks and last-chunk display flag. Local
research captures live under `evidence/device-protocols`; proprietary binaries
and raw disassembly are not required or distributed with Deckforge.

Checkpoint validation on 12 October 2026: 391 Rust tests passed (11 explicit
hardware/runtime probes ignored), 56 frontend tests passed, and strict Clippy
passed. Native captures cover all seven layouts. The installed application
reconnected the user's Plus and all four saved plugins, with four assignments
retained; saved state changed only by adding model metadata. No physical input
was injected into the user's configuration. The additional models still need
physical acceptance.

The following device checkpoint adds original 2017 key/input/brightness support
and Neo sensor indicators. It passes 400 Rust tests (11 explicit probes ignored),
59 frontend tests, strict Clippy, eight native model captures and the model
process suite. Both directions of 2017/later 15-key profile import are tested.
The independent public Qt fixture program reproduces the Rust BMP payload hashes.
The subsequent [controller-contract investigation](DEVICE_CONTRACTS.md) resolves
Pedal SDK geometry as 3 × 1, matching the implementation. It also records the
remaining original-2017 controls and Studio USB contracts. Those newly recovered
original-2017 controls are still pending. Studio key output, encoder input and
brightness now have codec, process and native editor coverage, with the remaining
limits listed above.

Encoder-display models offer **Page options → Set display background** and
**Remove display background**. Images are copied into managed profile storage
and embedded on export; changing the selected device while the file dialog is
open does not redirect the edit. Galleon also streams animated WebP backgrounds; other models retain still-image behavior.
See [Galleon backgrounds](GALLEON.md) for the rendering and evidence boundary.
