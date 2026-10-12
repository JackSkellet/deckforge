# Plugin browser links and callbacks

Registered plugins and property inspectors can send the documented contextless
`openUrl` request. Deckforge passes HTTP/HTTPS URLs to the default desktop
browser through `xdg-open`, without a shell. The request is asynchronous and
does not generate a new SDK acknowledgement event. Custom schemes are not
supported by this command. Up to eight launchers may remain active; the host
does not wait for a browser window to close or kill it on shutdown.

The native application also handles:

```text
streamdeck://plugins/message/com.example.plugin/auth?code=example&state=example
```

Only the named installed plugin receives:

```json
{"event":"didReceiveDeepLink","payload":{"url":"/auth?code=example&state=example"}}
```

The encoded suffix retains its leading slash, query, repeated parameters and
fragment. Query-only and fragment-only suffixes are supported. The host does
not decode and rebuild the suffix; the SDK can parse it into its own URL view.
Plugin identifiers match case-insensitively. An empty suffix sends no event;
a trailing slash is nonempty. The accepted route uses an ASCII plugin identifier;
percent-encoded identifiers and unrelated routes are rejected.

Active links show the editor. A query item named `streamdeck` whose value is
`hidden`, case-insensitively, suppresses foreground activation. That query item
remains in the plugin's payload. A running app receives the link through its
private, state-scoped IPC endpoint. A cold launch restores the saved session
and queues the callback until the target plugin registers. A slow or malformed
endpoint is an error, never a reason to start a second instance.

Callbacks awaiting registration stay only in memory, with a five-minute expiry,
eight messages per plugin and 64 messages overall. They are flushed after
registration and visible-action replay. They are not saved in profiles or session
files and do not appear in host snapshots or activity messages. Accepted messages
are sent once; the one-way SDK protocol cannot confirm that a plugin processed
them. Unknown plugins produce a sanitized error. Active failures remain visible
in the native app; passive failures do not foreground it.

## Desktop registration

The per-user installer advertises `x-scheme-handler/streamdeck` in the desktop
entry and adds the `%u` URI argument. It sets Deckforge as default only when no
handler exists or Deckforge already owns the association. Other defaults are
preserved. It reports registration failures without replacing unrelated settings.

You can also deliver a link directly:

```sh
deckforge 'streamdeck://plugins/message/com.example.plugin/auth?code=example'
deckforge desktop --state /path/to/session --uri 'streamdeck://plugins/message/com.example.plugin/auth?streamdeck=hidden'
```

The legacy egui editor does not accept callbacks; use the default native editor.
These features provide URL transport, not an OAuth provider or authorization
grant. Individual plugins still own their authorization/state validation.

## Evidence and validation

The contract follows Elgato's [system guide](https://docs.elgato.com/streamdeck/sdk/guides/system/),
[plugin WebSocket reference](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/)
and [property-inspector reference](https://docs.elgato.com/streamdeck/sdk/references/websocket/ui/).
Static inspection of the pinned 7.6 executable corroborates suffix slicing,
empty-message suppression, case-insensitive plugin matching and passive query
handling. Local records are in `evidence/plugin-system-contracts/`; no proprietary
binary or decompilation is distributed with Deckforge.

`tests/plugin_urls_interop.py` calls the unchanged official SDK's `openUrl` and
the real native property inspector against a test-owned opener. It checks exact
arguments, unsupported schemes and credential omission from host logs/snapshots.
`tests/native_deep_link_interop.py` covers cold-start queuing, warm handoff, real
GIO desktop-entry expansion, preserved suffixes, active/passive windows and
unknown-plugin errors. The instance tests cover ownership, malformed requests,
size limits and timeout behavior; installer tests preserve other URI handlers.
All fixtures use isolated state and avoid real authorization pages and USB.

Application monitoring, computer-resume events and plugin localization remain
separate compatibility work.
