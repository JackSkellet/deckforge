# Multiple devices

Deckforge discovers supported USB devices and keeps a separate running profile
for each. Choose a device in the header to edit it. Selecting another device does
not stop its plugins, hide its actions, release its held controls or change its
current page. Disconnected devices remain available for editing and reconnect
with their saved assignments.

A plugin runs once for the application. Its SDK registration includes every
associated device, including offline devices. Action events and property
inspectors use the action's owning device, regardless of which editor is open.
Plugin global settings, the audio mixer, soundboard and system-volume services
remain shared.

**Preferences → Device → Device name** gives each deck a custom name. The picker,
profile-copy destinations and plugin metadata use it. Empty names reset to the
model name. Names accept up to 64 Unicode characters without control characters;
leading and trailing whitespace is removed. Duplicate names are allowed because
the picker also shows device identity. Renaming works while a device is offline
and preserves its serial, profiles and action contexts.

Names are saved before plugins receive the documented
[`deviceDidChange`](https://docs.elgato.com/streamdeck/sdk/references/websocket/plugin/#devicedidchange)
event. Failed saves preserve the previous name and emit no change. Launch
registration and reconnect events include the current name; an offline metadata
refresh after plugin registration also covers renames during plugin startup.

## Forgetting a device

Disconnect the device, then choose **Preferences → Device → Forget device…**.
The confirmation removes that device's profiles, name, brightness/sleep
preferences and action clipboard. Other decks keep running. The host closes its
property inspectors, sends action disappearances and cancels its pending Multi
Actions and held controls. A device that reconnects while the confirmation is
open cannot be forgotten until it disconnects again.

Before removal, Deckforge writes a private recovery copy of the complete session
under `forgotten-devices/` beside `session.json`. The Activity log records the
exact path. If either this copy or the final save fails, the device stays in the
session. Copies are retained without overwriting earlier ones, and imported
asset archives are not deleted. To recover, quit Deckforge, keep a copy of the
current `session.json`, then replace it with the chosen recovery JSON. This
restores the entire session as it was at that point, including other devices.

Forgetting the last device leaves an empty workspace for its model. Plugging a
forgotten device in again discovers it afresh. Old USB events are ignored until
its worker has stopped; subsequent discovery uses a new generation. No factory
reset or firmware write occurs. This removal behavior is a Deckforge policy;
recovery copies are not an Elgato profile-package format.

## Profiles and action editing

Each device owns its profiles and navigation history. **Profile options → Copy
to device…** copies the current profile to a compatible device and activates the
copy there. The source stays unchanged, and the editor remains on the source.
Copies receive fresh page and action identities, including nested actions; they
are independent profiles rather than shared editable templates.

You can copy or cut an action, select another device, and paste onto a compatible
control. A cut remains at its source until a successful paste. Stale targets,
read-only destinations and incompatible controllers are rejected. A failed save
preserves both devices and the pending cut. Moving a folder also requires
compatible profile geometry so its contained controls cannot be silently lost.

Unsaved drafts and selected controls survive switching devices. An asynchronous
save or file picker keeps the device it was opened for. Ordinary settings edits
apply only changed fields to the latest plugin settings, preserving values such
as a Counter count that advanced while another device was selected. Arrays are
replaced as units; explicit edits and deletions still apply. A conflicting object
type change requires reloading the draft. Elgato's SDK `setSettings` continues to
replace the whole settings object as its protocol specifies.

## Discovery and ownership

Without a filter, supported devices are discovered automatically. `--serial` or
`--model` restricts which physical devices that process opens; other remembered
devices stay offline in the picker. The registry reserves each HID path before
opening it and verifies serial/model identity before handing it to a worker.
Known nonmatching serial descriptors are skipped. When a supported device lacks
a serial descriptor, a reserved metadata probe identifies it; a mismatch closes
before any worker, input loop or image/control output is admitted. Reconnect
probes cannot open paths held by another worker in the same registry.
These reservations do not claim to exclude unrelated third-party applications.

Each worker keeps its own latest frames, input transitions and reconnect pacing.
Its events carry an identity and generation, and retired events cannot update a
replacement runtime. A new transport is bound to the host only after its model
is confirmed, so an offline placeholder cannot send frames for the wrong model.
On shutdown, every worker receives an ordered Stop before any worker is joined.
No device reset or firmware update is part of discovery or shutdown.

The current limits are 32 associated device runtimes and 32 USB workers. Unsupported transports remain
listed in [Device models](DEVICES.md); simulated devices do not establish physical
USB acceptance.

## Persistence and internal ownership

The session adds a `devices` array of stable runtime identities and per-device
navigation. `active_scope` records the runtime whose projection was serialized;
`selected_scope` records the editor selection. They can differ after a background
plugin update. Startup restores each runtime, then the saved editor selection.
The old session is retained as `session.before-multi-device.json` on migration.

The shared Host mutex serializes operations. `with_device` temporarily binds the
requested runtime, keeps the previous runtime available in the pool, and restores
it on both successful and failed results. It owns this ordering; callers must not
change editor selection inside such an operation. Timers, physical holds,
inspectors, badges and display channels are device-owned. Profiles, plugin
processes/settings, OS services, renderer and persistence are application-owned.

Cross-device clipboard operations first validate and persist the complete new
profile graph. During its lifecycle transition, that graph is authoritative;
old visible projections cannot overwrite it. Releases and disappearances are
then applied only to affected runtimes before normal projection synchronization
resumes.

## Validation

`tests/multi_device_interop.py` runs one unchanged official Counter plugin and an
inert protocol probe across three simulated devices. It checks SDK ownership,
connection metadata, independent held controls and timed actions, global settings,
bundled profiles, cross-device profile copy and restart after a background save.
`tests/native_multi_device_interop.py` exercises the actual embedded picker,
retained drafts and live settings through two concurrent Counter contexts.
Both suites also check renaming: SDK notifications and restart metadata in the
process test, plus save/reset, picker updates and retained names in the native UI.
`tests/device_forget_interop.py` checks removal, cancellation, other-device
isolation and restarting both the resulting session and its recovery copy.
`tests/native_device_forget_interop.py` checks the actual confirmation, Cancel
focus, picker removal and last-device workspace using an absent Mini serial and
model filter. It never operates the attached Plus.

Unit tests cover exclusive path reservations, metadata identity checks,
generations, Stop ordering, device limits, inspector permissions, restart cleanup,
clipboard save failure and recovery, and deferred transport binding. Physical
operation of two simultaneous USB devices remains unverified: only a Plus is
available on the development machine.
