# System action continuation, 14 October 2026

This pass adds Hotkey Switch, Close, Text clipboard paste, Timer, and Mouse Wheel.
The action list groups navigation, sequences, Timer, brightness and Sleep under
Stream Deck. The System category matches the original Website / Hotkey Switch /
Hotkey / Open / Close / Text / Multimedia names; Mouse Wheel appears for dials.
Random Action was already implemented and is retained.

## Reference contracts

The supplied Windows MSI SHA-256 is
`b380e3b43e44674c874262c0453bda6cc9a1e5a6cfad8df6fd67eec56a48cd57`.
Its extracted 7.6.0.23012 executable SHA-256 is
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
The supplied macOS PKG and extracted binary also match the existing inventory.
The first macOS Ghidra import timed out; no runtime conclusion is drawn from it.
Windows Ghidra analysis completed and supplied the following native evidence:

- Hotkey controller selects the saved hotkey record by state on press and releases
  that record on key-up: `0x1402bc670`, `0x1402bc710`. `Hotkeys` and `Coalesce`
  serialization is retained. The switch uses two states and works inside the
  existing key-to-dial adapters.
- Timer settings are `duration`, `lastUserFile`, and `actionIdx`:
  `0x1402d8d50` / `0x1402d8fd0`. Default duration is 30 seconds at
  `0x1402d8ac0`; the key-up call multiplies duration by 1000 at `0x1402d95e5`.
  Sound selections 0–3 name vendor built-in WAVs, 4 selects the user file, and
  6 is silent, established by `0x1402da4a0`. Deckforge supplies its own generated
  tones instead of redistributing vendor sounds. Countdown formatting uses
  minutes/seconds or hours/minutes/seconds, as observed at `0x1402d9ff0`.
- Mouse Wheel forwards signed dial ticks and the held flag at `0x14084b5c0`.
  The Windows backend at `0x14059d920` uses `ticks * 120` and selects WHEEL
  `0x800` versus HWHEEL `0x1000` from that held flag. Linux emits signed
  REL_WHEEL / REL_HWHEEL events from a separate owned uinput device.

Private REA evidence and the precise vtable lookup script are retained under
`../evidence/system-actions-next/`. These are static observations, not execution
of the Windows or macOS app. The public
[System actions guide](https://help.elgato.com/hc/en-us/articles/360028234471-Elgato-Stream-Deck-System-Actions-Hotkey-Open-Website-Multimedia)
also describes the two-shortcut switch, application closing and Text modes.

## Linux behavior and limits

New Close assignments and native imports request normal KDE Wayland/local X11 window closure,
letting applications handle unsaved work. Explicit Send SIGTERM and Force close
(SIGKILL) are available for background processes. Existing saved Linux settings
retain automatic window-close/headless-SIGTERM behavior. All modes match the exact
same-user executable and optional literal argument prefix, with pinned process
identities. Desktop errors never trigger signal fallback. See
[Close application](CLOSE_APPLICATION.md) for defaults and verified limits.
Open Application shares the window-close implementation for held keys; see
[Open Application](OPEN_APPLICATION.md).

In paste mode, Text accepts Unicode and optional Enter, sets the desktop clipboard and sends
Ctrl+V, or a retained custom paste shortcut. Clipboard contents are replaced.
Explicit `isTypingMode: true` now uses the desktop keyboard portal after explicit
permission setup. Clipboard-free Unicode typing passes focused GTK and Konsole
acceptance; see [TEXT_TYPING.md](TEXT_TYPING.md) for behavior and remaining boundaries. The subsequent [dial hotkey queue](HOTKEY_COALESCE.md)
implements the original Coalesce limits, paced key strokes, reversal and cancellation.

Timer uses monotonic deadlines owned by each device. Press starts, pressing while
running cancels, and completion shows Done with an optional alarm. Hidden pages
continue counting. Timers reset when the host restarts; saved duration/settings
remain. The native vendor alarm animation is not copied. Custom audio uses the
existing decoder/output path; timer cancellation stops only that timer's sound.

## Verification

- Rust planning tests cover Unicode, invalid input, state-selected chords and
  scroll packet axes; the Close test terminates a uniquely copied test executable
  while leaving an unrelated process alive.
- `tests/system_actions_next_interop.py` exercises actual host press/release
  ownership, switch state, Close, and Timer start/cancel/completion/hidden pages.
- Timer tone decoding and scoped stopping use the real soundboard mixer.
- `tests/native_system_next_interop.py` exercises native WebKit form saves and
  the Mouse Wheel editor. Frontend field tests retain unknown settings.

Focused GTK Unicode Text/Enter and real Konsole custom-shortcut paste now pass
the isolated worker/EIS acceptance in [OS_ACTIONS.md](OS_ACTIONS.md). This does not
exercise real uinput discovery or USB. Hardware input, audible alarm acceptance
and the user's physical dial behavior remain separate checks.

The live installed release is
`5e2f18640dc37a7b41713a03ac14e807f19fd37f70b417ed47b89120b87a24d2`.
`tests/timer_audio_interop.py` captured generated alarm PCM on an owned virtual
output and checked custom audio plus stopping only the timer voice. The broader
soundboard suite passed its PCM/volume/fade/stop checks before reaching the
unchanged unsupported GRETSCH profile-import fixture. That fixture remains open.

Verification update, 14 October 2026: the Galleon profile/model gap is resolved.
The complete Action Trigger and soundboard suites now pass on their fixtures'
declared device models. See [Galleon support and remaining USB work](GALLEON.md).
