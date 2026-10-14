# Galleon 100 SD

Deckforge supports Galleon profiles, native editing, simulation and a USB
transport recovered from the original executable as the separate `galleon` model. It preserves `GRETSCH` and `GRETSCH1G` profile identities
instead of treating them as Stream Deck+.

The canvas has twelve keys in three columns and four rows, and four encoder
widgets in a two-by-two display area above them. Two separate dial controls let
the simulator press, rotate and select rows.
Encoder previews are 320 × 160, composed into a 720 × 384 area at (24,24),
(376,24), (24,200) and (376,200); key previews are 160 × 160. The public SDK
device type is 12. Feedback
uses the existing canonical layout renderer scaled into the widget area; exact
vendor rasterization and physical image transforms remain unverified.

Logical widget indices are row-major: 0/1 on the top row, 2/3 on the bottom row.
SDK coordinates, profile import/export and nested key actions preserve the
original column and row. Nested actions now inherit the coordinates of the
owning control instead of recomputing them from the key grid. Existing models
retain their previous coordinate behavior.

The display is not touch-enabled. Simulated touch and swipe requests are
rejected, and touch controls stay unavailable. The simulator offers both logical-widget events and physical-dial routing. USB
transport is implemented from native contracts, with physical validation pending.

## Dial routing

The two physical dials address their selected assigned widget. Selection is saved
in the original Encoder controller `Routers` map and survives export and restart.
Empty rows are skipped. With two assigned rows, a press is deferred: release
after more than 500 ms changes rows without pressing either action. A shorter
press replays down/up with the measured duration. With one assigned row, down/up
is immediate. This follows the supplied 7.6 executable; Corsair's guide instead
describes a three-second hold.

Held rotations retain the physical pressed flag. Pending gestures are cancelled
on page departure, explicit cancellation or device loss, using existing input
ownership cleanup. Rapid short presses use a bounded FIFO of 32 pulses to keep
down/up balanced; equivalence to the vendor's rapid-input race behavior is not
claimed. Native simulator buttons submit complete clicks, rotations or row
changes, so closing the editor cannot strand an editor-owned press.

A rounded white border marks the selected row in columns with multiple
assignments. A 250 ms hold check starts the next-row preview after >500 ms,
fading the old border out and the next one in over 150 ms. Routing still targets
the original action until release. Cancellation restores the selected border.
The border sits outside the widget content and uses the recovered geometry and
outline color; exact Qt rasterization and physical appearance remain unverified.

## Imported controller backgrounds

Page-owned Encoder `Background` images now render behind feedback and in empty
widgets, in both the native editor and physical frame composition. The host
resolves the existing archive provenance, including copied pages, and caches
scaled images and preview crops. Alpha is composed over black; ordinary layout
items, Dial Stack children, Action Trigger and Action Wheel draw over the correct
widget crop. Plus uses the same background path without Galleon's gutters.

Source images and references remain byte-identical on export. Page changes clear
or restore the current background. Missing assets fall back to the regular
canvas with one diagnostic per cached source. Restart uses the managed archive,
so the user's original import file is no longer needed.

The Page options menu offers Set display background and Remove display
background on encoder-display models. Selection captures the device, profile
and page before opening the file dialog. The host stores a separate managed
background archive, preserving existing action assets and metadata. Duplicate
pages and copied folders retain this source; exports embed the original chosen
bytes. Invalid images and read-only profiles are rejected before changing the
page. Removing a missing imported background is also supported.

Galleon WebP backgrounds now play with encoded frame delays and finite/infinite
loop counts. Playback retains the encoded file and decoder canvas rather than
all decoded frames, including at the full 720×384 display size. Each device has
its own playback state. Leaving the page drops that playback; returning or
restarting begins again. A decoding error stops at the last good frame and logs
a diagnostic. Original WebP bytes remain unchanged on export.

The input limit is 16 MiB and source images are limited to 16 megapixels. These
are Linux resource limits. Zero-delay frames advance one per scheduler turn.
Other encoder models continue to render still backgrounds, following the
recovered Galleon-specific movie override. The vendor's Create New Background workflow edits an individual action's
200×100 image using a cropper; it is a separate feature. See
`ENCODER_APPEARANCE.md`.
Exact Qt rasterization, unusual alpha/disposal files, large-image timing under
load and physical output remain unverified.

## Verification

- Physical-routing process tests cover SDK coordinates, held turns, short-press
  timing, long-press switching, cancellation, rapid clicks, export and restart.
- Native simulator tests check balanced SDK events, row buttons and the visible
  border in the captured window, as well as its source image pixels and cancellation of a held preview.
- The model process suite checks all ten supported editor layouts, SDK device
  identity, last-key/last-widget events, bounds, export/import and restart. Galleon
  includes lower-row nested Action Trigger coordinates and image dimensions.
- A captured renderer test checks four distinct widgets at their actual positions
  in the composed display frame without writing to USB.
- Native WebKit tests check the two-row display above the keys, two physical dial
  markers, selection of the last widget and decoded preview dimensions.
- The complete Action Trigger suite now passes, including all eleven unchanged
  original Windows adaptor records. Each original profile runs on its declared
  model so foreign-model imports do not overwrite the active Plus profile.
- The complete soundboard suite now passes on the original Galleon fixture,
  including its embedded MP3, with captured PCM on an owned virtual audio sink.
  No physical speakers, microphones or USB devices are used by these tests.

Reports and captures are under `target/galleon/`. Original profile content is
used as a fixture, not distributed as application source. Windows acceptance of
Deckforge exports and physical Galleon behavior remain unverified.

## USB transport and remaining validation

The supplied Windows executable identifies Corsair devices `1b1c:2b18` and
`1b1c:2b4b` with usage-page/usage filtering. The native hardware adapter shares
modern JPEG key and display writers. Its base initializer keeps keypad polling
at `0x100`; the Galleon override adds encoder polling at `0x103` and unsupported
handlers for identifiers 0 and `0x112`. Those handlers do not establish that
keypad polling is disabled.

The transport filters Corsair VID/PID, usage page 12, usage 1 and interface 0.
It sends the recovered initial keepalive, waits 200 ms, reads firmware and
sends periodic keepalives at 500 ms. Firmware minor versions below 6 use a
32-byte feature report `03 27`; minor versions 6 and above use a 1024-byte
output report `02 25`. Identity selection precedes all writes. Descriptor-less
serial probes remain read-only.

Key, encoder-display and full-LCD output use the recovered JPEG writers at
quality 90 with the default identity orientation. Normal display restoration
clears background residue before replaying the twelve keys and encoder area.
Brightness maps 0–100% to the native 4–100 range. Firmware timer controls remain
unavailable because readback is unproven. No reset or firmware update is sent.

Input decoding uses twelve keys and two physical dials. Nonzero button states
are pressed, rotations are signed, and partial key reports preserve unreported
states. Unsupported controller reports neither disconnect nor wake the software
screen saver. Truncated reports are rejected before state changes. Keepalives
continue between image chunks and during input/screen-saver polling. Reconnect
uses the existing exact-device worker and reruns startup.

Codec, heartbeat, identity-filter and device-worker tests cover these contracts.
Only the user's Stream Deck+ is currently connected; no Galleon readback or
physical acceptance has been performed. Packet fixtures cannot prove keyboard
coexistence, device wake/sleep, image orientation or firmware-version behavior
on hardware. Static notes and REA evidence are in `../evidence/galleon/`,
`../evidence/galleon-routing/` and `../evidence/galleon-transport/` relative to
the repository root.

See [reference geometry and sources](GALLEON_REFERENCE.md).
