# Audio mixer

Deckforge's mixer is an original Rust implementation using the PulseAudio protocol,
including PipeWire's PulseAudio server. It provides independent monitor and stream
mixes inside the desktop application. It does not run Wave Link or Wine.

![Native mixer with an isolated test configuration](../assets/screenshots/audio-mixer.png)

## Mixer and routing setup

**Mixer** shows compact channel rows and mix columns. Each routed cell has a mute
button and level slider; meters stay beside channel and mix names. Click an empty
cell to add that route to your draft. Detailed names, membership and device
choices live in **Routing setup**.

1. Open **Audio mixer**. Browsing reads applications and devices without enabling
   routing.
2. Choose **Create channel** and search for an installed application or input
   device. Selecting it creates a named channel with its identity already filled
   in. An empty channel can group several apps later.
3. Add the desired mix routes by clicking empty cells. Open **Routing setup** to
   choose listening outputs, rename channels/mixes or edit their members.
4. Enable routing and choose **Apply routes**. Membership, route switches, output
   assignments and names stay drafts until applied. Saved levels and mutes respond
   immediately and preserve unapplied routing drafts.
5. Select a mix's **Deckforge mix** virtual input in your recording or streaming
   application. Deckforge does not change system default devices.

Monitor and Stream are the initial mixes. The current resource bounds are 64
mixes, 128 input groups and 4096 explicit patchbay connections. Stereo input ports
can each accept multiple feeds; they are not a two-source limit. A channel can contain several apps and input devices; a mix can
feed several outputs. Missing saved devices stay visible and are never silently
replaced with the system default. Audio uses 48 kHz stereo; volume percentages
represent linear amplitude.

## Automatic app discovery

The picker includes installed desktop applications before they play audio. It
reads XDG desktop entries and Flatpak exports, uses localized display names and
saves a stable desktop-entry ID. Hidden helper entries stay out of the picker;
identical hidden portal aliases fold into their visible application. No application
is launched to identify it.

When a stream appears, Deckforge matches explicit app/desktop identities, launch
hints, Flatpak metadata or a unique executable identity. Wrapper-based apps can
also match an exact reported name within the same private installation directory.
Shared launchers such as
`flatpak`, `env`, interpreters and Electron are not treated as unique applications.
Ambiguous matches remain unresolved. **Playing applications** provides observed
audio IDs/process names as a fallback; **Advanced: manual identity** remains
available. An installed app marked **Pending audio** has no currently resolved
stream, so it can be configured ahead of time.

## All audio and explicit exceptions

Choose **All audio** under Create channel, or enable **Default for other apps** on
an existing channel. Multiple channels can share this source. After Apply, one
capture bus accepts application audio without an explicit assignment, including
new streams, and fans it out to the chosen channels.
Explicit app assignments take priority. A stream deliberately sent to another
Deckforge channel stays there; manually moving an already routed stream to another
output is also respected, including when an unrelated app assignment changes.
Changing that stream's assignment or explicitly re-enabling routing can route it
again. Turning the default option off restores streams Deckforge moved to it.

This does not automatically capture microphones or loop Deckforge's own mixer
streams into themselves. Input devices still require explicit selection.

## Listening to a mix

Choose **Listen on** in the Mixer view, then use the listen button beside a saved
mix name. Clicking the selected button again, or **Stop listening**, selects no
mix. These choices save immediately, including while routing is off; they never
enable routing themselves. New, unapplied mixes cannot be monitored yet.

This main listening destination is separate from fixed per-mix outputs and virtual
recording inputs. Switching it replaces only the listening connection: mix buses,
recordings, fixed output feeds and in-progress level fades continue. A missing
destination stays selected and unavailable until that same named device returns;
Deckforge never substitutes the system default. Its current system mute state is
shown, but choosing a mix does not change the device's system volume or mute.

Several mixes may feed the same output, including the chosen listening device.
An identical fixed-output/listening pair is sent once, using that fixed send's
gain and mute. Different mixes remain separate feeds and are summed by the audio
server. Existing routing drafts survive listening changes. Deleting the selected mix clears its listening selection when
that deletion is applied and retains the chosen output device.

## Live controls and meters

Saved route, mix master and per-output levels/mutes apply immediately. With routing
inactive, they save the preset without enabling audio. Controls on a newly added,
unapplied route or output remain draft values until **Apply routes**.

**Link mix levels** moves a group's existing mix faders by the same percentage-point
change, clamped to 0–100%. Each route's mute remains independent. **Default fade**
sets a 0–10000 ms linear-amplitude transition for subsequent level changes, including
keys/dials; 0 is immediate. Retargeting a fade starts at its current interpolated
level. Mutes apply immediately and do not cancel unrelated fades.

Each assigned output has its own level and mute. These affect only Deckforge's
stream into that destination, leaving the physical output's system volume alone.
A mix master affects both its recording source and all its output sends.

