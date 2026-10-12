# Protected plugin packages

Some Marketplace plugins store `manifest.json` in an Elgato binary envelope.
Deckforge can authenticate the supported envelope and check its required files
before installing the package. It needs a local copy of the original Stream Deck
**7.6.0.23012** Windows or macOS executable as a static reference. The reference is hash-pinned and
never executed. Vendor key material is not embedded in Deckforge or published
with the source.

In **Plugins → Protected Elgato packages**, click **Import Elgato installer**
and select `Stream_Deck_7.6.0.23012.pkg`. Deckforge extracts the application,
validates its identity, and saves a private managed copy. No Elgato login is needed
for local manifest verification. Marketplace downloads and account entitlements
are separate from this step.

The importer requires `bsdtar` from libarchive. It reads only the expected
application member through two stdout pipes; it never installs the package or
runs its scripts or executable. Extraction has a 30-second timeout, a 512 MiB
input limit and a 64 MiB application limit. The managed copy is mode 0600 inside
a mode 0700 `references` directory beside `session.json`. Reimporting the same
application reuses that copy. Failed imports preserve the configured reference.
Other application versions are rejected until their identity and format are
verified.

**Advanced: use an extracted executable** retains the original manual path
option. **Clear reference** removes the saved selection without deleting the
cached file. The command-line package installer also accepts:

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
The supplied macOS installer also passes the real extraction test. Its universal
application authenticates the same Discord manifest as the Windows reference,
including all required-file checks. Altered application bytes are rejected.
These tests never execute the vendor backend.

## Runtime compatibility

Successful package verification does not translate a Windows or macOS executable
into a Linux program. Such backends remain visible as unsupported. Deckforge's
independent Rust Discord replacement runs natively; importing the original package
does not replace an already installed plugin with the same identity.

Native Linux executables and supported portable scripts use the existing plugin
runtime. No Wine compatibility layer is used.
