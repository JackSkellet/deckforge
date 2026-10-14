# Opening Stream Deck files from Linux

Deckforge accepts local `.streamDeckAction`, `.streamDeckProfile`,
`.streamDeckPlugin` and `.streamDeckIconPack` files from its desktop launcher. Action files populate the action clipboard; select a control
and paste. They do not overwrite the selected control or create a profile.
Profile files use the existing importer and its device-model compatibility rules.
Plugin files use the existing validated installer and normal runtime loading;
unsupported native backends remain visible in Plugins. Icon packs install as inert
files and become available in Appearance -> Icon library. Package files do not
replace a canvas control. Duplicate installs fail without overwriting existing
packages; managed plugin updates remain available through Plugins -> Update.

The same entry point accepts a local path or percent-encoded `file://` URI:

```sh
deckforge /path/to/Shared.streamDeckAction
deckforge desktop --open /path/to/Profile.streamDeckProfile
```

Existing private instance IPC queues the file in the running native editor and
shows its window. The one-second IPC exchange acknowledges enqueueing, not
completion of archive parsing. Parsing runs in the existing bounded editor worker
queue; success/errors are displayed through a uniquely identified file result so
an older UI notice cannot hide them. A missing instance starts normally and opens
the file. Failed imports keep the previous action clipboard. Existing drafts are
retained when the native window is reopened. No source file is modified. Clipboard-only staged archives are removed when
replaced/cleared or after a successful shutdown save; archives referenced by
pasted actions remain available.

The supplied Windows executable has an explicit
`ESDProfileManager::importActionToClipboard` path at `0x140181110`, separate from
`importActionToCanvas`. It queues the shared action-import operation with an empty
destination and invalid coordinates. This supports the clipboard-first desktop
behavior. Static evidence is in `../evidence/file-open/rea-bundle.json` (134 records);
it does not establish official-app acceptance of generated archives.

`src/file_open.rs` accepts only local regular supported files, rejects remote
schemes/hosts, queries/fragments and control characters (including percent-encoded
controls after URI decoding), and canonicalizes paths. `host/file_open.rs` reuses
Controller install paths for global packages and device-scoped action/profile
imports. Failed plugin archive validation preserves Undo history when no plugin
was added.
The existing bounded ZIP parser remains authoritative. Leaf encoder actions can
load into the clipboard while a Mini is selected, then be pasted into a compatible
control; folder layouts still must fit the staging device geometry. Existing
single-root action-sharing restrictions remain explicit.

`assets/deckforge-mime.xml` declares `application/x-streamdeck-action` and
`application/x-streamdeck-profile`, `application/x-streamdeck-plugin` and
`application/x-streamdeck-iconpack` as ZIP subtypes. The installer updates the MIME
database before desktop caches, advertises all four types in the desktop entry, and
registers defaults only when none exists. Other applications' defaults and the
existing Stream Deck URI handler are preserved. The distribution includes this XML.

Verification: isolated GIO MIME detection, mixed-case extensions, preserved
default-handler mock checks, IPC routing tests and CLI/path tests. The real native
test uses `gio launch` with the actual desktop Exec shape and a filename containing
spaces, `%` and `#`. It verifies same owner PID, hidden-window reopening, an unsaved
draft surviving action import/paste, a bad archive displaying an error without
replacing the clipboard/draft, and cold-start profile import. All runtime fixtures
use private state and simulation, with no USB. Reports: `target/file-open/`.

## File drops

The main WebKit view now installs Wry's native drag/drop handler. An action file
dropped over a key/dial/Infobar imports into that control using the existing
transaction. Hover outlines the target; leaving clears it. An action dropped
outside the canvas loads the clipboard. A profile dropped anywhere imports through
the existing profile path. Plugin and icon-pack drops use the global installer,
even over a key, retaining pending action drafts and never highlighting that key
as a replacement target. One supported file is accepted per drop; multiple and
unsupported files produce an error without partial imports or page navigation.

`desktop_file_drop.rs` normalizes Wry0.57 GTK signals. Its GTK implementation passes
widget logical coordinates directly; normalization divides by WebKit page zoom,
not the display scale factor. Paths must be absolute UTF-8, bounded and free of
control characters. No archive reads occur on hover. Events from old window
generations are ignored. The main view blocks the native default file-drop action;
plugin inspector views are unchanged.

`frontend/src/fileDrop.ts` hit-tests the actual DOM control. The handler captures
device/profile/page/target context at drop time, rejects readonly or dirty target
actions and blocks drops while a dialog or another edit is active. Profile and
background imports preserve the selected device captured by the event and leave
other drafts alone. Existing internal HTML action/slot drag handlers remain in
place. This is an implementation statement, not physical-gesture acceptance.

Verification: Rust event/path/zoom normalization and frontend hit-test, captured
ownership, readonly/draft tests. `tests/native_file_drop_interop.py` injects the
same notification into real WebKit, uses actual archive files and host imports,
and verifies target hover/leave, destination import, unchanged clipboard, dirty
target rejection, bad/multiple files, dialog blocking, background clipboard,
profile import, draft preservation and restart after deleting original files.
Report/captures: `target/file-drop/native/`. The native test uses synthetic events;
it does not automate the physical OS/file-manager drag gesture or establish
compositor-specific behavior. No USB is used.

`tests/native_package_open_interop.py` covers both package kinds in three modes:
warm `gio launch` with the actual desktop Exec shape, cold startup with --open,
and synthetic native drop notification over a dirty control. Warm runs verify
same owner PID and hidden-window reopening. All cases reject duplicate/bad
packages without changing assignments/history; the installed unchanged Counter
plugin retains its PID. Source archives include literal spaces, percent and hash
characters. Tests use the original Counter sample and Color Logo pack in private
simulated state. Captures/reports: `target/package-open/native/`. Original native
action/profile opening regression also passes after moving the dispatcher.

Physical file-drop acceptance, multi-root action files, update-on-open review,
official Windows/macOS runtime acceptance and broader parity remain open. A full restart loads file-opening/drop changes into an older running host.
