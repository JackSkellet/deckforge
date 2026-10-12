# Discord for Deckforge

An independent native Rust Discord plugin for Linux. No Wine or vendor executable
is used. The plugin retains the 17 original Elgato Discord action UUIDs and their
settings keys so imported profiles can resolve the same controls. Its backend,
settings panel and SVG artwork are new implementations.

## Build and install

From the Deckforge application directory:

```sh
python scripts/build-discord-plugin.py
```

Import `target/release/discord-for-deckforge.streamDeckPlugin` through
**Plugins & profiles**. The package contains a native Linux ELF executable;
it is separate from the downloaded Elgato Marketplace package.

Select a Discord action and open **Plugin settings…**. Under **Application
credentials**, enter your own Discord application client ID and client secret,
then choose **Save and connect**. Discord requires RPC approval or tester access
for the application. The [official RPC guide](https://docs.discord.com/developers/topics/rpc)
describes authorization and restrictions. Elgato's private authorization service
and signing credentials are not reused.

The plugin stays disconnected until Connect is pressed, including after restart.
Credentials and tokens are saved in an owned mode-0600 file inside the host's
mode-0700 plugin data directory. They are excluded from action/global settings
and profile exports. **Disconnect** releases owned held controls before closing
the connection; **Forget private credentials** removes the local saved credentials.

## Current behavior

Implemented request routes cover mute/deafen, voice/text channels, push-to-talk
and push-to-mute, audio-device selection, voice mode, local/user volume, soundboard,
video and screen sharing. Some commands are documented Discord RPC; the PTT,
video, screen-share and soundboard extensions were recovered from the supplied
original plugin. Permission errors remain visible rather than being treated as
success.

Notifications and server statistics are still being implemented. Imported dynamic
icon settings are retained, but the current artwork is static. These limitations
are not exclusions from the project goal.

Unit and mock integration tests exercise IPC framing, ping/reconnect, OAuth against
a local test server, command routing, held-control cleanup and private storage.
The actual Deckforge host installs and launches the Linux executable, discovers
all 17 actions, opens the native inspector and retains settings across restart.
Live Discord authorization and real control behavior have not yet been verified.
No test requires an actual Discord account or sends a real Discord command.
