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
describes Marketplace DRM, manifest encryption and integrity checking. Protected
package verification/decryption is not implemented in Deckforge. The installer
recognizes this header and reports that limitation before creating an installation
directory. Original archive bytes remain unchanged. Tests use an independently
constructed header fixture; the vendor package is not redistributed here.

The actual package import was attempted locally and failed at manifest decoding.
No plugin process or Discord operation ran. Native executable support is a second
unresolved requirement after protected-package import. Showing these errors is
not a claim that the Discord plugin is supported.

## Remaining runtime work

- Protected Marketplace packages and their validation contract.
- Windows/macOS executable plugins and OS-specific native dependencies.
- Platform-specific entry paths, explicit Node runtime selection and updates.
- Enable/disable, replacement/rollback and uninstall with retained profile data.
