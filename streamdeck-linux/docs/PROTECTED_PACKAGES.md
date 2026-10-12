# Protected plugin packages

Some Marketplace plugins store `manifest.json` in an Elgato binary envelope.
Deckforge can authenticate the supported envelope and check its required files
before installing the package. It needs a local copy of the original Stream Deck
**7.6.0.23012** executable as a static reference. The reference is hash-pinned and
never executed. Vendor key material is not embedded in Deckforge or published
with the source.

In **Plugins → Protected Elgato packages**, choose the reference executable and
save it before importing the package. The command-line installer also accepts:

```sh
deckforge install example.streamDeckPlugin --destination /path/to/plugins \
  --reference-app /path/to/original/StreamDeck.exe
```

Protected manifests remain unchanged on disk. Loading or restarting a protected
plugin requires that reference and checks required files again. Missing files,
wrong digests, unsafe paths, symlinks and special files are rejected. A failed
archive check leaves no partial installation. Existing plugin folders are never
overwritten by the installer.

## Verified contract

Static analysis of the supplied Windows executable identifies the file checksum
as BLAKE3 with a 32-byte result compared to the Base64 `Hash` field. The original
caller checks entries whose `Required` flag is true. Deckforge preserves this
policy; optional entries are not an installation or loading integrity gate.
The supported protected format must have a `Files` array, and hashes must use
standard padded Base64. Other reference versions and unverified formats fail
explicitly rather than silently skipping verification.

The supplied Discord 2.4.0.123 package contains 179 listed files; all match the
recovered checksum algorithm, and 54 are required by the original manifest.
Tests cover a valid inert installation, missing and changed required members,
optional-member changes, and required-file revalidation after installation.
These tests never execute the vendor backend.

## Runtime compatibility

Successful package verification does not translate a Windows or macOS executable
into a Linux program. Such backends remain visible as unsupported. Deckforge's
independent Rust Discord replacement runs natively; importing the original package
does not replace an already installed plugin with the same identity.

Native Linux executables and supported portable scripts use the existing plugin
runtime. No Wine compatibility layer is used.
