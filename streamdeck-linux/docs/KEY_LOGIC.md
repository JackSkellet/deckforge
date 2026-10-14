# Key Logic

Key Logic now assigns independent actions to single press, double press, and
press-and-hold. Add it from the action library or a key's right-click/Edit menu,
select one of its three gesture positions, and assign or drop an action there.
Child settings, appearance, plugin inspectors and live previews use the existing
container editor. Saving preserves unknown fields and blank positions.

The Linux classifier follows the recovered Windows 7.6 timer paths: a quick tap
is resolved after 300 ms and sends a 60 ms key press; a second press within the
300 ms window starts the second action until release; holding for 500 ms starts
the third action until release. The single-press queue retains its native 15 ms
release gap and three pending-item bound. A first press released between 300 and
500 ms produces no action in this recovered path. That unusual boundary is
explicitly covered by deterministic tests; Windows runtime confirmation remains
open. See [native evidence](KEY_LOGIC_REFERENCE.md).

Children are ordinary Keypad actions, not Multi Action steps. The manifest's
`SupportedInKeyLogicActions` flag is respected independently of
`SupportedInMultiActions`. The unchanged official Counter works inside Key Logic
although it opts out of Multi Actions. Nested Key Logic and encoder-only actions
are rejected. A missing gesture shows a warning on the parent key.

Page departure, replacement, reconfiguration and device loss cancel pending
classification and release only the child actually owned by that gesture.
Single-press pulses reuse the existing keyboard preparation and release queue.
Profile export/import and restart preserve child data and resource handling uses
the shared container path. Parent artwork remains independently editable.

## Verification and boundaries

Pure timing tests cover single/double/hold, duplicate events, context isolation,
the medium-press gap and cancellation. Host tests check balanced SDK events,
manifest opt-outs, pending reconfiguration and device-disconnect cancellation.
The real-host process suite runs unchanged official Counter and a generated
protocol probe, then exports, reimports and restarts. The native WebKit suite
exercises right-click creation, fixed positions, nested fields, library drop,
saving and restart. It does not execute the configured native actions.

No physical key input or Windows runtime was used. A vendor-exported Key Logic
fixture has not yet been located, so generated Linux round trips are not evidence
of Windows export acceptance. Exact vendor artwork and gesture timing under the
Windows event loop remain unverified.

The broader Action Trigger regression now passes, including all eleven original
adaptor records on their declared device models.

Sources: [Elgato Key Logic guide](https://www.elgato.com/jp/en/explorer/products/stream-deck/key-logic-stream-deck/),
[manifest eligibility](https://docs.elgato.com/streamdeck/sdk/references/manifest/).

Verification update, 14 October 2026: the Galleon profile/model gap is resolved.
The complete Action Trigger and soundboard suites now pass on their fixtures'
declared device models. See [Galleon support and remaining USB work](GALLEON.md).
