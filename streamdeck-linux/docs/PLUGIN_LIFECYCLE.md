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

## Remove and reinstall

**Remove** opens an inline confirmation for the selected plugin and its exact
folder. It retains profile assignments, action/global settings, application
mappings and host-managed private plugin data. Missing actions show question
marks on device previews, including Action Trigger children; saved artwork and
titles remain available for reinstall. Pedal retains its software preview.

A real, user-owned folder directly inside this session's `plugins` directory is
moved to `removed-plugins/<unique-id>/<original-folder>`. The backup directories
are private and existing backups are never overwritten. Other canonical plugin
folders are detached without moving their files. Missing entries can also be
removed. The dialog states which behavior applies. Paths are canonicalized at
load time, so classification uses the resolved folder rather than a historical
symlink used to select it.

The host saves the plugin's removal before stopping its runtime. If saving fails,
the running plugin and inspectors remain intact. If archiving fails after that
save, the error states that the plugin was removed from Deckforge but its files
remain at their original location. A crash between these steps can likewise
leave inert files behind; it does not erase profile data. Reattach that folder
through **Choose folder** or move it before installing a fresh package.

Reinstall the original package through Plugins to restore matching action UUIDs
and reuse host-managed settings/data at the original install path. Files written
inside the old plugin folder are preserved in its backup and are not blindly
merged into a new package. Native Remove does not log out of external services.
Explicit CLI `--plugin` paths attach those folders on launch and override the
saved plugin list; remove that explicit argument to keep an external folder
detached. Normal desktop launches use the saved list.

Live socket ownership follows launch/session identity rather than vector
position. Removing one plugin therefore cannot redirect another plugin's
messages. Remaining inspector indices are updated for every device while the
host mutex is held.

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

`tests/plugin_uninstall_interop.py` removes the first of two live plugins, keeps
the other backend and its existing native inspector on a second device running,
then performs settings and global-settings exchanges through their original
sockets. It checks input cleanup, archive contents, retained data, stale-session
rejection, absence on ordinary restart and package reinstall with the original
action contexts. Native removal confirmations, managed archiving, external
detachment and retained editor drafts are checked by
`frontend/tests/native_plugin_remove_interop.py` using temporary plugins.
Component tests also cover Cancel/Escape, stale confirmation and busy/error states.
