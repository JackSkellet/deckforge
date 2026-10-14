# Lock keys, other keyboards and repeat

Logical shortcuts respect desktop lock state. Caps Lock changes letter case,
Shift reverses Caps Lock's case behavior, and logical number-row resolution keeps
producing the selected digit. The action does not clear Caps Lock. Numeric-keypad
identities remain physical: Num Lock selects their digit or navigation behavior.

In controlled KDE tests, Ctrl, Shift and AltGr held by a second virtual keyboard
remain effective while Deckforge sends a shortcut. Sending and releasing the same
modifier through Deckforge does not clear the second device's held modifier.
Releasing the second device restores the unmodified state. These results cover
separate owned virtual devices, not physical hardware or every compositor.

Holding a letter produces the focused Qt client's normal repeat. A modifier-only
hold does not repeat. Releasing the action stops repeat and delivers one final
non-repeat release. No extra synthetic repeat timer was added to the Linux writer.

## Verification

`tests/keyboard_state_interop.py` creates a private KWin session and two independent
EIS keyboard devices with distinct packet files. The first forwards the actual
Rust imported-chord resolver/keyboard writer. The second supplies controlled lock
and modifier events. Both relay connections verify the exact private KWin PID.
An owned Qt window records key identity, native scan, modifiers, text and repeat
flags. No user desktop, physical keyboard, uinput or USB device is used.

US, German, Norwegian and French pass:

- Caps Lock on/off, Shift inversion, digit1 and lock preservation.
- Numeric keypad7 changing with Num Lock and restoring its previous behavior.
- Left/right Ctrl and Shift inherited from the second keyboard, including sending
  the same modifier from Deckforge and preserving it after Deckforge releases.
- German/Norwegian AltGr inherited and shared across both devices.
- Repeating heldY, nonrepeating heldCtrl, final release and no post-release events.

The final traces contain19 repeat presses in each1.3-second held-letter check.
This records the tested desktop's behavior, not a promised repeat rate. Report:
`target/keyboard-state/proof.json`, including exact test-binary/runtime-source
hashes and all client traces. Physical-device validation remains user-owned.
No production code or installed build changed for this acceptance increment.

## Native reference

Static analysis of the supplied7.6 Windows binary
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`
shows the ordinary KA_HotKey press path at `0x1402bc670` sending configured modifiers
then the main key. `0x1402bc710` releases the main key then configured modifiers.
These traced paths do not surround dispatch with an external-modifier reset/restore.
The existing OS helpers `0x14059d990` and `0x14059d9f0` read Windows keyboard delay
and speed settings. The Windows app was not executed; its physical-key interaction
is not established by this static evidence. Evidence is in
`../evidence/keyboard-state/native-dispatch.json` and the existing sided-modifier
analysis. The Linux result above is established independently by owned-client tests.
