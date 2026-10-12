# Wave Link 3.3 reference investigation

The user supplied Elgato Wave Link **3.3.0.3125** for macOS to guide Deckforge's
mixer behavior and layout. The app was inspected offline. No vendor executable,
installer, account flow or audio operation was run.

## Artifact identity and tools

- DMG: `ElgatoWaveLink-3.3.0.3125.dmg`, 98,207,136 bytes.
- SHA-256: `df5c55e5a5f5df4dc2f8ead68471e6357737db761bf6af2cee1627f044897183`.
- REA 6.4.0 inspection evidence:
  `ev_2a76777d8f31a4e409df286da15409a8f689d2559f7ef51c3c533ef37cec38eb`.
- REA cannot extract DMG contents on this Linux host. A separate 7-Zip operation
  extracted 208 validated regular application files, excluding symlinks and
  alternate streams. REA then inspected that extracted tree, producing
  `ev_20a386b0461dd216bb6a802f307f50982e1d0234fd1b837f01c9394579cf6f1f`.

The bundle identifies itself as `com.elgato.WaveLink3`. Its universal executable
contains SwiftUI/AppKit/CoreAudio imports. It is a native macOS app, with no web
application bundle found in the extracted files. Platform-specific plist/Mach-O
REA utilities were unavailable; ordinary Python/LLVM observations are recorded
separately from REA evidence. Full local records and extraction provenance remain
under ignored `evidence/wavelink-3.3/`; proprietary app files are not redistributed.

## Behavior supported by shipped resources

The English `Localizable.strings` resource has SHA-256
`6e98ffebb2da0d53f7662298f05284a13b5961cc1faa0e0faebe88027573f516`.
Its workflow declarations and the supplied screenshots establish:

- Channel creation offers input devices, apps and non-running apps. Selecting an
  app creates a named channel. The initial channel has no mix route; clicking an
  empty matrix cell adds one.
- Channel settings support adding, transferring and removing applications, so a
  channel can group several apps.
- **All audio** covers system sounds and applications not assigned to another
  channel. Explicit membership is excluded from that catch-all.
- The matrix separates input, channel, mix and output. Resource keys distinguish
  channel mute, route mute, mix mute and per-output availability/mute.
- A main listening device and an ear selector choose which mix is monitored;
  a no-mix state also exists. This is a separate concept from assigning outputs
  permanently to mixes.

These are shipped UI/onboarding statements, not recorded execution of the app.
The relevant key families include `mixer.channel.create`,
`mixer.onboarding.channels`, `mixer.settings.channel.app`,
`mixer.channel.systemAudio`, `mixer.channel.inMix` and `mixer.output.device.main`.

Deckforge adopts the compact matrix, app-first creation and explicit-assignment
exclusion behavior. Its installed-app matching uses Linux desktop entries and
observed audio/process identities. Detailed routing changes still require Apply;
that is Deckforge's current interaction policy, not a recovered vendor rule.

## Native mechanisms and control declarations

The x86_64 slice contains `computerAudioExcludedApps`,
`updateComputerAudioTapProcesses()`, `createComputerAudioTap()` and
`AppDiscoveryTracker` names. It imports NSWorkspace application lifecycle notifications. A separate bounded
LLVM disassembly binds the call at x86_64 virtual address `0x10061da1f` to the
CoreAudio global stereo tap initializer with process exclusions, through stub
`0x100c34488`. A getter/setter pair also copies a process list in the reanchor path.
These are static call observations, not runtime audio observations; the exact
excluded-PID selection and helper-process grouping remain unresolved.
Linux implements the exclusion behavior through its own audio graph.

The bundled App Intents metadata declares Set volume ranges of 0–100, relative
Adjust ranges of −100–100, channel-and-mix targets, and mute/unmute/toggle choices.
It also declares assigning an output to a mix and switching an output between two
mixes. These are structured declarations, not observed audio effects.

Named meter fields reference left/right peaks in decibels, channel/route/mix peak
holds and decay. Actual meter positions, timing and formulas remain unverified.
Gain taper, linked-fader arithmetic, fade curves, output exclusivity and helper
process aggregation also remain unresolved. Effects remain deferred. Deckforge's
linear-amplitude percentages and linked percentage-point changes are explicit
local policies until stronger reference evidence establishes otherwise.

## Native analysis limit

REA's Ghidra inspection of the exact derived x86_64 slice ended with a
`provider_timeout` after 330 seconds. It produced no snapshot or pseudocode, and
its analysis processes exited. The retained failure is not counted as a successful
native reconstruction. The LLVM observations above are explicitly separate from
REA evidence; no unsupported native algorithm is presented as recovered source.
