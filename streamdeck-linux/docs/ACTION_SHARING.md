# Action sharing

The selected control's Edit menu has Import Action and Export Action. Export
includes one root action, its nested actions, referenced folder pages and media.
Import replaces the selected control using the existing copy/paste transaction;
it creates independent action/page identities and does not add a profile or
replace the clipboard. Read-only profiles can export but cannot import.

## Native contract

Read-only REA analysis of the supplied Stream Deck 7.6 Windows AppExecutable,
SHA-256 `2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`:

- `0x14085f280`: Export Action dialog, `.streamDeckAction` extension, selected
  action address passed to the exporter.
- `0x1403c8170`: collect action stashes, create a temporary device `$storage`
  with model `Fake/Storage`, create an initial profile, insert actions with a
  separate increasing column counter starting at `0,0` for each controller.
- `0x1403c8c50`: common profile/resource packer; stage `Profiles` and `Resources`
  under a temporary Bundle directory, write package.json, ZIP to destination.
- `0x140436110` / `0x140436730`: package descriptor serialization/deserialization:
  FormatVersion, AppVersion, OSType, OSVersion, DeviceModel, DeviceSettings,
  RequiredPlugins. The previously unidentified JSON field is DeviceSettings.
- `0x140433320` / `0x140434030`: bundle reader distinguishes version-1 wrapped
  packages from legacy `.sdProfile/manifest.json` trees and unpacks them.
- `0x1403bf260` / `0x1404179a0`: load extracted profile storage and collect root
  action stashes. Empty action data is rejected.
- `0x14085e620` / `0x1401812e0`: Import Action dialog and profile-manager dispatch.

Evidence bundle: `../evidence/action-sharing/rea-bundle.json`. These are static
contracts, not proof that the official Windows/macOS UI accepts our output.

## Linux implementation

`src/host/transfer/sharing.rs` constructs a temporary Profile in memory and uses
the existing exporter. It writes `Profiles/<id>.sdProfile/manifest.json`, page
manifests, assets and package.json with FormatVersion 1 and Fake/Storage. The
descriptor reports Deckforge's actual version and Linux OS, not a fabricated
Stream Deck version. RequiredPlugins includes nested action dependencies.

Neo's infobar uses `0,0` in shared storage and `1,0` in the managed Neo layout.
Imported roots are positioned at the chosen control. Known actions must support
that controller; unavailable actions retain their original controller type.

Import reuses the bounded ZIP/profile parser (10,000 files, 32 MiB/member,
128 MiB total). The staged source archive lives under the private state sources
directory so artwork and resources survive moving/deleting the chosen file.
Preparation does not publish a temporary profile or send plugin events. The
normal paste transaction validates target ownership and persists before replaying
plugin lifecycle events. Failed unpublished imports remove their staged archive.

The UI captures device, profile, page and target context before opening the file
picker. A changed destination is rejected. Unsaved selected-action drafts block
sharing until saved/reverted. Cancel does nothing. Export uses the same validated
atomic ZIP writer and existing-output backup as profile export, and rejects
overwriting managed source archives.

## Verification and remaining scope

Rust tests cover independent identities, unknown metadata, cropped encoder art,
cross-controller import, retained clipboard, folder page remapping, nested Multi
Action resources after deleting the original resource, Neo addressing, rejection
of multiple roots, stale targets/pages and persistence failure with cleanup.
Existing transfer, profile import and export tests exercise the reused paths.

Frontend tests change devices while the native picker is pending and verify the
original action/profile/page owner. `tests/native_action_sharing_interop.py`
checks real WebKit menu availability/bounds and host import/export, clipboard
retention, app restart and re-exported PNG bytes after deleting the original
files. Frontend tests separately check dispatch and picker cancellation with a
mock bridge. Native OS dialog operation is not automated. The fixture uses no USB.

Files containing multiple roots now open an explicit chooser. This works for
canvas import, native drops, and warm/cold desktop file opening. Filtering and
controller checks expose one chosen root, including its referenced folder tree.
Other roots are not imported. Source contents are hashed; changing the file after
opening the chooser rejects the import. Captured destination guards, cancellation,
native-view recreation, Undo/Redo, clipboard preservation and restart after deleting
inputs are covered by `tests/native_action_choice_interop.py` and focused tests.
Single-root files keep their direct path. A call without a selection still rejects
multiple roots. Fake/Storage folder cells must fit the destination layout.

Additional native evidence found that the original app's import operation chooses
the first stash from a vector, then imports that single stash to canvas or clipboard:
`0x140329b50`, `0x140329f30`, `0x1403bfc50`, `0x1403c2ef0`. Its first-root order is
not established. The Linux chooser is an explicit extension, not an assertion of
vendor batch placement. Evidence: `../evidence/action-choice/native-analysis.json`.

General multi-selection placement, OS clipboard action-graph interoperability,
original-app acceptance and broader hardware checks remain open. File drop/MIME
and session Undo/Redo now have their own verified subsets in FILE_OPEN.md and
UNDO_REDO.md. This does not complete UI-04 or PR-06.
