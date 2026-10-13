# Plugin inspector file controls

Visible native property inspectors now use a GTK file chooser with the input's
file-type filter and single/multiple-selection setting. The selected input's
`value` exposes its full Linux path with URI escaping, which lets unchanged
`sdpi-file` controls decode and save it. Ordinary browser `C:\fakepath` values
otherwise leave plugins with only a filename. The native `FileList` and File
reading APIs still come from WebKit's actual file selection.

The adapter is injected only for a visible inspector's private plugin scheme
and host. Same-origin local iframes and shadow-DOM inputs use the same selection
tracking. Remote or opaque frames do not receive the adapter. Each chooser has
an independent request ID; navigation or a removed input cancels completion.
Only the native chooser supplies selected paths. No arbitrary file-read bridge
is added. Cancelling preserves the existing selection; resets, clearing a value,
and assigning a new FileList clear the recorded path.

The expected full-path value is documented by the [SDPI file component](https://sdpi-components.dev/docs/components/file).
The implementation uses WebKitGTK's FileChooserRequest API and requires WebKitGTK
4.1 version 2.40 or later, alongside the existing GTK3 dependency. It does not
rewrite a plugin's files or replace its controls with Deckforge forms.

## Verification

Run the explicit native test with a desktop session:

```sh
cargo test native_file_controls -- --ignored --nocapture
```

It opens a temporary native WebKit window and uses the retained, unchanged
SDPI Components file from the official Data Sources sample, SHA-256
`f6c0dfd2ed68e18084b9952842b86e3850cf837d674704700c2a0718e0a24f6b`.
A test picker supplies temporary paths through the real WebKit file-chooser
request; this does not claim manual acceptance of the system chooser dialog.
The test verifies native change/cancel events, full paths in ordinary and SDPI
inputs, readable file contents, multiple selection, accept-filter propagation,
Unicode/space/percent/hash filenames, clear/reset, programmatic FileList
replacement, local iframes, unrelated-frame isolation and stale request IDs.
The files, browser profile and native window are isolated from real user state.

Existing native HTML/inspector SDK messaging tests cover the subsequent settings
transport. A live third-party action consuming a selected path still needs its
own functional acceptance. Directory-selection inputs, filesystem paths on
native drag-and-drop, and file controls in separately opened popup windows
remain unverified; they are not implied by this chooser test.
