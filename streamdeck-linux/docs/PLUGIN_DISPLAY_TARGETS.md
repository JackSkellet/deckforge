# Plugin display targets

Key actions can update their hardware image/title separately from the editor:
`target: 0` changes both, `target: 1` changes hardware, and `target: 2` changes the
editor. Omitting target uses both. `state` addresses an inactive state without
selecting it. Omitting state updates the declared states; `setState` remains a
single state shared by hardware and editor.

The same renderer draws both surfaces. User-authored titles and images take
priority over runtime plugin content, followed by manifest defaults. Missing
image/title values clear that target's runtime override independently. An empty
title is an explicit blank title. Deckforge retains its empty-image reset and
JSON-null reset policies; exact original native behavior for those inputs has
not been established. Invalid target, state and field types are rejected before
publishing the replacement action.

## Image formats

The shared action decoder supports the documented SVG, PNG, JPEG and WebP
formats for SDK file paths and data URLs. WebP lossless, lossy and alpha-bearing
images use the existing input-size, dimension and allocation limits. Invalid
SDK images are rejected before replacing the current display. Static WebP is
also supported for action-library thumbnails, encoder software icons (including
`@2x` variants), layout pixmaps and user-selected/imported artwork. Extensionless
manifest lookup and export use the same suffix list, so WebP/JPEG artwork is
preserved as its original bytes rather than silently flattened or omitted.

`tests/image_formats_interop.py` calls the unchanged SDK 2.0.4 through a generated
plugin. It verifies actual preview pixels, malformed-image rejection, dial
feedback, and exported artwork after deleting the original file and restarting.
`frontend/tests/native_dial_icons_interop.py --image-format webp` checks decoded
pixels in native Plus/Studio windows and independence from the Plus LCD. Decoder
and thumbnail tests cover alpha and dimension limits. These checks use simulated
devices and do not establish physical WebP display acceptance.

The [SDK key guide](https://docs.elgato.com/streamdeck/sdk/guides/keys/#images)
explicitly excludes animation from `setImage`. Deckforge now deliberately extends
that command with GIF animation at the user's request. It also animates saved
user/imported key GIFs and encoder app icons. Static WebP support and the separate
screensaver policy are unchanged. See [animated images](ANIMATED_IMAGES.md).

## Dial icons and touch displays

The SDK's `DialAction.setImage` changes the action's circular icon in the editor.
It does not replace touchscreen feedback. `Encoder.Icon` supplies the default,
including extensionless asset paths and available `@2x` variants. User artwork
still has precedence. Dial Stacks show the active child's software icon. The
Studio layout displays the icon without inventing an LCD for its screenless dial.

`setFeedback` and `setFeedbackLayout` retain ownership of the touch display. The
SDK's dial `setTitle` method already translates to feedback. Raw targeted encoder
titles and hardware-targeted encoder images remain rejected because their native
semantics have not been established; omitted/both/software encoder images target
only the app icon.

Old Deckforge runtime dial-image caches migrate to the app icon without becoming
exported user artwork. Plugin updates clear derived images for both targets and
the dial icon while retaining authored content. Removing a plugin shows the
unavailable placeholder while keeping saved artwork/settings for reinstallation.

## Evidence and remaining boundaries

The retained unchanged official SDK 2.0.4 establishes the command payloads and
Target enum. Official contracts are documented in the [key guide](https://docs.elgato.com/streamdeck/sdk/guides/keys/),
[dial guide](https://docs.elgato.com/streamdeck/sdk/guides/dials/),
[encoder manifest](https://docs.elgato.com/streamdeck/sdk/references/manifest/#encoder)
and [WebSocket commands](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/).
The public omitted-state wording describes two states; applying it to every
additional declared state is Deckforge's generalization.

- `tests/display_targets_interop.py` uses actual SDK KeyAction/DialAction methods
  through a generated plugin. Its 30 operations cover targets, inactive states,
  resets, user overrides, saved settings and restart. A same-socket settings
  round-trip is the synchronization barrier; SDK send completion alone is not a
  display acknowledgment.
- Renderer tests inspect generated `Draw::Key`, `Draw::Strip` and `Draw::Infobar`
  frames independently of editor PNGs, including state changes, custom artwork,
  a Trigger inside a Stack, and Neo's 232×50 surface. No physical device is opened.
- `frontend/tests/native_dial_icons_interop.py` verifies actual native Plus/Studio
  icon decoding, SDK icon changes and independent Plus LCD feedback. Frontend
  tests check that a dial icon cannot be mistaken for a physical LCD.
- Malformed commands and title-only image-cache splits are tested for atomic
  rejection. All runtime/user/target image planes share a 64 MiB decoded budget
  per action; splitting a previously shared image counts toward that budget.

Nested composition currently uses the parent's target for its selected children.
That preserves target isolation, but exact vendor nested-rendering semantics are
unproven. Runtime cache persistence is local policy. Native title-parameter events
after target divergence, raw encoder target combinations, pixel-identical Windows
font rendering and physical-display acceptance still need separate evidence.

The complete Stack workflow now passes, including import/export of all four
original stacks at dial columns 1–4 on the fixture's Plus XL model (`20GBX9901`).
The old test incorrectly used a Plus host and expected only three stacks. The
importer retains a supported foreign-model profile without changing the active
profile; that behavior was correct and required no production change.

The Trigger workflow now passes all eleven original adaptor records. The
"Galleon Default Windows" profile uses the distinct Galleon model `GRETSCH`,
with lower-row encoder coordinates preserved. Each original fixture is tested
on its declared model; foreign imports keep the active profile unchanged.

Verification update, 14 October 2026: the Galleon profile/model gap is resolved.
The complete Action Trigger and soundboard suites now pass on their fixtures'
declared device models. See [Galleon support and remaining USB work](GALLEON.md).
