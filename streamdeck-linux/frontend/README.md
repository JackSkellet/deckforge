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
