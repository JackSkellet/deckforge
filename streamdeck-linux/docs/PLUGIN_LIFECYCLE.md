# Plugin lifecycle

In Plugins, **Disable** stops the selected plugin and its property inspectors.
Installed files, profile assignments, action settings and global settings remain.
The disabled preference survives application restart. **Enable** reloads the
plugin and presents its visible controls again. A failed launch is shown as a
failure; it does not silently change the enabled preference.

Disabled actions remain visible in the library and assigned controls. New
assignments, test presses, inspector opening, URL callbacks and Restart are
unavailable until Enable is selected. Existing assignments can still be edited,
moved, copied or removed. Other plugins continue running.

## Cleanup and persistence

The host saves the preference before stopping the plugin. A failed save leaves
the current process and session intact. Disable releases owned held inputs,
cancels queued sequences using that plugin, closes inspectors and sends action
disappearances for every active device. It then revokes the old connection and
registration before stopping the owned child process. Queued callbacks are
discarded rather than replayed on Enable.

Outgoing cleanup has a bounded 150 ms socket drain before process termination.
A successful drain means the socket sink was flushed, not that a third-party
plugin completed its external effects. A timeout is logged and does not prevent
stopping an unresponsive plugin. Per-launch identities reject messages from old
processes, including writes attempted during shutdown.

This durable disable preference is Deckforge policy. The original application's
Stop and Restart operations are documented, but identical persistence and cleanup
semantics have not been established for the original application.

## Validation

`tests/plugin_enable_interop.py --require-wire-cleanup` uses a cooperative probe
and the unchanged official Counter sample on two simulated devices. It verifies
the probe processes queued cleanup frames before its cooperative shutdown exits.
The probe exits as soon as its held inputs, visible actions and inspectors have
been cleared, with a bounded fallback deadline. SIGTERM can arrive before the
remote message handlers run; socket flush does not establish that ordering.
The fixture also verifies stale registrations and shutdown writes are rejected,
native inspector children are reaped, disabled state survives restart, and other
plugin PIDs and settings are unchanged.

`frontend/tests/native_plugin_lifecycle_interop.py` exercises the actual native
Disable and Enable buttons, preserves an unsaved editor draft and checks the
assigned control and action library states. No physical device or live user
session is changed by these fixtures.
