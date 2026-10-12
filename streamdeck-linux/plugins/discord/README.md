# Discord for Deckforge

An independent native Rust Discord plugin for Linux. No Wine or vendor executable
is used. The plugin retains the 17 original Elgato Discord action UUIDs and their
settings keys so imported profiles can resolve the same controls. Its backend,
settings panel and SVG artwork are new implementations.

Action icons use distinct microphone, headphones, channel, camera, screen-share,
soundboard, statistics and volume symbols. Automatic key-state alternation is
disabled. Mute/deafen and input-mode indicators follow observed Discord state;
voice-channel selection follows the reported channel. Push-to-talk/mute highlight
the local held gesture. Video, screen sharing and other actions without reliable
state readback retain neutral artwork rather than claiming that a command worked.

## Build and install

From the Deckforge application directory:

```sh
python scripts/build-discord-plugin.py
```

Import `target/release/discord-for-deckforge.streamDeckPlugin` through
**Plugins**. The package contains a native Linux ELF executable;
it is separate from the downloaded Elgato Marketplace package.

Select a Discord action and open **Plugin settings…**. Under **Application
credentials**, enter your own Discord application client ID and client secret,
and a registered redirect URI, then choose **Save and connect**. In the Discord
Developer Portal, first add a URL under **OAuth2 → Redirects** and save it. For
local use, `http://127.0.0.1/callback` is one option. Enter exactly the same URL in
the plugin. An empty portal redirect list can cause RPC error 5000 with a missing
`redirect_uri` detail before credentials are exchanged. The OAuth2 URL Generator's
scope checkboxes do not configure this plugin's requested permissions.

Discord requires RPC approval or tester access
for the application. The [official RPC guide](https://docs.discord.com/developers/topics/rpc)
describes authorization and restrictions. Elgato's private authorization service
and signing credentials are not reused.

The plugin stays disconnected until Connect is pressed. Once enabled, it resumes
its saved session after restart without opening a new consent prompt. Disconnect
persists until Connect is pressed again.
Credentials and tokens are saved in an owned mode-0600 file inside the host's
mode-0700 plugin data directory. They are excluded from action/global settings
and profile exports. **Disconnect** releases owned held controls before closing
the connection; **Forget private credentials** removes the local saved credentials.

## Current behavior

All 17 actions have audited property-inspector forms. Server, channel, user and
audio-device choices use names when Discord supplies a list, with custom-ID
fallbacks. Changing servers clears the channel choice; Disconnect is explicit.
Volume adjustments display signed percentages while preserving the original
profile encoding. Input volume is capped at 100%, output/user volume at 200%.
Actions without per-action settings say so; connection controls remain available.

The local inspector loads `ui/fields.js` as a script because native WebKit cannot
fetch the original `file://` JSON resource. `ui/fields.json` remains canonical;
after changing it, regenerate with `node plugins/discord/tests/generate-fields.cjs`
from the application directory. Packaging rejects a stale generated file.

Implemented request routes cover mute/deafen, voice/text channels, push-to-talk
and push-to-mute, audio-device selection, voice mode, local/user volume, soundboard,
video and screen sharing. Some commands are documented Discord RPC; the PTT,
video, screen-share and soundboard extensions were recovered from the supplied
original plugin. Permission errors remain visible rather than being treated as
success.

Notifications support unread counts, opening and cycling notifications, and
clearing the local queue. Server statistics track messages and recent authors;
online counts require the server's public widget. Additional permissions require
an explicit **Authorize required access** action in settings. Missing permissions
and failed subscriptions stay visible.

Local and user volume actions render live horizontal or vertical meters.
Voice-channel avatar layouts and long-press notification actions remain unfinished;
their imported settings are retained. These limitations are not exclusions from
the project goal.

Unit and mock integration tests exercise IPC framing, ping/reconnect, OAuth against
a local test server, command routing, held-control cleanup and private storage.
The actual Deckforge host installs and launches the Linux executable, discovers
all 17 actions, opens the native inspector and retains settings across restart.
The automated regression tests require no Discord account and send no real
Discord commands. A separate user-authorized live check on 12 October 2026
completed consent, token exchange, RPC authentication and a read-only voice
settings query through this Rust backend. Tokens remained in private storage.
Physical control actions and all 17 features still require live acceptance.

Authentication failures now identify the failing command and RPC code. Recognized
OAuth causes receive specific setup guidance; unknown reasons remain unknown.
The plugin never echoes arbitrary server error text, credentials or tokens.
