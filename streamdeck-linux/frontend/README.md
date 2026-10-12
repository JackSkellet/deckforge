# Deckforge desktop editor

The editor is written from scratch in TypeScript and React. Rust owns the device,
plugins, profiles, filesystem, audio and application lifetime. Wry embeds the
production bundle in the native desktop window; no development server or browser
is needed after building.

## Build and check

Use Node.js 20.19 or newer:

```sh
cd streamdeck-linux/frontend
npm ci
npm run build
npm test
npm run test:e2e
```

The local browser checks use `/usr/bin/chromium`; set `CHROMIUM_PATH` for a
different executable. They cover desktop and short-window layouts, keyboard
assignment and dialog focus against the isolated fixture. They do not run in
GitHub CI.

`dist/index.html` contains the compiled JavaScript and CSS. The Rust build embeds
that generated file. Dependencies are locked in `package-lock.json`; no app
starter or Stream Deck implementation was used.

For isolated visual development:

```sh
npm run dev -- --port 5191
```

Open `http://127.0.0.1:5191/?fixture=1` for an explicit synthetic fixture. It does
not access devices, files or plugins. The fixture import is removed from
production builds. Opening the production editor without its native bridge
shows a connection error and does not fall back to the diagnostic HTTP editor.

## Native bridge

`window.deckforge.request(method, payload)` returns a Promise and rejects with a
readable error. `window.deckforge.onSnapshot(callback)` returns an unsubscribe
function. The frontend uses:

- `snapshot`: the existing Rust Controller snapshot.
- `command`: the existing typed host command payload, including explicit settings
  saves, profile operations and opening a plugin's separate property inspector.
- `action_icon {uuid, generation}`: a bounded, host-resolved icon data URL.
- `pick_file {kind}`: `plugin`, `profile`, `image`, `audio`, `file`, `directory`, or
  `export_profile`; returns an absolute path or null on cancellation.
- `window {action}`: hide the window or quit the application.
- `startup_status` and `startup_set {enabled}`.
- `ui_state_get` and `ui_state_set {state}`: opaque process-memory editor drafts,
  selection, filters and unapplied advanced JSON text, retained when the native
  window is destroyed and recreated.

The frontend emits `deckforge-ready` after its first snapshot is rendered, for
app-owned capture. It never embeds plugin HTML in the editor's privileged webview.

## Editing behavior

Live snapshots update clean drafts. Unsaved drafts remain attached to their
original action context when selections change or a plugin updates settings.
Explicit saves preserve unknown settings and imported action fields; sequence
editing preserves other branches and extra trigger positions. Changes made while
a save is in flight remain dirty. Read-only plugin profiles can be duplicated.

Keys and dials support drag assignment and an equivalent keyboard-accessible
Assign button. Plugin inspectors open separately through the native host. Native
HTML dialogs retain focus, forms use labels, and reduced-motion preferences are
respected. Tests cover these contracts and command payloads independently of
hardware. Actual hardware rendering and native window integration must also be
validated through the Rust application.

## Audio mixer

The mixer panel reads `snapshot.mixer` and requests `mixer_refresh` when opened.
This only enumerates audio devices and playing applications. Editing inputs,
outputs, volumes, mutes, routes or the enable switch changes a retained local
draft. **Apply routes** sends `mixer_config` with the full `settings` and the
original `expected` configuration. External changes leave the draft intact and
require an explicit reload; object-key ordering does not count as a change.

The input selectors store application IDs/process names and exact source names.
Transient stream indexes are never saved. Owned mixer devices are excluded from
assignment, and unavailable saved devices remain visible until removed. Monitor
and Stream mixes have independent route gains, mute states and master controls.
The panel displays backend connection, active/pending status and warnings; it
does not invent audio meters or claim official Wave Link compatibility.

The two native Deckforge mixer actions have structured target and gesture
settings. Those controls only edit the action; they cannot create or enable
routes. Actions with `user_title_enabled: false` show a read-only live title and
never send an appearance override during Save. Mixer tests use synthetic
inventory and command stubs, without changing the user's audio graph.

## Action editing

The selected control's **Edit** menu exposes Copy, Cut, Paste and Remove.
Right-click a canvas control to open that menu at the pointer, or focus the
control and press Shift+F10 / Menu. Text inputs keep their native context menus.
`Ctrl+C`, `Ctrl+X`, `Ctrl+V` and `Delete` do the same outside text editors and
modal dialogs. Escape cancels a pending cut when no text editor or menu owns it.
The cut source remains assigned until a successful paste, and is marked on the
canvas. Copying uses the saved action state; unsaved source or destination drafts
must be saved or reverted first.

Drag an assigned key/dial to move it; hold Ctrl to copy. Moving onto an occupied
control swaps the actions when both support their new control. Copy/paste-copy
replaces the destination. The native host owns the process clipboard and all
nested action/resource transfer. Every destination request includes its observed
`expected_target` context (or null for an empty control), so a stale UI cannot
silently replace a different action. Read-only actions can be copied; they cannot
be cut or removed. The explicit development fixture models these interactions
without touching real profiles.

## Appearance drafts

The Appearance editor reads `slot.appearance` and `slot.state_count`. It keeps
independent edits for each target state, while `expected_state` guards the live
action state. Selecting another appearance state only requests a native preview;
it never flips the physical action. If the live state changes during editing,
Save stops until **Refresh state and preview** explicitly accepts the new live
state while retaining the targeted drafts.

`appearance_preview` sends the same context, state guard and changed appearance
fields to Rust, without issuing a host command. Requests are debounced, and
replies for older edits or contexts are ignored. The result supplies PNG pixels,
resolved title/style, custom-override flags and optional font fallback details.
`fonts` returns the native font-family inventory. Empty/default and unavailable
saved families remain selectable and are not rewritten on display. The browser
fixture deliberately does not pretend to reproduce Rust's font or image renderer.

Title, style, icon, settings and container edits carry separate intent. A
settings-only save cannot create a user title override, and an appearance-only
save cannot overwrite a plugin's live settings. Style payloads contain only
changed fields; the host merges them onto the selected state's resolved style.
Explicit title/icon resets are distinct from choosing a replacement. Unknown
settings and other state records remain untouched.

## Device geometry

The canvas consumes Rust's `device_model`, `device_name`, `device_geometry` and
`device_capabilities` snapshot fields. Key/dial counts, grid columns, image
capabilities and available action filters come from that descriptor. Restored
selections cannot leave an empty out-of-range physical control selected.

Neo's infobar is a configurable `Neo` control at index 1; it supports compatible
actions, settings, appearance and copying, but has no input-test button. Its two
sensor controls navigate pages. Pedal displays software assignment labels rather
than pretending to have key LCDs. Foreign-model profiles stay listed with their
required device name and cannot activate on incompatible hardware. Out-of-layout
records remain explicitly visible for inspection/export; no projection silently
replaces their saved positions.

The isolated browser fixture accepts `&model=mini`, `classic`, `classic-2017`, `xl`, `plus`,
`plus-xl`, `neo`, or `pedal`; `&overflow=1` retains its deliberately mismatched
assignments for inspection. These fixtures and unit tests are software evidence,
not a claim that every physical device has been tested.

Neo sensor button availability follows the host’s `page_navigation.previous` and
`page_navigation.next` flags, including folder contexts. Older snapshots without
those flags keep the previous enabled-button behavior. The synthetic fixture
models first/last-page boundaries without wrapping.
