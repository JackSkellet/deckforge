# Left and right shortcut modifiers

Hotkey, Hotkey Switch, dial shortcuts and custom Text paste shortcuts support the
original side-specific modifier mode. In the editor, enable **Choose modifier
sides**, then select Off, Left, Right or Both for Ctrl, Shift, Alt and Super.
Right Alt acts as AltGr when the desktop layout maps it that way.

Changing the main key preserves selected modifier sides. Turning side selection
off returns to generic modifiers; turning it on starts enabled modifiers on the
left. Clear removes the whole chord. Imported unknown settings remain intact.
Existing generic shortcuts continue to use left modifiers. Captured side bits
without the original mode flag do not activate side-specific behavior.

Physical mapping keeps Linux key positions, so Y produces Z under a German layout.
Choose Follow desktop layout to resolve supported characters against the active
KDE Wayland map; see [Logical shortcuts](LOGICAL_SHORTCUTS.md). The mode preserves
modifier sides and captures concrete identities for release. Other compositor,
physical keyboard acceptance remains open. Caps/Num Lock and modifiers from a
second virtual keyboard pass [controlled keyboard-state tests](KEYBOARD_STATE.md).

## Legacy and modern records

The original loader treats a missing KeyModifiers field or the exact value8192
(0x2000) as the legacy format. Ctrl, Shift, Alt and Super then come from KeyCtrl,
KeyShift, KeyOption and KeyCmd booleans. In modern records the numeric mask wins;
old booleans are dormant data, even when stale or contradictory. Deckforge now
uses that rule for key dispatch, dial timing and displayed checkbox states.

Loading, saving unrelated fields, export and restart preserve the original record.
Editing a modifier or key writes a modern mask for that deliberate edit. The
legacy marker itself is never sent as a modifier. Invalid active legacy booleans
and unsupported mask bits remain errors; dormant modern boolean fields are ignored.
The portable Qt identity policy is unchanged: platform-local NativeCode/VKeyCode
values are not blindly treated as Linux scan codes.

Native evidence: loader0x1402bb4e0 reads mask with default0x2000, tests exact equality,
and reads the four boolean fields in that branch. Modern records skip those reads.
The original Windows key resolver0x14059e1c0 prefers a supplied VKeyCode, while
label renderer0x14059dda0 consults the current Windows layout. Therefore a blanket
conversion of every imported QTKeyCode to a desired Unicode character is not
established by this evidence. General Hotkey imports/new records retain the explicit
physical/logical choice; existing absent modes remain physical. Static evidence is
in ../evidence/shortcut-legacy/native-analysis.json. No vendor app was executed.

New Clipboard actions and standard Text paste actions default to logical mapping,
where Copy/Cut/Paste have an unambiguous character shortcut. Existing records and
imports remain unchanged. Text's custom paste shortcut uses its own mapping; a
valid older custom chord is not overridden by the new standard-paste default.

## Original evidence

The supplied Stream Deck 7.6 executable SHA is
`2963df27b13f97e5736e3f931348d89d2f693109f3cb9dc3c9899300e35ea9e3`.
Static analysis of `0x14059d3b0` shows that KeyModifiers bit `0x10000` switches
from a four-entry generic modifier table to an eight-entry left/right table.
Press iterates forwards and release backwards. The four modifier families use
low summary bits Shift=1, Ctrl=2, Alt=4, Super=8. Left bits are summary<<4;
right bits are summary<<8. The generic table at `0x1415a1f38` maps masks
8/4/2/1 to Windows VK 5b/12/11/10. The sided table at `0x1415a1f60` maps
80/800/40/400/20/200/10/100 to VK 5b/5c/a4/a5/a2/a3/a0/a1.
`0x14059da90` reads matching left/right key-state masks. Evidence is saved in
`../evidence/sided-hotkeys/native-analysis.json`. No vendor executable was run.

Deckforge translates those modifiers into Linux left/right key codes and uses
its existing hold ownership/reverse release path. It preserves legacy generic
ordering and does not reset external keyboard state. Backtab adds Left Shift only
when neither Shift side is already selected. Keypad and OEM identity flags remain
separate from modifier flags.

## Verification

Rust tests cover all four modifier families, right/both sides, modifier-only
chords, generic-mode captured side metadata, sided keypad identity and Backtab.
The existing frame/ownership/failure tests also pass.

`tests/sided_hotkeys_interop.py` runs actual translated input packets through the
existing test-only EIS relay into private KWin instances configured for US, German
and Norwegian layouts. An owned Qt client records native scan codes, key values,
text and modifiers. Left/right/both Ctrl+C, Right Shift+A, AltGr and subsequent
plain keys pass. German Right Alt+Q and Norwegian Right Alt+2 produce @; US Right
Alt+Q produces q with Alt behavior. German Y-position output confirms the remaining
physical-versus-logical layout gap. The tests never open real uinput or target
user windows; they do not establish the behavior of a physically held user key.
Reports include test-binary and source hashes in `target/sided-hotkeys/proof.json`.

`tests/native_sided_hotkeys_interop.py` checks the real WebKit editor: imported
Right Alt, selecting both Ctrl sides, main-key changes, opaque settings, Save and
restart. It executes no shortcuts. Frontend tests additionally cover switching
between sided and generic modes. Native proof/screenshots live in
`target/sided-hotkeys/native/`.
