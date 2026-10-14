# Galleon profile and layout gap

The profile/layout implementation is now verified; see `GALLEON.md`. This
reference records the geometry behind the original `GRETSCH` importer gap.
It is a distinct device, not an alias for Stream Deck+.

The supplied MSI includes archive
`../evidence/elgato-7.6/raw/flsKaSPXYxUUvmKtC3TdqgYiqQcKKY`, SHA-256
`b9fa67b7956999fabf6bc2308504cfa23bf99e744a5233a63dcfd17c0698a7b1`.
Its root profile manifest identifies `GRETSCH` and the name
`Galleon Default Windows`.

Retained REA evidence `ev_5e7c4f74880089001467d30b6931c4f2d5b48826c0a1f4c48daae299083c3fab`
in `../evidence/device-controls-followup/rea-session/responses/014-model-lookup.json`
contains the native descriptor construction. The block immediately BEFORE
`FUN_1401d56b0(local_638,&PTR_s_GRETSCH_14159f550,local_29b8)` records:

- Keypad: 3 columns, 4 rows; full frame 720 × 1280; key images 160 × 160;
  secondary geometry also 160 × 160 and preview 120 × 120.
- Key origins: x = 56, 280, 504; y = 443, 667, 891, 1115.
- Encoder controller: 2 columns, 2 rows; display area 720 × 384.
- The following descriptor, ending at `PTR_s_GRETSCH1G_14159f558`, repeats this
  geometry. Do not assume identical transports from matching geometry.

Official SDK documentation assigns device type 12. Corsair documents two physical
dials controlling four widgets, two per dial. Holding a dial for three seconds
and releasing changes its selected top/bottom widget. The display is not touch
enabled. Logical encoder positions therefore differ from physical dial count.

Implementation must preserve the original profile model and two-dimensional
encoder coordinates. The host now preserves both encoder rows and routes physical
dial events through the saved selection. The native-contract transport is now
implemented; physical verification remains pending.

