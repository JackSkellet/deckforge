# Recovered device contracts

These findings come from static inspection of the original Windows Stream Deck
7.6.0.23012 executable, SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
Addresses are virtual addresses with image base `0x140000000`. The executable
was not run. No existing Linux implementation was used, and none of these
findings establishes physical acceptance on another device.

## Pedal SDK geometry is 3 × 1

The hardware factory stores `QSize(1, 3)`, but the SDK does not take its size
from that field. The complete SDK path instead uses a controller-description
table with a separate Pedal entry:

| Step | Native evidence |
| --- | --- |
| Create the device-data service | `0x1401438e8` calls constructor `0x140458f80`; `0x1401438ee` stores its result at `0x141a11aa8` |
| Bind the implementation | Constructor `0x140458fa9` installs vtable `0x141683130`, identified as `ESDDeviceDataModel` |
| Select controller geometry | Vtable offset `0x98` resolves to `0x140459a10`; its final argument is controller kind, not an orientation flag |
| Find the Pedal model entry | Model pointer cell `0x14159f5a8` points to `20GBF9901` at `0x14164ad70`; pair builder `0x1401d56b0` dereferences the cell |
| Read the keypad descriptor | Constant `0x1416ded40` begins `[1, 3, 1, 0]`: controller kind 1, width 3, height 1 |
| Retain the fixed size | Pedal flags are `0xA11`; bit 28 is clear, so the getter does not consult configurable dimensions |
| Serialize SDK size | `0x14036ec4a` calls the getter with kind 1; width becomes `columns` at `0x14036edef`, height becomes `rows` at `0x14036ee64` |

The controller vector uses 0x50-byte records. Builder `0x1401d5a90` and copier
`0x1401d6940` preserve the dimensions at offset 4. The Pedal pair is inserted at
`0x1401d419f` and is included in the map initialized at `0x1401d4af5`.

This confirms Deckforge's existing three-column, one-row SDK layout. The earlier
interpretation of argument 1 as `orientation=true` was incorrect and is
withdrawn. A separate horizontal Pedal feedback overlay was never sufficient
evidence for SDK geometry. An original exported Pedal profile and physical
Pedal acceptance are still unavailable.

The local evidence includes REA/Ghidra records
`ev_a45d8ebc970a0e57aac0c23df3ec70506dbfe619ab08929923da84cc618c619b`
for the geometry getter,
`ev_9751d65ba19634a92d381d5b5f5b9ed7bcce839e4c890031c984a06f0da6c641`
for service references, and
`ev_5e7c4f74880089001467d30b6931c4f2d5b48826c0a1f4c48daae299083c3fab`
for the model-table initializer. Bounded disassembly independently corroborates
the relevant instructions. Proprietary binaries and raw decompilation are not
distributed in this repository.

## Original 2017 controls awaiting implementation

The remaining controls are now partly specified beyond the existing
[key-image/input/brightness transport](LEGACY_HID.md):

- Firmware sleep uses a 32-byte feature report beginning `0B A2`, followed by
  little-endian seconds at bytes 2–5. The original app enables it only for
  firmware `1.0.<build>` with build at least `191203`. Its task rejects negative
  values; zero disables the timer. Setter `0x140618330`, task `0x140591200`, and
  firmware gate `0x140618f80` establish this path. A sleep getter was not found;
  do not assume the Mini or modern-device query protocol applies.
- Orientation is a saved software preference, not a recovered USB rotation
  command. Values 0, 1, 4, 2 and -4 select standard, 90-degree icons,
  180-degree icons, 270-degree icons, and device upside down. The image pipeline
  applies Qt rotations 0, 270, 180, 90 and 180 degrees respectively, followed by
  the firmware-dependent image transform. Only -4 also reverses the logical
  key index before the existing row-reversal map.

The Linux transport still rejects these unimplemented controls. Enabling sleep
requires separating setter support from readback support and retaining unknown
observed state when no getter is established. Screen saver work remains deferred.

