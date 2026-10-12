# Original 2017 Stream Deck

The original 15-key Stream Deck (`0fd9:0060`, profile model `20GAA9901`)
uses a separate BMP transport. Deckforge implements its key images, input,
brightness, serial and firmware metadata from static inspection of the original
Windows app. No existing Linux driver was used.

Run it with `--model classic-2017`, or let connected-device discovery select it.
Its profile coordinates match the later 15-key devices, so these profiles can be
used on either generation without discarding their original metadata. USB
selection and reconnect still distinguish the generations.

## Firmware-dependent behavior

The implementation reads firmware before enabling output. It accepts the
recovered `1.0.<build>` family, with an optional fourth numeric component.
Malformed or other firmware families produce an explicit error. Supporting an
unknown family requires evidence for its behavior.

| Build | Image transform | Logical brightness range |
| --- | --- | --- |
| Before 170133 | Rotate 180° | 10–70 |
| 170133 | Rotate 180° | 3–100 |
| 170134–191202, except 170143 | Flip horizontally | 3–100 |
| 170143 | Rotate 180° | 3–100 |
| 191203 and later | Rotate 180° | 3–100 |

A 72 × 72 image becomes a 15,606-byte, bottom-up BGR BMP with a 54-byte header.
Two 8,191-byte output reports carry 7,803 bytes each, with a one-millisecond
interval. Hardware image indices are one-based and reversed within each row;
input indices are zero-based and similarly reversed. The editor and plugins
always use ordinary left-to-right coordinates.

Sleep, full-panel output, firmware updating and user-configurable device rotation
are not implemented for this model. Software screensaver work remains deferred.
The transport has no physical acceptance test yet.

## Evidence and reproducibility

The pinned Stream Deck 7.6.0.23012 `AppExecutable` has SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
The following addresses identify the independently inspected path:

| Evidence | Native address |
| --- | --- |
| Factory and model geometry | `0x1405eff30` |
| Firmware branches | `0x140618f80` |
| Image transform and serialization caller | `0x1406164c0` |
| BMP serializer, format 0 | `0x14079ac9e` |
| Two-report image writer | `0x1406188b0` |
| Input reader | `0x1406185e0` |
| Input/image index mapping | `0x140619140`, `0x140619170` |
| Serial and firmware readers | `0x1406183d0`, `0x1406184e0` |
| Brightness packet and percentage conversion | `0x140618280`, `0x1406177f0` |

The unit tests cover firmware thresholds and the exceptional build, malformed
metadata/input, image orientation, row mapping, report lengths/padding and
brightness conversion. A synthetic four-quadrant image is independently
transformed with Qt and serialized by our own small fixture program in
`tests/fixtures/legacy2017/qt-reference.cpp`. Its complete BMP hashes match the
Rust payloads. This program neither loads nor executes Elgato code.

With Qt6 development packages installed, reproduce the optional fixtures:

```sh
mkdir -p /tmp/deckforge-2017-fixtures
c++ -std=c++17 -fPIC tests/fixtures/legacy2017/qt-reference.cpp \
  $(pkg-config --cflags --libs Qt6Gui) -o /tmp/deckforge-2017-reference
/tmp/deckforge-2017-reference /tmp/deckforge-2017-fixtures
sha256sum /tmp/deckforge-2017-fixtures/*.bmp
```

Expected SHA-256 values:

- Horizontal flip: `52d8b2fb72247894d29252880203e59cc03b8ece7214d97c1a24004eb3d8cb6c`
- 180° rotation: `84aeb2ce6d8cb35705491f0a113ef3dadb7444cd4aa753bc862a8aa7c715eb95`

Qt is not a dependency of the production Rust transport. These fixtures verify
transformation and framing, not physical USB acceptance or visual equivalence of
font rasterization.