Sources:
- [SDK devices](https://docs.elgato.com/streamdeck/sdk/guides/devices/)
- [Corsair guide](https://www.corsair.com/us/en/explorer/gamer/keyboards/galleon-100-sd/)
- [Corsair display and dials](https://www.corsair.com/us/en/p/keyboards/ch-912a31i-na/galleon-100-sd-stream-deck-integrated-mechanical-keyboard-ch-912a31i-na)

## Additional native transport evidence, 14 October 2026

The focused native investigation is saved under `../evidence/galleon/`, including
`rea-bundle.json`, `rea-snapshot.json`, `registry.asm`, `factory.asm`, and vtable
inventories. The source binary was never executed.

Registry code at `0x140135b12` binds `GRETSCH` to VID/PID `1b1c:2b18`;
`0x140135bba` binds `GRETSCH1G` to `1b1c:2b4b`. Both specify usage page `0x0c`,
usage `0x01`, interface `0`, and the same factory `0x1405f16f0`. The HID enumeration
comparison at `0x14062d570` corroborates the usage-page/usage/interface fields.
Factory `0x1405f16f0` installs `ESDHWGRETSCHImp` vtable `0x1416a3320` and confirms
the previously recorded geometry, JPEG selection and quality 90.

The shared writers establish these static packet formats:

- Key JPEG, `0x14062dcf0`: 1024-byte reports, 1016-byte payload,
  `02 07 key last lengthLE16 chunkLE16`, then image bytes.
- Whole LCD, `0x14062dbb0`: `02 09 00 last chunkLE16 lengthLE16`, then payload.
  The field order differs from the key writer.
- Display image, `0x14062de30`: controller identifiers `0x102` and `0x1fc` select
  command `0x0b`; `0x1fe` selects `0x08`. Key-style header with index zero.
- Rectangle update, `0x14062dfe0`: `02 0c xLE16 yLE16 widthLE16 heightLE16
  last chunkLE16 lengthLE16 00`, with up to 1008 payload bytes.

Polling must not be inferred from a single override. Base initializer
`0x1406191a0` installs `ESDCommKeypadPolling` at `0x100` in normal mode, and
`ESDCommCommandPolling` at `0x27e`. Galleon `0x14061b3e0` calls that initializer,
adds encoders at `0x103`, and replaces identifiers 0 and `0x112` with callbacks
returning `-2`. It does not replace the keypad entry. The sleeping variant uses
`0x140619250` and explicitly substitutes no-op keypad/encoder handling.

`ESDGretschAdapter` has a secondary interface vtable at `0x141672990`; its primary
QObject table alone does not expose the adapter methods. Constructor
`0x14033a1f0` creates the encoder backing image from the descriptor. Method
`0x14033aa10` uses full `0x102` display updates or aligned rectangular updates;
it aligns x/y to even positions and widths/heights to multiples of 16. Its
coordinate transform at `0x14033a2a0` uses an aspect-preserving fit; the source
canvas and resulting content rectangles are recorded below. Pixel
equivalence still requires background and marker rendering verification.

The native-contract startup, keepalive, input and image transport are now
implemented; the additional evidence is recorded below. The latest local USB
inventory
contains the Stream Deck+ only. These findings do not establish physical Galleon
acceptance.

## Native routing and exact content geometry

The retained bundle and snapshot in `../evidence/galleon-routing/` contain the
Windows 7.6 investigation (source SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`). The executable was
not run. Relevant functions:

- `0x140422440`: physical press resolves the selected row. Multiple assignments
  defer down; release at <=500,000,000 ns replays the press for its measured
  duration, while a longer press changes rows without action activation.
- `0x14017f9c0`: performance-counter conversion establishes nanosecond units.
- `0x140423d20`: cycle assigned rows, skipping empty positions and wrapping.
- `0x140423e60`: retain valid selected rows or select the first assigned row;
  persist the Controller `Routers` map. Shipped profiles corroborate this field.
- `0x140426390`: a 250 ms timer previews the next row after the threshold.
  Deckforge does not yet reproduce this preview animation.
- `0x1404246b0`: marker rendering applies only to columns with multiple actions.
- `0x140422a40`: rotations route to the selected row with the physical held flag.

Composer setup `0x140420420` and constructor `0x140429f30` establish a 2×2
grid of 200×100 cells, outer margins 15, horizontal gap 20 and vertical gap 10.
The canonical canvas is 450×240. The adapter's aspect-preserving scale to
720×384 is 1.6: cells are 320×160, origins (24,24), (376,24), (24,200),
(376,200), gaps 32×16. This replaces the earlier equal-quarter approximation.

Routing, timer and clock evidence includes
`ev_da1420b4c11740af1d6ab6291327a3723f63cb6419098f573ab221e499ea98bc`,
`ev_15766e138fbfe91456abd82670b7d98758a29df484db4210084caf0c3d2c2c58`,
and `ev_11d8f73b2a8ccad73b91ea27f350b2810020a4b676aacacfd6fbdd3900262d43`.
Composer evidence includes
`ev_6764ff99c4899f1c0816f6916dfef7bccf9618da4ac59262511ca6523c003c5e`
and `ev_720ca528d42e46bdcd27e266fa5eba343b92529c60fe0f7165d70e5075fe1d94`.

## Native startup, heartbeat and input contract

The follow-up bundle and snapshot in `../evidence/galleon-transport/` contain
these observations from the same binary. Assembly and RTTI inventories are in
`../evidence/galleon/metadata.asm`, `transport-tables.json` and `init-tables.json`.

- `ESDCommOpenTask` dispatch `0x140590eb0` calls hardware slot 5 to open the
  filtered HID interface. Open completion `0x1405934a0` sends slot 7 (keepalive)
  before scheduling a preheat wait. Factory `0x1405f16f0` supplies 200 ms.
- `0x140593720` reads firmware, adjusts capabilities, reads serial, initializes
  display content and starts polling plus periodic keepalive. Factory offsets
  `0x138/0x13c` enable keepalive with 500 ms interval; worker `0x140595c70`
  schedules the next ping after each successful ping.
- `ESDCommKeepAliveTask` (`0x140590ec0`) dispatches hardware slot 7 to
  `0x14061b1e0`: initially a 32-byte feature report `03 27`, or a 1024-byte
  output report `02 25` when its firmware flag is set. `0x14061b110` sets that
  flag when the second dotted version component is >=6. This is a minor-field
  comparison, not a general semantic-version threshold. Reserved bytes in the
  Linux implementation are zeroed.
- Serial reader `0x14061aa40`: feature report 6, 32-byte response, length at
  byte 1, ASCII from byte 2, capped at the factory's 14 bytes and first NUL.
- Firmware reader `0x14061ab50` / decoder `0x14061b070`: report 5, 32 bytes,
  version at byte 6, length `min(report[1]-4, 8)`, terminated by NUL.
- Input reader `0x14061abe0` maps byte 1 to controller `0x100 | byte`, uses
  little-endian payload length at bytes 2/3 and data at byte 4, independently of
  byte 0. Linux validates the actual received length before accessing payload.
- Keypad polling `0x140869df0` processes min(payload length, 12) states, treats
  nonzero as pressed, preserves omitted keys, and emits only changes. The
  default index map at `0x14062e310` is identity.
- Encoder polling `0x14086a140` uses subtype 0 for nonzero pressed states and
  subtype 1 for signed 8-bit relative rotation. Galleon has two physical dials;
  logical widget routing remains in the host, not the HID decoder.
- Background clear `0x14062e190` accepts controller `0x1fe` and emits feature
  `03 05 R G B`. Brightness `0x14062daa0` uses `03 08 value`; factory range
  is 4–100. Default image transform fields are identity and rotation is zero.
- `0x14061b2c0` supplies LED-RGB `03 24 index R G B` (negative index iterates
  eight LEDs); mapping these LEDs to user-visible controls remains unimplemented.
  `0x14061aca0` is a firmware upload path, not normal initialization; Deckforge
  does not invoke it.

Evidence records include
`ev_ef21d3874f733bce03d98b396bc7c12439b8d4cb019ac203c54d7a82b5fe4fd9`
(metadata/keepalive),
`ev_f5fd26f52c45bf98140c0a3294b35dc0c97cd97276464311c32a5f245d24a08c`
(input), and
`ev_5a8f7688fed746d774b48be83da6f72e94a78eb4781ce2dced26f12f39745d38`
(startup sequencing). Static evidence and software tests support implementation;
physical Galleon operation remains unverified.

## Background composition and pending marker evidence

The follow-up bundle in `../evidence/galleon-display/rea-bundle.json` records:

- Controller `Background` parses at `0x1403d66d0` and serializes at
  `0x1403db670`; it is independent of action state images and saved `Routers`.
- `0x140407220` loads a still QImage; encoder override `0x1404213f0` installs
  it into the full-composer background layer and hides the movie layer.
- Galleon override `0x140421130` handles `.webp` using the movie layer and starts
  its animation. Deckforge now streams Galleon WebP animations; the implementation and
  verification limits are recorded below.
- `CPixmapElement` rendering `0x1405c11a0` calls `0x1405c1380`. Its default
  no-effect branch draws the source pixmap into the destination rectangle,
  establishing stretch-to-canvas sizing for the background layer.
- Marker construction `0x14042fc70` expands each canonical 200×100 widget
  rectangle by 6 on all sides, sets border radius 5, border width 2, white border
  (Qt color3), and outline width1. The shared CElementImp constructor `0x1405ad310` confirms the outline
  color RGBA30,30,30,180. The marker element type is `rectangle`, independently
  confirmed by its factory argument at `0x14042fe53`. It is an external rounded border, not an underline.
- Pending row preview `0x14042f3b0` fades the old marker out and the next marker
  in over 0.15 seconds. The recovered 250ms hold timer calls it after >500ms;
  actual routing remains unchanged until release. Deckforge now implements these marker and preview contracts, with
  scheduler, SDK-route, image-layer and native-window checks. Its SVG rasterizer
  is independent of Qt; exact antialiasing equivalence remains unverified.

Relevant records:
`ev_1ad58786150736814c7f602e583ceae1566b488d3f28ccfacd5abea6efb34445`
(background metadata),
`ev_5ef7f1fa0b279edeffde793dc2b588322171ff85e40caff2a35ebb6f40bbbaca`
(background application),
`ev_a36aa6ee8ec82ae945d9b7091280d513bc53b1072fc9b1402599945765295ad3`
(pixmap rendering), and
`ev_b9f559234d7a1e48a5ce869d0f30daa578cbcde4f57a4c377de4bf054ef219c6`
(marker construction). The original fixture image is 720×384 and was extracted
for local verification only; it is not added to application assets.

The marker follow-up confirms the base outline color using
`0x1405ad310` and retains the same recovered rectangle geometry. Linux samples
preview fades on its existing 33 ms scheduler and stops scheduling after the
fade completes. Preview state is device-owned and never persisted into Routers.
Only release commits the next route.

## Streaming WebP implementation and controlled Qt comparison

The recovered Galleon movie-layer override is implemented with the existing
`image-webp` decoder (now an explicit dependency). The encoded asset and one
streaming decoder canvas are retained; frames are not accumulated in memory.
Playback is device-owned through the existing runtime swap. Frame publication
uses the current background/feedback renderer and scheduler. Original asset
bytes and profile metadata do not change during playback.

[Qt's WebP handler](https://raw.githubusercontent.com/qt/qtimageformats/6.8/src/plugins/imageformats/webp/qwebphandler.cpp)
returns encoded frame durations and translates the WebP loop count to Qt's
repeat convention.
[QMovie](https://raw.githubusercontent.com/qt/qtbase/6.8/src/gui/image/qmovie.cpp)
uses those delays and stops after finite playback. These Qt6.8 sources are a
reference, not proof of the exact Qt build shipped inside the supplied app.

`tests/background_movie_reference.py` builds a small owned Qt reader and compares
three synthetic frames with the Rust decoder, including alpha blending and
clearing the previous frame's rectangle. Installed Qt6.12.0 reports delays
40/80/120ms and one repeat for a two-cycle WebP; the maximum channel difference
is1. Results: `target/galleon/webp-reference/comparison.json`. This is controlled
local replay, not execution of the Windows application.

Rust tests verify finite/infinite scheduling mechanics, zero-delay progression,
decode-error termination, identical editor/device frame pixels, full-resolution
output, independent device clocks, page cancellation/return and original-byte
export. Native tests verify changing WebP colors, page departure/return, export
and restart after removing the user's original source file. Long/high-resolution
clips under CPU load, uncommon disposal combinations and physical Galleon output
remain outside this acceptance evidence.

The Create New Background callback0x140853140 is a small dispatch thunk, not a
Ghidra procedure. Its assembly calls virtual slot0x1a8 on the owning UI object.
The resolved handler is0x140852170: it opens the per-action200×100
Customize Image cropper. It is separate from page backgrounds; see
`ENCODER_APPEARANCE.md` for the recovered fields and remaining UI work. Assembly is retained in
`../evidence/galleon-display/create-background-thunk.asm`; do not repeat the
failed procedure decompilation at the thunk address.