Input-group and mix meters measure stereo signal peaks from their owned virtual
buses. They show dBFS, silence, unavailable state and a full-scale indicator.
They do not record a default source or microphone to obtain meter values. Input
meters precede route controls; mix meters include route and master levels.
Display names are aliases inside Deckforge; external audio node names use stable
IDs so renaming a group or mix does not interrupt attached applications.

Rapid control edits are coalesced before saving; the UI reports unsaved controls or
save errors. A save failure keeps the selected live level and retries persistence.
Quit flushes pending control settings. Routing edits still require explicit Apply.

## Keys and dials

The action library contains **Mixer channel** and **Mixer output mix**. Choose a
configured input group and mix for the channel action, or a mix for its master
control. Dial rotation adjusts the configured percentage step; push or touch
toggles mute. Keys can toggle mute, set a level, or change it by a fixed amount.
These actions also work inside supported Multi Actions and Dial Stacks.

Device actions never enable the mixer or create routes. Their displays show the
last level accepted by the mixer worker, with pending and unavailable states.
This is control feedback, not an audio level meter. The action IDs and settings
are original Deckforge definitions, not guessed Elgato settings.

## Recording input names

Routing setup displays each mix's verified recording input description and exact
name. Use that input in your recording application. The default Monitor mix now
uses an input ending in `.source.monitor.input`, labelled **Deckforge mix monitor
(recording)**. Other mix input names are unchanged.

