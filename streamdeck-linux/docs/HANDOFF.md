# Deckforge continuation handoff

The user asked to finish the animated-image work and stop the active goal because
this chat was too long. Do not resume automatic goal work without a new request.
The full Stream Deck desktop port is unfinished. The last rough engineering
estimate was 60–70% overall, with substantial uncertainty around native plugins.

## Project and user constraints

- The repository root contains the app in `streamdeck-linux`; run app commands there.
- Public repository: https://github.com/JackSkellet/deckforge
- Local branch: `codex/streamdeck-linux`; publish to verified origin `main`.
  Inspect/fetch current history before continuing; preserve user changes.
- Native Rust host and Wry/WebKit desktop shell, with an embedded TypeScript UI.
  Dark theme. Build independently; no existing Linux Stream Deck implementation
  as a starting point. No Wine. Rust ports for platform-only plugin backends.
- Use the installed `i-have-adhd` skill. Prefer simple solutions without losing
  functionality. Do not spawn agents unless newly authorized.
- GitHub CI stays manual-only. The user handles long reliability tests.
- Studio LED is skipped. Effects and screensaver completion are deferred.
  Sharing and Smart Profiles go last. Larger plugin and built-in action gaps
  are the next priority. Audio routing was physically accepted by the user.

## Latest delivered work

Folder auto-exit defaults/overrides, a countdown ring, confirmed override reset
and Back-key long-press to the root page are implemented. Key ownership guards
prevent stray releases from navigating. Native/process checks cover Counter,
export/restart, held input and adapted Back through an Action Trigger.
See `FOLDER_EXIT.md` for contracts and limits.

Animated GIF key artwork now works from user images, imported profiles, manifest
images and SDK file/data-URL writes. Animated `setImage` is explicitly requested
as a Deckforge extension even though the official SDK excludes animation.
Encoder `setImage` animates the editor icon separately from LCD feedback.
Targets, user precedence, state changes, original bytes and restart are covered.
See `ANIMATED_IMAGES.md` for timing, budgets and remaining boundaries.

The installed release SHA-256 is `173e85ee2fc937884f2242f430bac91fc2fd4a3dfabf9d4ce946485402301cd4`.
Installation preserves saved state, Discord credentials and the running app.
The user can reopen Deckforge when convenient to load it.

## Evidence and verification

Private evidence is under `../evidence/key-animation/`:
- `sdk-final.json`: unchanged SDK 2.0.4, animated files/data URLs, state/target
  isolation, encoder icons, original-byte export/import and restart.
- `native-final/report.json`: actual WebKit pixels change, Counter works while
  the image animates, and restart works after deleting the original image.
- `static-regression.json`: static PNG/JPEG/WebP and dial behavior still pass.
- `rea-bundle.json` and `rea-snapshot.json`: native decoder timing/loop evidence.

Additional targeted Rust checks cover timing, finite loops, GIF disposal,
frame-count rejection, hardware/editor frames, user priority and page cleanup.
Appearance, display-target, Wheel, Trigger and update-asset checks passed.
Strict all-target Clippy passed. These are simulated-device/native-window checks;
no USB acceptance was newly claimed. REA was exported and closed.

## Remaining scope

Read `FEATURE_PARITY.md`, but distinguish current rows from its historical
baseline sections. Some old OS/account/audio baseline wording is stale; Discord
and Marketplace auth and the native mixer have since been implemented.

1. Broader real plugin coverage and native ports for Windows/macOS-only backends.
2. Text entry, Hotkey Switch, application closing/activation, Key Logic and
   remaining dial/display behavior. Animated WebP/APNG and feedback pixmaps are
   still open, as is exact Windows image/state-reentry parity.
3. Full backup/restore, migrations, undo, system clipboard, icon packs and file
   associations. Windows acceptance of exported profiles still needs evidence.
4. Remaining transports, Mobile/virtual/MCP support, Network Dock, firmware
   updating and physical device checks beyond the tested Plus.
5. User-deferred sharing, Smart Profiles, effects and screensaver completion.

## Operational safeguards

Do not print or publish credentials. User state is in
`~/.local/share/deckforge/session.json`; Discord private auth is under
`~/.local/share/deckforge/plugin-data/`. Public Discord packaging uses an
allowlist and excludes connection tokens. Do not rerun live auth casually.

For an app update, build frontend before Rust because the frontend is embedded.
Use the existing targeted checks and reuse valid results. Install atomically with
`scripts/install-desktop.py --without-bundled-plugins`, compare state/auth hashes
and modes before/after, verify the installed binary hash and leave the running
app alone. `~/.local/bin/deckforge.previous` is the executable rollback.
