# Screensaver protocol and evidence

Deckforge implements its own Rust host, using the original Windows 7.6 application
and Elgato's published protocol as references. No existing Linux Stream Deck
implementation was used. The analyzed StreamDeck.exe SHA-256 is
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
Vendor binaries and private captures are not distributed here.

## Reference behavior

The original `com.elgato.streamdeck.system.sleep` action executes on key release.
Missing/empty `DeviceUUID` targets all device IDs, while a nonempty value selects
one. Static custom images and GIFs both create a software screensaver object.
That mode disables the firmware idle timer and uses a communication-worker
activity deadline. Without the software object, Plus can use the firmware timer
and a brightness-fade path. The native waking path uses no-op key/touch/encoder
handlers, consumes the first positive read, drains queued reports, then restores
normal image caches. Window activation also requests waking devices.

These facts come from bounded native traces, not a complete Windows runtime
comparison. Global activity sources, every separately arriving release edge and
higher-level Windows lock enforcement remain incompletely established. Linux
therefore explicitly suppresses physical input while known locked and retains
that policy if lock observation becomes unavailable.

The original Plus brightness mapper uses minimum 4 and maximum 100:
`raw = 4 + floor(logical_percent * 96 / 100)`. Its normal logical zero is raw 4;
raw zero is used separately by hardware sleep. This corrects the initial direct
logical-to-HID mapping. The raw HID API remains 0–100.

## Published USB contract

[Elgato's general HID reference](https://docs.elgato.com/streamdeck/hid/general/)
defines 32-byte feature reports: brightness03/08, sleep duration03/0D with signed
little-endian seconds, and getter0A. Zero seconds disables the firmware timer.
Firmware 2.0.3.4 was observed briefly returning a previous duration after a write;
Deckforge retries readback for up to 100 ms without repeating the setter.

[The Plus reference](https://docs.elgato.com/streamdeck/hid/stream-deck-plus/)
defines full-LCD image output02/08, reserved byte 0, JPEG 800×480, no rotation,
1024-byte reports and 1016-byte chunks. The
[official pixel diagram](https://docs.elgato.com/img/streamdeck/hid/sd-plus-pixel-layout.svg)
places 120×120 keys at `(13 + 219*column, 12 + 160*row)` and the touch strip at
`(0,380,800,100)`.

## Implementation boundaries

The device worker owns temporary firmware-timer changes. It captures a known
baseline before disabling, restores in worker order, coalesces duplicate values,
and never uses late host reports as restoration acknowledgements. Screensaver
frames carry generations so an old animation frame cannot re-enter sleep after
wake. Normal key/strip frames continue to update in the cache, and reconnect
replays the currently selected layer. No reset, persistent gallery installation
or firmware update is involved.

On Plus, normal rendering first clears the full LCD to black with the documented
`03/05` RGB fill feature report, then replays the key and touch-strip images.
This runs once on connection/reconnection and after leaving a software
screensaver. Individual image transfers leave the pixels outside their windows
untouched; without this clear, previous full-screen content can remain visible
along the strip and key edges. Active screensavers are never cleared by this
normal-rendering path. Other models retain their existing behavior.

The 14 October 2026 regression test first failed because no clear preceded normal
frames. It now checks startup, wake, reconnect, and absence of repeated clears on
ordinary updates. The 31 device-session tests and the simulated sleep/wake
integration suite pass. On the connected Plus, the installed release build
accepted the black fill followed by all eight key images and the touch strip.
User photos show colored fragments around normal controls; their disappearance
still requires optical confirmation after applying the fix.

Software playback uses Deckforge's logo by default, aspect-preserving black
letterboxing, continuous GIF looping and a 10 ms floor for zero-delay frames.
Native crop/zoom, finite GIF-loop semantics, standby installation and the original
firmware-only fade path are separate remaining work.

## Validation

Unit tests cover packet construction, pixel geometry, GIF disposal/timing,
resource bounds, layer restoration, stale generations, bounded wake draining,
worker-owned timer races, reconnect, lock-policy races and idle deadlines.
Private D-Bus fixtures exercise KDE/Mutter/logind queries; live KDE observation
confirmed lock detection while its Wayland idle query reports unavailable.

The process suite uses generated assets and an unchanged official SDK to check
plugin continuity, settings/cache persistence, rejected stale preparation,
static/GIF frames, wake gestures and lock simulation. A 20-frame 1024×1024 GIF
import took about 242 ms; concurrent state requests stayed under 1 ms after decoding
was moved outside the host mutex, compared with 232 ms before.

On the connected Plus, the full-LCD transfer and all eight key plus touch-strip
restore transfers returned successful HID writes. The original timer and saved
user session were verified afterward. This does not establish optical appearance,
physical wake-input acceptance, a five-minute idle wait, or real lock/unlock.