The old `.source.monitor` name conflicts with [PipeWire's reserved `.monitor`
suffix](https://github.com/PipeWire/pipewire/blob/1.6.9/src/modules/module-protocol-pulse/pulse-server.c#L2114): a client requesting it by name can be redirected to a sink monitor or the
default input. Applications configured with that old name should reselect the
corrected input. Deckforge does not retarget other applications' recording streams.
An actual reader still attached to the old source keeps that endpoint during the
upgrade; the canonical input is advertised separately, and the old endpoint is
retired after its last reader releases it.

## Lifetime and recovery

Closing the editor leaves routing active. Disabling the mixer or quitting restores
explicitly moved applications when their original output still exists. A stream
the user moved elsewhere is left there. If its original output is missing, the
owned channel remains silent until it can be restored. An external recorder still
attached to a mix likewise retains a silent virtual input rather than falling back
to a microphone.

Private ownership records under `$XDG_STATE_HOME/deckforge-mixer` retain exact
module arguments, session identities and original routes. A kernel file lock
prevents concurrent ownership. A disabled fresh session does not adopt or alter
parked nodes. Enabling the mixer recovers verified matching nodes. Normal
Quit/restart with attached recorder and application streams has virtual-audio
coverage; full audio-daemon restart and abrupt crash recovery still need broader
acceptance testing.

## Compatibility work still required

This is the routing foundation for native Wave Link functionality. The current
official Wave Link plugin's settings and protocol still need verification before
its saved actions can resolve to these controls. Imported unknown actions/settings
remain preserved. Effects/plugin hosting and Wave-device DSP controls remain
open and are deferred. Deckforge's link policy, fade curve and percentage scale
are explicit local behavior, not claims of identical vendor behavior.

The shipped Stream Deck 7.6 profiles establish six `com.elgato.wave-link` action
identities but contain empty settings. They cannot establish selector keys, enums
or volume curves. The original plugin and current vendor application are not
runtime dependencies of this implementation.

## Verification

Native-window tests now exercise **Add input group** and **Add mix** directly,
including draft retention after reopening the dialog. The earlier native build
called an unavailable `crypto.randomUUID` API and those buttons failed before
creating an item. A shared secure-random UUID fallback fixes both mixer creation
and nested-action creation. Before/after tests keep routing disabled and do not
apply routes or alter the audio backend.

`tests/mixer_application_discovery_interop.py --run-virtual` creates a private
PipeWire/Pulse server with policy support and no hardware monitors. A generated
native audio client proves installed-app selection before playback, automatic
future matching, default capture, explicit exceptions, manual-route preservation
and original-output restoration. It cannot route applications on the user's audio
server. The native installed-app fixture verifies creation, search, draft retention
and default selection through actual controls while routing stays disabled.

`tests/mixer_host_interop.py --run-virtual` exercises the real host, saved config,
independent routes, simulated keys/dials, linked fades/retargeting, immediate mute,
revision guards, coalesced saving, control feedback, restart and cleanup.
`tests/mixer_virtual.py --run-virtual` sends generated PCM through owned temporary
nodes to measure route gain, mute and separation, physical-route primitives using
virtual endpoints, app restoration and attached-reader recovery. Neither test
captures a real microphone, routes to a real speaker, or moves a user's app.
`tests/mixer_meter_virtual.py --run-virtual` measures stereo peaks, fade progression
and independent output gains/mutes using generated PCM, and verifies stable rename identities and
meter cleanup. `frontend/tests/native_mixer_controls_interop.py` exercises the actual
native sliders, links, output controls and fade preference with routing disabled,
including preservation of an unapplied mix. Physical audio acceptance and full
Wave Link parity remain unverified.

`tests/mixer_listening_interop.py` runs a private audio server and generated tones
through the real host. It measures mix/output switching, No mix silence,
uninterrupted recording windows, fade continuation, endpoint identity preservation,
output disconnect/reconnect, unchanged device controls and inactive restart.
These are functional checks on a private server without real-time scheduling.
One run concurrent with frontend tests showed a transient gap in a fixed output;
isolated runs passed. Timing under CPU load and physical-device acceptance remain
unverified and are part of the user's reliability testing.
It also verifies each recorder's actual source identity and signal frequency.
`tests/mixer_recording_migration_interop.py` compares a supplied previous binary
with the current build on that private server, retaining a numeric legacy reader
while creating the corrected named input and retiring the old alias afterward.

## Reference behavior

The compact matrix, app-first creation and All audio exclusion rule are grounded
in the user's Wave Link 3.3 screenshots and shipped resources. See
[Wave Link 3.3 reference investigation](WAVE_LINK_REFERENCE.md) for artifact identity,
REA evidence and the distinction between recovered declarations and inferred code.

## Advanced routing canvas

The Carla-style canvas has freely movable nodes, background panning, zoom,
Fit canvas, Arrange nodes and an expanded view. Drag node headers or use their
focused arrow keys; Shift moves by larger steps. Node positions, zoom and channel
expansion survive restart in a private `audio-canvas.json` file. These layout
changes never alter audio configuration or save unrelated action drafts.

Click ports or drag between them to connect. Each input accepts multiple cables.
Applications and microphones can feed several destinations, mixes can share an
output, channels and mixes can feed one another, and direct source-to-output
connections are supported. A physical output's outgoing port captures its monitor.
The recording node is the mix's automatic virtual input for other applications;
its outgoing port can feed another mixer node. Sources expose output ports and
receiving devices/buses expose input ports according to their role.

Use **L/R** to expose individual channel positions at both ends of a connection.
Microphones retain their actual mono/channel map. For example, a source's Left
port can connect only to the headphones' Right port. Split an existing stereo
cable to remove or trim one side while retaining the other. Selecting a cable
exposes its level, mute and Disconnect controls. Connection edits share the
normal retained routing draft and require **Apply routes**, also available in
expanded view. Runtime errors and unapplied-change status remain visible there.

Applications are captured once before fan-out, including a shared All other audio
source. This avoids repeatedly moving a single playback stream between competing
destinations. Existing single-group captures keep their original routing. Ambiguous
alternative selectors still produce a notice instead of guessing an application.
Node-level feedback cycles are rejected before changing the graph, including
known physical monitor loops. A channel-only self-route stalled an isolated
PipeWire graph even though its signal channels looked acyclic, so such node cycles
are not permitted. Missing devices stay named and unavailable without fallback.

The new typed connection model uses independently implemented Rust planning and
ownership tracking. Individual channel mappings use an owned
[`module-remap-source`](https://docs.pipewire.org/page_pulse_module_remap_source.html)
followed by a loopback with remixing disabled. No Carla code, JACK host, or effects
engine is embedded. Carla's [patchbay description](https://kx.studio/Applications:Carla)
and [JACK engine](https://github.com/falkTX/Carla/blob/main/source/backend/engine/CarlaEngineJack.cpp)
were behavior references. This canvas edits Deckforge-owned audio routing; MIDI,
effects and editing unrelated applications' raw PipeWire/JACK port graphs remain
outside this implementation.

## Patchbay verification and reported silence

`tests/patchbay_channel_probe.py` checks the native routing primitives on private
PipeWire with generated PCM. It verifies left-to-right mapping, two sources summed
into one ear, and simultaneous right-to-left mapping to another output.
`tests/mixer_patchbay_interop.py` exercises the real host with three inputs to a
mix, chained channels and mixes, output-monitor and recording taps, an application
feeding two mixes, shared fixed/listening destinations without accidental double
feeds, per-connection gain and feedback rejection. Defaults remain unchanged.

Frontend tests cover fan-in/out, individual cables, gains, duplicate suppression,
node gestures, draft retention and Apply. The native window fixture verifies
multiple mixes on one device, individual channel connections, expanded view,
keyboard movement/disconnect and saved node positions/port expansion across a
full process restart. Native UI tests keep routing disabled. Physical drag/cable
acceptance and physical headphone audio still need user acceptance.

On 13 October 2026 the user confirmed that audio routing now works in their
tested setup. This closes the reported silent-output case at the user-acceptance
level; it does not establish every channel mapping or device combination.
Previously, read-only inspection found a voice playback stream on a muted,
removed channel without an original-stream move record. That observation alone
did not establish the original failure's cause. Effects remain deferred.
