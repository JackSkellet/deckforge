# Animated GIF images

Saved user artwork, imported GIF artwork and GIF manifest images can now animate
on keys. GIFs also work through plugin `setImage`, using a file or a data URL.
For encoders, `setImage` animates the circular editor icon; it does not change the
physical touch-strip layout.

Animated `setImage` is an intentional Deckforge extension requested by the user.
The [official SDK key guide](https://docs.elgato.com/streamdeck/sdk/guides/keys/#images)
excludes animated image formats from that command. Existing static image and
hardware/software target behavior remain supported.

## Playback and ownership

- Hardware and software targets keep independent playback clocks. A repeated
  `setImage` restarts only the assigned target. Title updates keep its clock.
- User artwork takes precedence over SDK artwork. Hidden plugin updates do not
  restart the user's animation. Reset reveals the applicable plugin/default image.
- Each action state uses its own image source. Showing a different state or
  returning to a hidden page starts the visible animation at its first frame.
- Frame composition handles partial frames, transparency and GIF disposal.
  Delays have a 25 ms minimum. Infinite loops continue; finite loops stop on the
  last frame and retire their timer. A stalled renderer advances one frame and
  schedules that frame's delay from the new time rather than skipping frames.
- Trigger child images and Wheel cards use the same animated background lookup.
  An active Wheel carousel retains its frozen card snapshots until it settles.
  Appearance drafts remain still previews; playback begins on the saved canvas.

Original user/imported GIF bytes survive export. SDK GIF data is copied into the
private appearance-asset store so deleting the source file does not break a
restart. It remains runtime artwork rather than becoming a user override in an
Elgato-format export. Decoded frames and playback positions are transient; they
are never written over saved source pixels or original image bytes.

## Limits

GIF input is limited to 8 MiB, 16 megapixels per frame, 1024 frames, 64 megapixels
of total decoded frame work and 32 MiB of retained display frames per clip.
Display frames are reduced to at most 256 pixels per dimension; originals remain
unchanged. A shared cache retains at most 128 MiB of frame pixels and 2048 source
entries. Active clips are protected from eviction. These budgets are separate
from temporary decoder allocations and the existing still-image caches.

Malformed or oversized SDK/user edits are rejected before replacing an image.
An imported animation that cannot fit the runtime cache keeps its still image
and reports a diagnostic. Returning to its page can retry after capacity frees.

## Reference and validation

REA inspected Stream Deck 7.6.0.23012, executable SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
Image-node routines `0x1405d18f0` and `0x1405d1ba0` retain per-frame delays and clamp
them to 25 ms. `0x1405d1f70` advances one frame at each deadline, decrements finite
loop counts at the end, and stops on the last frame. Those are static contracts,
not a live Windows comparison. The private evidence bundle is
`evidence/key-animation/rea-bundle.json`.

Tests cover frame boundaries, nonuniform delays, finite/infinite loops, disposal,
frame-count rejection, generated hardware/editor target isolation, state changes,
user priority and retirement on page changes. Existing appearance, display-target,
Trigger, Wheel and plugin-update checks cover affected integration paths.

`tests/key_animation_interop.py` uses unchanged SDK 2.0.4 methods in an isolated
real host. It checks file/data-URL animation, static replacement, targeted writes,
state changes, encoder app icons, source deletion, restart and original-byte
profile export/import. `tests/native_key_animation_interop.py` observes changing
pixels in the actual WebKit window and exercises unchanged Counter input while
artwork plays. Neither suite opens USB. Physical acceptance, animated WebP/APNG,
feedback-layout pixmap animation, and exact Windows state-reentry/rendering parity
remain open.
