# Plugin compatibility checkpoints

Deckforge keeps plugins that fail to start visible in the native plugin manager.
A bad plugin does not prevent the editor or other plugins from starting. Restart
reloads one plugin while retaining configured actions and settings. It does not
convert Windows or macOS binaries into Linux executables.

## Discord 2.4.0.123

The user supplied `discord-2.4.0.123-mac_windows.streamDeckPlugin` on 12 October
2026 as a real compatibility target. Package SHA-256:
`9d063e9b3616693093e565f762cdc65bd5638bdbb9f42e535e1c738b429302cd`.

The ZIP contains `com.elgato.discord.sdPlugin`, Windows `ESDDiscord.exe`, macOS
`ESDDiscord`, and HTML/JavaScript property inspectors. Its 32,512-byte manifest
starts with an `ELGATO` binary header rather than JSON. Manifest SHA-256:
`b12534e6177cd673e60d58652f29194a03acdd9d85b44c5d204554646b4b6583`.

Elgato's [distribution documentation](https://docs.elgato.com/streamdeck/sdk/introduction/distribution/)
describes Marketplace DRM, manifest encryption and integrity checking. The new
Rust reader authenticates and decrypts this manifest using a locally supplied,
hash-pinned Stream Deck 7.6.0.23012 executable as its reference. It verifies the
trusted signer, application identity, Ed25519 signature and AES-GCM tags. No vendor
keys or decrypted manifest files are embedded in or written by Deckforge.

The unchanged package's authenticated manifest SHA-256 is
`038c015493d11a665e7c561b993afc6f6933ad9db0ec926e1d0c6e50c33bae25`.
It declares 17 actions and 179 file-integrity records. The latter hash algorithm
remains unresolved, so the installer still refuses the protected package without
creating partial files. Manifest authentication alone is not package validation.

At the user's request, Discord execution uses a new [native Rust plugin](../plugins/discord/README.md).
It preserves the original action/settings identities and supplies its own settings
panel and artwork. No Wine or original Windows/macOS backend is used. The native
replacement has host/process and mock-RPC evidence; actual Discord authorization
and live controls remain unverified.

## Remaining runtime work

- Protected Marketplace packages and their validation contract.
- Windows/macOS executable plugins and OS-specific native dependencies.
- Broader runtime/version selection and updates. Portable generic scripts and unambiguous Windows/macOS script entries now work, including `.cjs`; Node paths are searched deterministically.
- Enable/disable, replacement/rollback and uninstall with retained profile data.
