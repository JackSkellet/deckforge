# Plugin system events

Deckforge observes systemd-logind's paired `PrepareForSleep(true)` and
`PrepareForSleep(false)` signals on the system bus. After that completion
notification it sends `willAppear` again for each visible plugin action on every
device, followed by one `systemDidWakeUp` event to every connected plugin.
Plugins with no visible actions still receive the global event. Hidden pages and
property inspectors do not receive wake notifications.

Action contexts, settings, selected device and plugin processes are retained.
The observer makes no suspend, inhibit, lock or service-start calls. Unlocking,
screensaver changes and USB reconnects are separate events and do not report a
computer wake. An initial or repeated `false` signal is ignored. Bus reconnects
and logind owner changes discard incomplete pairs; signals must come from the
current logind owner.

The snapshot's `system_resume` status reports observer availability. The app
continues running if logind or the system bus is unavailable and retries the
connection. Simulation does not start the real observer. Wake injection exists
only in simulated diagnostic mode.

## Evidence and limits

The SDK event contract follows Elgato's
[system guide](https://docs.elgato.com/streamdeck/sdk/guides/system/) and
[plugin WebSocket reference](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/).
The Linux source follows systemd's
[sleep notification contract](https://systemd.io/INHIBITOR_LOCKS/).
Logind also emits the completion signal after a failed or cancelled sleep
attempt. A paired signal is therefore a completion/resume indication, not proof
that the machine physically suspended.

Private D-Bus tests exercise signal pairing, sender validation, owner changes,
bus reconnection and shutdown. Host tests check separate USB/lock behavior,
plugin-only delivery and unchanged device state. `tests/plugin_wake_interop.py`
uses the unchanged official SDK 2.0.4 in real Node processes. It verifies visible
replay across two simulated devices, exclusion of a hidden page, notification of
an idle plugin, ordering and stable settings, contexts, selection and process IDs.
These checks do not suspend the user's computer. Physical suspend/resume
acceptance remains unverified.

See [application monitoring](PLUGIN_APPLICATIONS.md) for launch and quit events.
See [plugin localization](PLUGIN_LOCALIZATION.md) for language preferences and labels.
