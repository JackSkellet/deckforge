# Imported Encoder appearance

Stream Deck7.6 has two distinct background layers:

- Page controller `Controllers[].Background` covers the whole encoder display.
- An action's `Encoder.background` covers its own canonical200×100 widget.

The original Plus default profile uses a transparent action background to show
“Swipe left to turn page” over the page artwork. Deckforge now resolves this
asset through the action's retained archive provenance, composites it over the
correct page crop, and renders it in editor and physical frame preparation.

`Encoder.Icon` supplies a custom dial icon independently of the background.
`Encoder.IconVisible=false` disables the feedback layout's `icon` item without
hiding the separate software dial icon. Visibility is applied to a rendering
copy of feedback, leaving the plugin's data and saved metadata intact. Existing
explicit per-state image overrides retain their precedence.

Caches include source archive, page manifest and image path; removing a raw
reference invalidates its cache. Missing assets produce a diagnostic and blank
layer. Selected Dial Stack children can supply a background, with the parent's
explicit background taking precedence. Trigger/Wheel composition receives the
same resolved backdrop. Moving actions between profiles rebases both Encoder
asset references, including filename conflicts, without rewriting asset bytes.

## Native evidence

The source is the supplied Windows7.6 AppExecutable, SHA-256
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
The retained bundle is `../evidence/galleon-display/rea-bundle.json`.

- `0x1403d4bd0` / `0x1403da550` parse/serialize case-sensitive Encoder `Icon`,
  lowercase `background`, and optional `IconVisible`.
- `0x1403baf10` returns the action background. `0x14042aa20` installs it in that
  widget's background layer. `0x1403bc610` maps IconVisible to `icon.enabled`.
- “Create New Background” dispatches from UI helper slot53 to `0x140852170`.
  Both ESDPropertyInspectorDialHelper and DialStackHelper use that function.
  It opens `ESDBackgroundCropperDialog` at200×100 and applies its result through
  `0x1403a98b0` / `0x1403bb7c0`. It is not the page-background selection command.
- Cropper constructor `0x1405d46d0` names the dialog “Customize Image”. Its zoom
  slider spans40–4000; the exact transform and output contract still need tracing.

Key records: `ev_3a39555e07b31ad1ebd07c79028f0ba9d2fd70ab0447f37d8729efb94196a168`
(creator), `ev_15f3c8b530281bcce6c270fb0093c9ee18ad09352c4d355948c3c4a4e8229210`
(serialization), and
`ev_18e947861a6227ea8e61e7a33217facfd9d947a8f99f91605e357e140fb28258`
(visibility/background getters).

## Verification and remaining work

Rust tests cover separate icon/background pixels, transparent composition,
visibility precedence, both display planes, draft rendering and cross-profile
asset conflicts. Native WebKit tests load the untouched original Plus profile
`../evidence/elgato-7.6/raw/flsysF8Fq6CoTq162E1zc6fyeIlKcw`, preserve image bytes
and metadata on export, and repeat after restart. The tip's composed pixels
match a browser composition of the original image/page artwork within one
channel value. Reports/captures are in `target/encoder-assets/native/`.

## Customize Image

The selected dial now offers Customize background, Remove background and Show
feedback icon. The cropper provides pointer/keyboard pan, quadratic zoom,
align-to-width/height, cover/reset, edge snapping and still-frame selection from
GIF/WebP sources. Arrow keys move one pixel; Shift moves ten. A transparent
200×100 PNG is produced only when Set Image is pressed. Title and feedback icon
layers stay separate. The original per-action caller disables Set Animation;
this cropper therefore saves a selected still frame.

Source decoding and cropping run outside the engine lock. Saving rechecks the
source hash, action/device ownership, read-only status and previous background
reference. Cropped PNG bytes live in internal session overrides and are embedded
under the native Encoder.background reference on export; no source-file path or
private override field is exported. Copies retain these pixels independently of
the original file. Nested edits retain an override while its raw reference is
unchanged. Cancel does not mutate the action.

Drafts survive hiding/reopening the editor in the same host process. Saved
images survive a full app restart. A successful save clears the retained dialog
draft even if its original WebView was hidden before the response. The modal
keeps Save visible on short windows and supports normal dialog focus/Escape.

Resource limits are16MiB input,16MP source pixels,8192 pixels per side,1024
source frames and256MP decoded work for a requested frame. Preview thumbnails
are at most1024 pixels; saved output uses the original selected frame. The
scaled crop image is limited to4000 pixels per side. Native snap/rounding and
rasterization equivalence, large-source timing, broader SDK precedence, animated
per-action output and Windows edited-export/physical acceptance remain open.
Page-background controls still cover a separate feature.

Crop tests and native screenshots are in `target/cropper/`. The native test
checks blank-dialog cancel, actual GIF frame pixels, fit/reset, keyboard pan,
in-process window draft restoration, saving, export and restart after deleting
the source. Chromium layout checks separately cover reported viewports1180×763
and760×533. The OS file dialog is not automated; frontend tests cover its
request and late-result cancellation.

Additional native contracts: reset uses KeepAspectRatioByExpanding at
`0x1405daa50`; zoom maps slider value v to long-side pixels
`40 + (v-40)^2/3960` at `0x1405d8f50`. Fit-to-width is `0x1405da830`.
Saving at `0x1405d6920` fills a transparent target then draws the selected image
region. UI setup `0x1405d3550` and mode handling `0x1405d5cf0` establish the
controls and frame picker. Records:
`ev_b604c625188e083f6ba0442f7ce924e64c644185414b2158a73a3829c86103d2`,
`ev_07f806fe79f03649c53be9c54acd83f39dd7aec5414bd7cd1e577a7c5a20392b`,
`ev_70ace772b3dbaceda3edae90483cc37932e37245350ae6c5ed3c5b8afcc3d36a`.
