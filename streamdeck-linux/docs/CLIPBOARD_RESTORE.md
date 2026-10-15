# Text clipboard restoration: reference and backend work

Optional clipboard restoration is not enabled in Deckforge yet. The current Text
paste action still replaces the clipboard. This work establishes the native
contract and tests a coherent Wayland capture backend before adding restoration.
The new backend is compiled only for tests; the installed application is unchanged.

## Original behavior

The supplied Stream Deck7.6 executable defines these global configuration keys:

- `text_action_restore_clipboard_enabled`, initialized at0x14011db40. The paste
  manager constructor at0x1403956b0 uses a false configuration fallback.
- `text_action_delay_ms_before_restore_clipboard`, initialized at0x14011dbb0.
  The same constructor uses250ms as the fallback delay.

ESDPasteManager0x140252390 retains the prior clipboard before a paste queue and
restores it after the queue drains when restoration is enabled. MIME clone helper
0x140373e00 copies HTML, colors, images, text and URLs, then any remaining advertised
formats. Restoring only plain text would lose part of that contract.

These are static observations, not execution of the Windows application or evidence
of a particular user's runtime settings. Private evidence is retained in
`../evidence/text-typing/clipboard-restore-analysis.json`. The executable SHA-256 is
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.

## Linux capture contract

`src/system_clipboard/mime_wayland.rs` reads every format from one Wayland offer.
It synchronizes and checks selection generation after transfers. If ownership
changes, it rejects the whole snapshot rather than returning mixed data. It never
changes the clipboard. Empty selection is distinct from reported transfer failures.
A provider that silently closes a transfer without data looks like a valid empty
payload. Generation checks detect selection changes, not a provider mutating its
content without replacing the offer.

Capture buffers remain in memory. Limits are32 formats,256 bytes per format name,
16MiB total and a3-second capture deadline. Local socket connection is bounded at
500ms, with a separate2-second setup bound. Transfers use nonblocking receive pipes
and check cancellation. A timed-out capture can recover using the same connection;
seat/manager failures remain latched. Only one unambiguous seat is accepted.
Inherited `WAYLAND_SOCKET` connections are rejected rather than sharing another
client's protocol stream. PRIMARY metadata is discarded and its contents are never
requested; an oversized PRIMARY offer does not invalidate the regular clipboard.

The backend prefers `ext-data-control` and includes a `wlr-data-control` fallback.
The owned KWin session advertises only the former. The ext backend has runtime
acceptance evidence; the wlr binding compiles but lacks runtime acceptance here.
X11 capture/restoration remains to be implemented.

## Evidence

`tests/clipboard_mime_reference.py` creates a private session bus, runtime directory
and virtual KWin compositor. Rust checks the compositor socket's peer PID/UID before
any clipboard access. No user clipboard, physical input or USB device is used.

The fixture verifies exact text, HTML, file-URI, PNG and custom binary MIME bytes
through a temporary Text write and reference restoration. It also proves that
independent `wl-clipboard-rs` reads can combine old text with new HTML even when the
format list is unchanged. The offer-scoped backend rejects that race and recovers
on its next capture. PRIMARY isolation, excessive format rejection, empty selection,
provider timeout, cancellation and recovery are covered. A stalled-provider case
pauses only the test's owned child and always cleans it up.

The reference writer uses `wl-clipboard-rs0.9.3`; it is not the production restore
implementation. That library's convenience API uses temporary files and separate
read connections. It is suitable for controlled fixture data but does not satisfy
the intended coherent, memory-only snapshot/restore lifecycle by itself. See its
[read APIs](https://docs.rs/wl-clipboard-rs/0.9.3/wl_clipboard_rs/paste/index.html) and
[prepared-copy lifecycle](https://docs.rs/wl-clipboard-rs/0.9.3/wl_clipboard_rs/copy/struct.PreparedCopy.html).

`target/clipboard-mime-reference/proof.json` records actual protocol availability,
source/test-binary hashes and verified cases. It explicitly records that production
restoration is disabled. The16MiB limit is implemented; an oversized payload stress
case remains to be added before production integration.

## Work remaining before enabling the setting

1. Add an owned MIME producer that serves from memory and reports ownership loss.
2. Integrate capture/restore asynchronously with Text paste, preserving responsive
   key releases. Failed capture must leave the clipboard untouched and suppress paste.
3. Restore only while Deckforge still owns the temporary paste selection; preserve a
   newer external copy. Consecutive Text pastes need queue-level restore behavior.
4. Add bounded transfer/shutdown behavior, X11 support and backend acceptance beyond
   this KWin session, including oversized payloads and failed providers.
5. Add the opt-in global setting and delay, default off, with saved-state and native
   UI tests. Do not rewrite existing profiles or store clipboard contents in sessions.
