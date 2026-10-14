# Spotify for Linux

This is a local adaptation of the user's installed Elgato Spotify 2.5 plugin.
The repository contains a Linux controller and a guarded installer, not the vendor
bundle. Original actions, images, inspectors, settings and Spotify account UI stay
in the installed copy. The adaptation is visibly named **Spotify for Linux** in the manifest and all
root localization overlays; account and inspector translations are preserved.

The original plugin crashes at startup because its data-directory selection only
handles Windows and macOS. Its player router also only implements those systems.
The adaptation adds the Linux directory and router branches, using the session
D-Bus MPRIS interface through `busctl`. It requires no Windows runtime or new Node
dependencies.

## Supported local controls

Play, pause, toggle, stop, next/previous, volume, shuffle, repeat, seeking, opening
Spotify URIs, and current-track metadata/artwork use the local Spotify MPRIS
service. The existing plugin polling loop updates the deck. Spotify must be
running and have playable content. `xdg-open spotify:` handles launch.

MPRIS does not supply a user's saved library, liking tracks, Spotify search,
Spotify Connect device enumeration/transfer, or the cloud playback queue. These
remain the original plugin's Spotify Web API features and require its account
sign-in flow and any applicable Spotify account/API permissions. This adaptation
does not bypass those requirements. Account sign-in and cloud features have not
been verified here. The vendor's developer clipboard tool still assumes
Windows/macOS. Hidden legacy repeat action aliases remain platform-specific;
the current Repeat action is supported.

## Install

Build Deckforge's release dependencies first. The installer uses its existing
protected-manifest reader in a temporary Rust helper, authenticates the original
manifest against the user's original application reference, and verifies every
required file hash before creating the local derivative.

```sh
python scripts/patch-spotify-linux.py \
  "$HOME/.local/share/deckforge/plugins/com.elgato.spotify.sdPlugin" \
  --reference-app /path/to/original-stream-deck-reference
```

The entire original directory moves into the sibling `plugin-backups` directory.
The derivative has a plain manifest with the original action declarations, an
explicit Linux identity, and no vendor integrity/Marketplace table. Deckforge's
protected-package validation is unchanged. The installer refuses unfamiliar
bundle layouts and is idempotent for an identical adaptation.

Restart **Spotify for Linux** in the plugin panel after installation. A vendor
package update replaces this adaptation; re-run the installer against the newly
installed original only if its guarded patterns still match.

## Verification

```sh
python tests/spotify_linux_interop.py
python tests/spotify_linux_interop.py \
  --plugin "$HOME/.local/share/deckforge/plugins/com.elgato.spotify.sdPlugin"
```

The tests use a private real D-Bus service, never the user's Spotify playback. They
verify metadata conversion, playback, volume clamping, shuffle, repeat, negative
seeking, URI dispatch and absent-player behavior. With `--plugin`, the real host
runs in USB simulation mode and its original Spotify Playback key action reaches
the private MPRIS service through actual key-down/key-up events.

Live read-only checks also found Spotify's MPRIS service and read its stopped
state/volume. Actual music playback, account sign-in, and physical key use remain
separate user checks.

## Rollback

Disable the plugin, move the derivative directory aside, then restore the entire
original directory from the printed `plugin-backups/spotify-...` location to
`plugins/com.elgato.spotify.sdPlugin`. Enable/restart it. This restores the vendor
package exactly, including its authenticated manifest; the original package will
again lack Linux support. Existing deck assignments and host settings are not
changed by the installer.
