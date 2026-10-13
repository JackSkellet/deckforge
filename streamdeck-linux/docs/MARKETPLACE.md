# Marketplace sign-in and browser installs

Open **Preferences → Marketplace → Sign in to Marketplace**. Sign in on Elgato's
own page in the separate native window. Deckforge retains that browser session
under the application state's `marketplace/browser` directory. It is separate
from plugin HTML and the user's regular browser. No password field or OAuth
client credential is implemented by Deckforge.

Click **Open in Stream Deck** on a product already available to that account.
The registered `streamdeck://plugins/install/<variant UUID>` link activates
Deckforge and starts a background download. If the session is missing or expired,
the sign-in window opens and the pending request continues after sign-in.
The account must have access to the product; Deckforge does not place orders or
acquire paid products. Use the same account in both browsers.

**Check session** verifies the saved session with Elgato. A saved cookie is not
proof that an account is still authenticated. Elgato controls session expiration
and renewal. **Sign out on this device** deletes Deckforge's isolated session;
it does not sign the user out of their regular browser.

The editor shows progress and actionable errors. A failed request retains the
variant in memory for **Retry plugin install**. Repeated clicks on that same
variant while it is in progress are coalesced. Other requests wait until the
current request finishes. Pending installs are not saved across app restarts.

Downloaded packages go through the existing archive, manifest and integrity
checks. Protected packages still require the supported local verification
reference described in [Protected packages](PROTECTED_PACKAGES.md). Installation
does not make Windows/macOS-only plugin executables run on Linux.

For a new plugin, validated files are published without overwriting an existing
folder and registered through the existing host lifecycle. An already-installed
version is reported without replacement. A different version uses the existing
managed update preparation and rollback mechanism, with review in Preferences
before replacement. External plugin folders remain user-managed.

## Session and transfer boundaries

- The account browser has no Deckforge editor or plugin command bridge. Replies require the Marketplace top-level origin and a per-process capability
  supplied only to the top-frame session probe. This prevents an iframe from
  forging a response when WebKit reports the parent page URI. Identity
  provider pages cannot invoke the account-result bridge. HTTPS redirects remain
  available for normal sign-in; external protocols, browser downloads, new
  windows and camera/microphone permissions are blocked in that helper.
- The browser profile is private, mode 0700, and helper-created files use umask
  0077. A file lock prevents concurrent access to the profile. Session cookies
  stay in WebKit; they are never copied into profiles, snapshots or activity logs.
- Only the short-lived signed download link passes through an owned helper pipe.
  Download errors omit URLs. HTTPS download and redirect hosts are restricted to
  the observed Elgato Marketplace/CDN hosts. Transfers have time and size limits,
  and incomplete files are discarded. No account cookies are attached to the
  package transfer. Requests identify the app as `Deckforge/<version>`; the CDN
  rejects requests with no User-Agent even when the signed URL is valid.
- Browser and network work run outside the host lock. Archive staging is also
  outside it. Final registration uses the existing host installation policy.
  Shutting down Deckforge cancels the helper or pending transfer.

## Observed website contract

Verified from Elgato's own website on 13 October 2026:

- `GET https://marketplace.elgato.com/api/auth/session` returns `user: null` for
  a fresh session. Authenticated status is checked through the same endpoint.
- `/auth/login?callbackUrl=%2Faccount%2Fproducts` starts Elgato's own account
  sign-in flow and returns to its site. The website owns session renewal.
- `GET /api/proxy/purchase-link/<variant UUID>` uses the browser's session and
  returns download information including `direct_link`. The site's own download
  button uses this endpoint. Anonymous access returns 401.
- The captured variant `30bbf05a-ee28-45cd-b3a6-b072d3d56351` resolves to
  Twitch Creator Tools through Elgato's public
  `https://mp-gateway.elgato.com/allitems?ids=<variant UUID>` endpoint.

Sources are the public Marketplace bundles
[`9469-9939df46b122b422.js`](https://marketplace.elgato.com/_next/static/chunks/9469-9939df46b122b422.js)
and [`5315-1804d1b80d8901d7.js`](https://marketplace.elgato.com/_next/static/chunks/5315-1804d1b80d8901d7.js).
These are website interfaces, not a promised stable third-party API. Site changes
can require an adapter update. Unknown responses and download hosts fail visibly.

## Validation

Rust tests cover install-route parsing, duplicate requests, private profile
locking, safe logout, bounded transfers, action collisions, new installs,
same-version installs, and reviewed updates preserving disabled state.
Frontend tests exercise the actual session-probe script with session expiry,
denied access, origin isolation and download responses, plus account controls,
retry and update review.

`tests/marketplace_native_interop.py --report <path> --capture <path>` runs the
native Preferences UI and real anonymous session check in a temporary simulated
state. On 13 October 2026, the user signed in and a subsequent native session check
confirmed authentication. After a Deckforge restart, the captured Twitch Creator
Tools link downloaded version 0.5.0.50, passed package verification, installed as
`com.jackstmc.twitch-chat`, and registered its plugin WebSocket. No second sign-in
was needed. This proves that account/session and package path on this host; it
does not prove every Marketplace product or Twitch action works.

The first live download exposed a CDN requirement omitted from the initial
implementation: a valid signed URL returned 403 without a User-Agent and 200
with `Deckforge/0.1.0`. The downloader now sets that header. A transport regression
test fails without it and passes with it.