## Studio USB transport

Studio belongs to Stream Deck desktop parity over USB, as stated in
[Elgato's system requirements](https://help.elgato.com/hc/en-us/articles/30132132604813-Elgato-Stream-Deck-Studio-System-Requirements).
Its Ethernet workflows with Bitfocus are a separate product capability.

The native registry binds `20GBO9901` to USB `0fd9:00aa`, factory
`0x1405f1490`, and hardware vtable `0x1416a2f90`. The hardware has a 16 × 2
key matrix with 144 × 112 key images. The shared SDK type mapping returns 10.
The key writer at `0x14062dcf0` uses JPEG quality 90 and 1024-byte reports:
`02 07 key last lengthLo lengthHi chunkLo chunkHi`, with up to 1016 payload
bytes. Chunks start at zero and only the final chunk sets `last`. Default image
orientation is identity. Whole-panel command `02 09` is a different writer.

The shared input reader and encoder handler `0x14086a140` establish encoder
command 03. Its payload begins with subtype 0 for changed push/release states
or subtype 1 for signed dial deltas, followed by the two dial values. The
generic LED-RGB task is explicitly unsupported by Studio; button-RGB feature
`03 06 index R G B` exists, but its relationship to the physical rings has not
been established. Do not substitute Plus touch-strip behavior for these dials.

Deckforge now implements Studio discovery, key images, encoder input and
brightness, with codec/session fixtures, process-level SDK/profile tests and an
isolated native-window test. The editor uses the rectangular key aspect ratio
and software dial labels. Text is rasterized on a rectangular canvas instead of
being stretched from a square. Encoder feedback/settings remain portable
metadata; no Plus LCD layout is required or transmitted. UI appearance and
touch controls are unavailable for these encoders.

The controller table independently confirms Keypad 16 × 2 and Encoder 2 × 1:
model pointer cell `0x14159f578` binds at `0x1401d3fdc`, with keypad constant
`0x1416dee30` beginning `[1,16,2,0]` and encoder constant `0x1416df520`
beginning `[2,2,1,7]`. All four encoder image dimensions are unspecified.
Studio flags `0x1127` leave resize bit 28 clear.

The brightness writer `0x14062daa0` uses `03 08`; the native factory minimum is
1 and maximum is 100. The percentage mapper truncates `1 + percentage*99/100`.
A sleep setter exists, but readback remains unproven and is not enabled. Native
full-panel geometry is unspecified. LED/ring mapping, device rotation, sleep
readback, full-panel output and physical Studio acceptance remain open.


## Remaining non-USB surfaces

The desktop app's local Virtual Stream Deck and Mobile are distinct devices:
SDK types 11 and 3 respectively. The native model strings are `UI Stream Deck`
for local panels and `VSD/WiFi`, `VSD/WiFi48`, `VSD2/WiFi` for Mobile. Their
transports must not be conflated with the plugin WebSocket.

The local virtual-device descriptor in this 7.6 binary has a 10 × 10
maximum/fallback: pointer cell `0x14159f520`, binding `0x1401d449f`, controller
prefix `0x1416def50 = [1,10,10,0]`, flags `0x10000007`. The shared geometry
getter accepts a nonempty saved size only when both dimensions fit within that
bound. Older public guidance advertises 8 × 8; the user-facing size restriction
still needs a separate trace. Do not silently treat either number as a universal
contract across versions.

Mobile has separate legacy and v2 service names, `_vstreamdeck._tcp` and
`_vstreamdeck2._tcp`. The v2 initializer includes a WebSocket listener and sodium
initialization; its pairing and authorization protocol are not yet recovered.
Network Dock uses a Cora discovery/pairing path, including `_elg._tcp`, rather
than a new keypad SDK type. Studio's supported USB path does not establish
Studio Ethernet support through this dock. Local virtual panels, Mobile and
Network Dock remain unimplemented in Deckforge.
