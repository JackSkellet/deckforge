# ChatGPT & Codex for Linux

A local adaptation of the user's installed Elgato `com.elgato.openai-codex`
plugin. The repository contains the patcher and Linux discovery helper, not the
vendor bundle. The original action IDs, layouts and property inspector remain.

The original plugin registers with Deckforge on Linux, but its desktop IPC
endpoint and CLI discovery explicitly handle only macOS and Windows. This patch:

- Connects to `$CODEX_HOME/ipc/ipc.sock`, defaulting to `~/.codex/ipc/ipc.sock`.
- Preserves the socket/directory ownership and permission checks.
- Finds and verifies the desktop app's bundled Linux Codex CLI, with a checked
  PATH fallback, for the original app-server transport.
- Opens existing `codex://` links through `xdg-open`.

The installer authenticates the original protected manifest and file hashes,
stages an explicitly labelled derivative, and keeps the whole original folder
under Deckforge's `plugin-backups`. No host integrity policy changes are needed.

```sh
python scripts/patch-codex-linux.py \
  "$HOME/.local/share/deckforge/plugins/com.elgato.openai-codex.sdPlugin" \
  --reference-app /path/to/original/StreamDeck.exe
```

Release dependencies for Deckforge must already be built, because the verification
helper reuses its protected-manifest reader. An identical second run makes no
changes. Restart the plugin after installing. Marketplace updates can replace
this adaptation; the patcher refuses unrecognized source patterns.

## Verified boundary

On 14 October 2026, the actual shipped transport with the Linux endpoint completed
its `initialize` handshake with the running Linux desktop app. An isolated real
Deckforge host loaded the adapted plugin, rendered Activity/Usage/Health actions,
and received a complete catalogue of 217 chats. Its built-in protocol fallback
successfully retried history loading with version 1 after version 2 was rejected.
The Linux CLI was found and passed its `codex-cli` version check.

Validation was read-only. It did not send prompts, create chats, approve commands,
change settings, commit/push code, or interrupt tasks. Those controls retain their
original app-supported protocol and are not claimed as individually verified.
Voice/dictation observation remains platform-specific and was reported unavailable.
Some capabilities depend on the running desktop app version; an unsupported
capability remains unavailable rather than being replaced with guessed behavior.

To roll back, disable the plugin, move the derivative aside, restore the printed
backup to `plugins/com.elgato.openai-codex.sdPlugin`, and enable it again. Saved
Deckforge assignments retain their original UUIDs.
