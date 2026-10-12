//! Linux uinput keyboard and imported hotkey translation, written from public APIs.
//!
//! Sources: https://docs.kernel.org/input/uinput.html, linux/uinput.h,
//! linux/input-event-codes.h, Qt's Key enum, Microsoft virtual-key documentation,
//! and evidence/system-action-contracts.json from the original installer.
//!
//! Output is physical Linux key identity. This does not perform Unicode typing or
//! translate the desktop's current XKB layout. Qt identity wins over platform-local
//! VKeyCode/NativeCode; those fields can contain macOS codes even in bundled profiles.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{File, OpenOptions},
    io::{self, Write},
    mem::{offset_of, size_of},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    thread,
    time::{Duration, Instant},
};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_LEFTALT: u16 = 56;
const KEY_LEFTMETA: u16 = 125;
const QT_UNKNOWN: i64 = 0x01ff_ffff;
const LETTERS: [u16; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];
const DIGITS: [u16; 10] = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];
const NUMPAD: [u16; 10] = [82, 79, 80, 81, 75, 76, 77, 71, 72, 73];
const FUNCTION_KEYS: [u16; 24] = [
    59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88, 183, 184, 185, 186, 187, 188, 189, 190, 191,
    192, 193, 194,
];

/// Only actual keyboard keys supported by this module. No mouse/pointer buttons.
fn supported(code: u16) -> bool {
    matches!(code, 1..=83 | 86..=88 | 96..=100 | 102..=115 | 117 | 119 | 121 | 125..=127 | 138 | 163..=168 | 183..=194 | 207..=208)
}

#[derive(Default)]
struct Ownership {
    next: u64,
    holds: BTreeMap<u64, Vec<u16>>,
    counts: HashMap<u16, usize>,
    press_order: Vec<u16>,
}
impl Ownership {
    fn acquire(&mut self, keys: &[u16]) -> Result<(u64, Vec<u16>)> {
        ensure!(keys.len() <= 32, "Keyboard chord contains too many keys");
        ensure!(
            keys.iter().all(|&key| supported(key)),
            "Unsupported Linux key code in chord"
        );
        let token = self
            .next
            .checked_add(1)
            .context("Keyboard hold tokens exhausted")?;
        let mut unique = Vec::new();
        for &key in keys {
            if !unique.contains(&key) {
                unique.push(key);
            }
        }
        let mut pressed = Vec::new();
        for &key in &unique {
            let count = self.counts.entry(key).or_default();
            if *count == 0 {
                pressed.push(key);
                self.press_order.push(key);
            }
            *count += 1;
        }
        self.next = token;
        self.holds.insert(token, unique);
        Ok((token, pressed))
    }
    fn relinquish(&mut self, token: u64) -> Result<Vec<u16>> {
        let keys = self
            .holds
            .remove(&token)
            .context("Unknown or already released keyboard hold")?;
        let mut released = Vec::new();
        for key in keys.into_iter().rev() {
            let count = self
                .counts
                .get_mut(&key)
                .expect("owned key has a reference count");
            *count -= 1;
            if *count == 0 {
                self.counts.remove(&key);
                self.press_order.retain(|&pressed| pressed != key);
                released.push(key);
            }
        }
        Ok(released)
    }
    fn drain(&mut self) -> Vec<u16> {
        self.holds.clear();
        self.counts.clear();
        self.press_order.drain(..).rev().collect()
    }
}

/// Persistent virtual keyboard. A write failure closes the device; construct a new
/// instance to retry. Tokens belong to one instance and must not be shared across it.
pub struct Keyboard {
    device: Option<File>,
    ownership: Ownership,
}
impl Keyboard {
    /// Creates the keyboard and waits one second for userspace discovery, as in
    /// the kernel example. Call from the input worker, never under the host mutex.
    /// The grace period is not proof that a compositor has attached to the device.
    pub fn new() -> Result<Self> {
        let device = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")
            .context("Open /dev/uinput; keyboard injection needs device write permission")?;
        let fd = device.as_raw_fd();
        // SAFETY: The descriptor is owned/open and these ioctls take integer values.
        ioctl_result(unsafe {
            libc::ioctl(
                fd,
                libc::_IOW::<libc::c_int>(b'U' as u32, 100),
                i32::from(EV_KEY),
            )
        })?;
        for code in 1..=255u16 {
            if supported(code) {
                ioctl_result(unsafe {
                    libc::ioctl(
                        fd,
                        libc::_IOW::<libc::c_int>(b'U' as u32, 101),
                        i32::from(code),
                    )
                })?;
            }
        }
        // libc supplies the kernel ABI structures; all fields, including padding,
        // start zeroed before the kernel reads this buffer.
        let mut setup: libc::uinput_setup = unsafe { std::mem::zeroed() };
        setup.id.bustype = 0x06; // BUS_VIRTUAL; do not impersonate an Elgato USB device.
        setup.id.version = 1;
        for (dst, &src) in setup.name.iter_mut().zip(b"Deckforge virtual keyboard") {
            *dst = src as libc::c_char;
        }
        ioctl_result(unsafe {
            libc::ioctl(fd, libc::_IOW::<libc::uinput_setup>(b'U' as u32, 3), &setup)
        })
        .context("Configure uinput keyboard")?;
        ioctl_result(unsafe { libc::ioctl(fd, libc::_IO(b'U' as u32, 1)) })
            .context("Create uinput keyboard")?;
        let keyboard = Self {
            device: Some(device),
            ownership: Ownership::default(),
        };
        thread::sleep(Duration::from_secs(1));
        Ok(keyboard)
    }

    pub fn press(&mut self, keys: &[u16]) -> Result<u64> {
        ensure!(
            self.device.is_some(),
            "Keyboard device is closed after a previous failure"
        );
        let (token, pressed) = self.ownership.acquire(keys)?;
        // Ownership is recorded before writing so even a partial press is released
        // by failure cleanup. A repeated modifier is owned but is not re-pressed.
        if let Err(error) = self.emit(&pressed, 1) {
            self.close();
            return Err(error).context("Press keyboard chord; virtual device closed for cleanup");
        }
        Ok(token)
    }

    pub fn release(&mut self, token: u64) -> Result<()> {
        ensure!(
            self.device.is_some(),
            "Keyboard device is closed after a previous failure"
        );
        // Keep the full pre-release set available if the write fails partway.
        let before = self.ownership.press_order.clone();
        let released = self.ownership.relinquish(token)?;
        if let Err(error) = self.emit(&released, 0) {
            self.ownership.press_order = before;
            self.close();
            return Err(error).context("Release keyboard chord; virtual device closed for cleanup");
        }
        Ok(())
    }

    pub fn chord(&mut self, keys: &[u16]) -> Result<()> {
        let token = self.press(keys)?;
        self.release(token)
    }

    pub fn release_all(&mut self) -> Result<()> {
        ensure!(
            self.device.is_some(),
            "Keyboard device is closed after a previous failure"
        );
        let keys: Vec<_> = self.ownership.press_order.iter().rev().copied().collect();
        if let Err(error) = self.emit(&keys, 0) {
            self.close();
            return Err(error);
        }
        self.ownership.drain();
        Ok(())
    }

    fn emit(&mut self, keys: &[u16], value: i32) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let file = self.device.as_mut().context("Keyboard device is closed")?;
        write_frame(file, &event_frame(keys, value)).context("Write uinput event frame")
    }
    fn close(&mut self) {
        let keys = self.ownership.drain();
        if let Some(mut device) = self.device.take() {
            // Best effort explicit releases. Closing the uinput descriptor also
            // removes its virtual device if destruction or release writes fail.
            if !keys.is_empty() {
                let _ = write_frame(&mut device, &event_frame(&keys, 0));
            }
            unsafe {
                libc::ioctl(device.as_raw_fd(), libc::_IO(b'U' as u32, 2));
            }
        }
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        self.close();
    }
}

fn ioctl_result(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Construct complete native-ABI input_event records without reading Rust padding.
fn event_frame(keys: &[u16], value: i32) -> Vec<u8> {
    let size = size_of::<libc::input_event>();
    let mut bytes = vec![0u8; size * (keys.len() + 1)];
    for (index, &key) in keys.iter().enumerate() {
        let event = &mut bytes[index * size..(index + 1) * size];
        event[offset_of!(libc::input_event, type_)..offset_of!(libc::input_event, type_) + 2]
            .copy_from_slice(&EV_KEY.to_ne_bytes());
        event[offset_of!(libc::input_event, code)..offset_of!(libc::input_event, code) + 2]
            .copy_from_slice(&key.to_ne_bytes());
        event[offset_of!(libc::input_event, value)..offset_of!(libc::input_event, value) + 4]
            .copy_from_slice(&value.to_ne_bytes());
    }
    // Final zeroed record is EV_SYN / SYN_REPORT / 0. Timestamp is ignored by uinput.
    debug_assert_eq!(EV_SYN, 0);
    bytes
}
fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let mut offset = 0;
    let start = Instant::now();
    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "uinput wrote zero bytes",
                ));
            }
            Ok(size) if size % size_of::<libc::input_event>() != 0 => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "uinput wrote a partial input_event",
                ));
            }
            Ok(size) => offset += size,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                if start.elapsed() >= Duration::from_millis(100) {
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Translate one Hotkeys entry, never the entire array as a sequence. A sentinel
/// with supported modifier bits is a modifier-only chord; without them it is empty.
/// The caller selects the trigger's index.
pub fn imported_hotkey(settings: &Value, index: usize) -> Result<Vec<u16>> {
    let hotkey = settings["Hotkeys"]
        .as_array()
        .context("Hotkeys must be an array")?
        .get(index)
        .context("Hotkey index is outside the imported array")?;
    ensure!(hotkey.is_object(), "Hotkey must be an object");
    let qt = integer(hotkey, "QTKeyCode")?;
    let vk = integer(hotkey, "VKeyCode")?;
    let native = integer(hotkey, "NativeCode")?;
    let flags = match hotkey.get("KeyModifiers") {
        None => 0,
        Some(value) => modifier_mask(value)?,
    };
    let mut modifiers = Vec::new();
    for (name, bit, key) in [
        ("KeyCtrl", 2, KEY_LEFTCTRL),
        ("KeyShift", 1, KEY_LEFTSHIFT),
        ("KeyOption", 4, KEY_LEFTALT),
        ("KeyCmd", 8, KEY_LEFTMETA),
    ] {
        let enabled = match hotkey.get(name) {
            Some(value) => {
                let enabled = value
                    .as_bool()
                    .with_context(|| format!("{name} must be a boolean"))?;
                if hotkey.get("KeyModifiers").is_some() {
                    ensure!(
                        enabled == (flags & bit != 0),
                        "{name} contradicts KeyModifiers"
                    );
                }
                enabled
            }
            None => flags & bit != 0,
        };
        if enabled {
            modifiers.push(key);
        }
    }
    if qt.is_none() || qt == Some(QT_UNKNOWN) || qt == Some(-1) {
        let absent = matches!(vk, None | Some(-1) | Some(0))
            && matches!(native, None | Some(-1) | Some(0) | Some(146));
        ensure!(
            absent && flags & !0xf == 0,
            "Hotkey has no portable Qt identity; platform-local key codes need an explicit mapping"
        );
        // The original dial/system branch permits an unknown Qt key with a
        // nonempty modifier record. Only the four recovered low bits are mapped.
        return Ok(modifiers);
    }
    let qt = qt.unwrap();
    let identity_flags = flags & !0xf;
    let key = match identity_flags {
        0 => qt_key(qt)?,
        0x4000 | 0x24000 => {
            let native = native.context("Keypad identity is missing NativeCode")?;
            ensure!(
                matches!(vk, None | Some(-1)) || vk == Some(native),
                "Conflicting keypad native and virtual codes"
            );
            observed_keypad(qt, native, identity_flags)?
        }
        0x8000_0000 => match (qt, native) {
            (44, Some(188)) => 51, // Observed Windows VK_OEM_COMMA -> KEY_COMMA.
            (61, Some(187)) => 13, // VK_OEM_PLUS, unshifted '=' -> KEY_EQUAL.
            (46, Some(190)) => 52, // VK_OEM_PERIOD -> KEY_DOT.
            _ => bail!("Unrecognized imported OEM key identity"),
        },
        _ => bail!("Unsupported imported key identity flags {identity_flags:#x}"),
    };
    if qt == 0x0100_0002 && !modifiers.contains(&KEY_LEFTSHIFT) {
        modifiers.push(KEY_LEFTSHIFT);
    }
    if !modifiers.contains(&key) {
        modifiers.push(key);
    }
    Ok(modifiers)
}

fn integer(value: &Value, field: &str) -> Result<Option<i64>> {
    value
        .get(field)
        .map(|v| {
            v.as_i64()
                .with_context(|| format!("{field} must be an integer"))
        })
        .transpose()
}
fn modifier_mask(value: &Value) -> Result<u32> {
    if let Some(number) = value.as_u64().filter(|&n| n <= u32::MAX as u64) {
        return Ok(number as u32);
    }
    if let Some(number) = value
        .as_i64()
        .filter(|&n| (i32::MIN as i64..0).contains(&n))
    {
        return Ok(number as i32 as u32);
    }
    bail!("KeyModifiers must be a 32-bit unsigned mask or signed bit pattern")
}
fn observed_keypad(qt: i64, native: i64, flags: u32) -> Result<u16> {
    if (48..=57).contains(&qt) && native == qt - 48 + 0x60 && flags == 0x4000 {
        return Ok(NUMPAD[(qt - 48) as usize]);
    }
    Ok(match (qt, native, flags) {
        (43, 107, 0x4000) => 78,  // KEY_KPPLUS
        (45, 109, 0x4000) => 74,  // KEY_KPMINUS
        (42, 106, 0x4000) => 55,  // KEY_KPASTERISK
        (47, 111, 0x24000) => 98, // KEY_KPSLASH, observed extended flag.
        _ => bail!("Unsupported or contradictory imported keypad identity"),
    })
}
fn qt_key(qt: i64) -> Result<u16> {
    if (65..=90).contains(&qt) {
        return Ok(LETTERS[(qt - 65) as usize]);
    }
    if (48..=57).contains(&qt) {
        return Ok(DIGITS[(qt - 48) as usize]);
    }
    if (0x0100_0030..=0x0100_0047).contains(&qt) {
        return Ok(FUNCTION_KEYS[(qt - 0x0100_0030) as usize]);
    }
    Ok(match qt {
        32 => 57,
        39 => 40,
        44 => 51,
        45 => 12,
        46 => 52,
        47 => 53,
        59 => 39,
        61 => 13,
        91 => 26,
        92 => 43,
        93 => 27,
        96 => 41,
        0x0100_0000 => 1,                // Escape
        0x0100_0001 | 0x0100_0002 => 15, // Tab / Backtab, Shift added by caller.
        0x0100_0003 => 14,               // Backspace
        0x0100_0004 => 28,               // Return: main keyboard
        0x0100_0005 => 96,               // Enter: numeric keypad, explicitly separate in Qt.
        0x0100_0006 => 110,
        0x0100_0007 => 111,              // Insert / Delete
        0x0100_0008 => 119,              // Pause
        0x0100_0009 | 0x0100_000a => 99, // Print / SysReq
        0x0100_0010 => 102,
        0x0100_0011 => 107, // Home / End
        0x0100_0012 => 105,
        0x0100_0013 => 103, // Left / Up
        0x0100_0014 => 106,
        0x0100_0015 => 108, // Right / Down
        0x0100_0016 => 104,
        0x0100_0017 => 109, // Page Up / Down
        0x0100_0020 => KEY_LEFTSHIFT,
        0x0100_0021 => KEY_LEFTCTRL,
        0x0100_0022 | 0x0100_0053 => KEY_LEFTMETA,
        0x0100_0054 => 126,
        0x0100_0023 => KEY_LEFTALT,
        0x0100_1103 => 100, // Alt / AltGr
        0x0100_0024 => 58,
        0x0100_0025 => 69,
        0x0100_0026 => 70,
        0x0100_0055 | 0x0100_1120 => 127, // Application menu / Compose
        0x0100_0058 => 138,               // Help
        0x0100_0070 => 114,
        0x0100_0071 => 113,
        0x0100_0072 => 115,
        0x0100_0080 => 207,
        0x0100_0081 => 166, // Media Play / Stop
        0x0100_0082 => 165,
        0x0100_0083 => 163, // Previous / Next
        0x0100_0084 => 167,
        0x0100_0086 => 164, // Record / PlayPause
        0x0100_00c5 => 168, // Audio Rewind
        0x0100_0102 => 208, // Audio Forward
        _ => bail!("Unsupported Qt key identity {qt:#x}; no keyboard event emitted"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn hotkey(qt: i64, vk: i64, native: i64, flags: u32) -> Value {
        json!({"Hotkeys":[{"QTKeyCode":qt,"VKeyCode":vk,"NativeCode":native,"KeyModifiers":flags}]})
    }
    #[test]
    fn qt_identity_wins_over_platform_local_codes() {
        assert_eq!(
            imported_hotkey(&hotkey(0x0100_0013, 126, 126, 0), 0).unwrap(),
            vec![103]
        );
        assert_eq!(
            imported_hotkey(&hotkey(59, 41, 41, 8), 0).unwrap(),
            vec![125, 39]
        );
        assert_eq!(
            imported_hotkey(&hotkey(32, 49, 49, 10), 0).unwrap(),
            vec![29, 125, 57]
        );
        assert_eq!(
            imported_hotkey(&hotkey(90, 90, 90, 3), 0).unwrap(),
            vec![29, 42, 44]
        );
    }
    #[test]
    fn sentinel_and_numeric_keypad_are_distinct() {
        assert!(
            imported_hotkey(&hotkey(QT_UNKNOWN, -1, 146, 0), 0)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            imported_hotkey(&hotkey(55, -1, 103, 0x4000), 0).unwrap(),
            vec![71]
        );
        assert_eq!(imported_hotkey(&hotkey(55, 55, 55, 0), 0).unwrap(), vec![8]);
        assert_eq!(
            imported_hotkey(&hotkey(47, -1, 111, 0x24000), 0).unwrap(),
            vec![98]
        );
        assert_eq!(
            imported_hotkey(&hotkey(46, -1, 190, 0x8000_0000), 0).unwrap(),
            vec![52]
        );
        assert_eq!(
            imported_hotkey(&hotkey(0x0100_0004, 13, 13, 0), 0).unwrap(),
            vec![28]
        );
        assert_eq!(
            imported_hotkey(&hotkey(0x0100_0005, -1, -1, 0), 0).unwrap(),
            vec![96]
        );
    }
    #[test]
    fn sentinel_can_hold_only_known_modifiers() {
        assert_eq!(
            imported_hotkey(&hotkey(QT_UNKNOWN, -1, 146, 3), 0).unwrap(),
            vec![29, 42]
        );
        assert_eq!(
            imported_hotkey(&hotkey(QT_UNKNOWN, -1, 146, 8), 0).unwrap(),
            vec![125]
        );
        assert!(imported_hotkey(&hotkey(QT_UNKNOWN, -1, 146, 0x4002), 0).is_err());
    }
    #[test]
    fn signed_masks_keep_their_bits_and_media_keys_are_supported() {
        let mut settings = hotkey(46, -1, 190, 0x8000_0000);
        settings["Hotkeys"][0]["KeyModifiers"] = json!(-2147483648i64);
        assert_eq!(imported_hotkey(&settings, 0).unwrap(), vec![52]);
        assert!(modifier_mask(&json!(-2147483649i64)).is_err());
        assert!(modifier_mask(&json!(4294967296u64)).is_err());
        assert!(modifier_mask(&json!(1.5)).is_err());
        for (qt, linux) in [(0x0100_0081, 166), (0x0100_00c5, 168), (0x0100_0102, 208)] {
            assert_eq!(
                imported_hotkey(&hotkey(qt, -1, -1, 0), 0).unwrap(),
                vec![linux]
            );
            assert!(supported(linux));
        }
    }
    #[test]
    fn unknowns_and_contradictions_do_not_become_shortcuts() {
        for settings in [
            hotkey(55, -1, 104, 0x4000),
            hotkey(55, -1, 103, 0x4001_0000),
            hotkey(QT_UNKNOWN, -1, 103, 0),
            hotkey(0x12345, -1, -1, 0),
        ] {
            assert!(imported_hotkey(&settings, 0).is_err());
        }
        let mut settings = hotkey(65, 65, 65, 2);
        settings["Hotkeys"][0]["KeyCtrl"] = json!(false);
        assert!(imported_hotkey(&settings, 0).is_err());
        assert!(imported_hotkey(&hotkey(65, 65, 65, 0), 1).is_err());
    }
    #[test]
    fn selected_array_entry_is_not_a_sequence_and_f24_is_available() {
        let settings = json!({"Hotkeys":[{"QTKeyCode":65},{"QTKeyCode":0x0100_0047}]});
        assert_eq!(imported_hotkey(&settings, 1).unwrap(), vec![194]);
        assert_eq!(
            imported_hotkey(&hotkey(0x0100_0002, -1, -1, 0), 0).unwrap(),
            vec![42, 15]
        );
    }
    #[test]
    fn overlapping_holds_keep_shared_modifiers_until_last_owner_releases() {
        let mut ownership = Ownership::default();
        let (first, pressed) = ownership.acquire(&[29, 30, 29]).unwrap();
        assert_eq!(pressed, vec![29, 30]);
        let (second, pressed) = ownership.acquire(&[29, 48]).unwrap();
        assert_eq!(pressed, vec![48]);
        assert_eq!(ownership.relinquish(first).unwrap(), vec![30]);
        assert_eq!(ownership.relinquish(second).unwrap(), vec![48, 29]);
        assert!(ownership.press_order.is_empty());
        assert!(ownership.relinquish(first).is_err());
        let before = ownership.next;
        assert!(ownership.acquire(&[29, 0xffff]).is_err());
        assert_eq!(ownership.next, before);
    }
    #[test]
    fn frames_have_native_key_records_then_one_syn_report() {
        let bytes = event_frame(&[29, 194], 1);
        let size = size_of::<libc::input_event>();
        assert_eq!(bytes.len(), size * 3);
        for (i, key) in [29u16, 194].iter().enumerate() {
            let event = &bytes[i * size..(i + 1) * size];
            assert!(
                event[..offset_of!(libc::input_event, type_)]
                    .iter()
                    .all(|&b| b == 0)
            );
            assert_eq!(
                &event[offset_of!(libc::input_event, type_)
                    ..offset_of!(libc::input_event, type_) + 2],
                &1u16.to_ne_bytes()
            );
            assert_eq!(
                &event
                    [offset_of!(libc::input_event, code)..offset_of!(libc::input_event, code) + 2],
                &key.to_ne_bytes()
            );
            assert_eq!(
                &event[offset_of!(libc::input_event, value)
                    ..offset_of!(libc::input_event, value) + 4],
                &1i32.to_ne_bytes()
            );
        }
        assert!(bytes[2 * size..].iter().all(|&b| b == 0));
    }
    #[test]
    fn short_writes_resume_only_at_event_boundaries() {
        struct Short {
            bytes: Vec<u8>,
            limit: usize,
        }
        impl Write for Short {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                let n = b.len().min(self.limit);
                self.bytes.extend_from_slice(&b[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let frame = event_frame(&[194], 1);
        let mut writer = Short {
            bytes: vec![],
            limit: size_of::<libc::input_event>(),
        };
        write_frame(&mut writer, &frame).unwrap();
        assert_eq!(writer.bytes, frame);
        let mut writer = Short {
            bytes: vec![],
            limit: 1,
        };
        assert!(write_frame(&mut writer, &frame).is_err());
    }
    #[test]
    fn failed_press_or_release_closes_writer_and_discards_ownership() {
        // /dev/full always rejects writes. This exercises cleanup without creating
        // an input device or delivering a shortcut to the desktop.
        let writer = || OpenOptions::new().write(true).open("/dev/full").unwrap();
        let mut keyboard = Keyboard {
            device: Some(writer()),
            ownership: Ownership::default(),
        };
        assert!(keyboard.press(&[KEY_LEFTCTRL, 194]).is_err());
        assert!(keyboard.device.is_none());
        assert!(keyboard.ownership.holds.is_empty());
        assert!(keyboard.ownership.press_order.is_empty());
        let mut keyboard = Keyboard {
            device: Some(writer()),
            ownership: Ownership::default(),
        };
        let (token, _) = keyboard.ownership.acquire(&[KEY_LEFTSHIFT, 194]).unwrap();
        assert!(keyboard.release(token).is_err());
        assert!(keyboard.device.is_none());
        assert!(keyboard.ownership.counts.is_empty());
        assert!(keyboard.ownership.press_order.is_empty());
    }
    #[test]
    fn uinput_f24_only_with_explicit_environment_gate() -> Result<()> {
        // Normal tests, including --include-ignored, never create an input device.
        if std::env::var("DECKFORGE_TEST_UINPUT").as_deref() != Ok("1") {
            return Ok(());
        }
        use std::{
            collections::BTreeSet,
            io::Read,
            path::{Path, PathBuf},
        };
        fn devices() -> io::Result<BTreeSet<PathBuf>> {
            Ok(std::fs::read_dir("/sys/class/input")?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("input"))
                        && std::fs::read_to_string(path.join("name"))
                            .is_ok_and(|name| name.trim() == "Deckforge virtual keyboard")
                })
                .collect())
        }
        fn removed(path: &Path) -> bool {
            let start = Instant::now();
            while path.exists() && start.elapsed() < Duration::from_secs(1) {
                thread::sleep(Duration::from_millis(10));
            }
            !path.exists()
        }
        let before = devices()?;
        let mut keyboard = match Keyboard::new() {
            Ok(keyboard) => keyboard,
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<io::Error>()
                        .is_some_and(|e| e.kind() == io::ErrorKind::PermissionDenied)
                }) =>
            {
                eprintln!(
                    "SKIP uinput F24 verification: existing /dev/uinput permissions do not permit creation"
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let after = devices()?;
        let created: Vec<_> = after.difference(&before).cloned().collect();
        ensure!(
            created.len() == 1,
            "Cannot identify exactly one newly created keyboard; no input emitted"
        );
        let sysfs = &created[0];
        // Confirm this new path also belongs to the exact fd owned by this test.
        let mut sysname = [0u8; 128];
        ioctl_result(unsafe {
            libc::ioctl(
                keyboard.device.as_ref().unwrap().as_raw_fd(),
                libc::_IOR::<[u8; 128]>(b'U' as u32, 44),
                sysname.as_mut_ptr(),
            )
        })?;
        let end = sysname
            .iter()
            .position(|&b| b == 0)
            .context("Invalid uinput sysname")?;
        ensure!(
            sysfs.file_name().unwrap().as_encoded_bytes() == &sysname[..end],
            "uinput fd does not match newly created sysfs device"
        );
        let event = std::fs::read_dir(sysfs)?
            .filter_map(|entry| entry.ok())
            .find(|entry| entry.file_name().to_string_lossy().starts_with("event"))
            .context("New keyboard has no event node")?;
        let path = Path::new("/dev/input").join(event.file_name());
        let mut reader = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(reader) => reader,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "SKIP uinput F24 verification: cannot read own event node {}; no input emitted",
                    path.display()
                );
                drop(keyboard);
                ensure!(removed(sysfs), "Virtual keyboard remained after Drop");
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        // Exclusive grab applies to this test's new virtual keyboard only. No
        // existing physical device is touched, and no event is emitted before it.
        if let Err(error) = ioctl_result(unsafe {
            libc::ioctl(
                reader.as_raw_fd(),
                libc::_IOW::<libc::c_int>(b'E' as u32, 0x90),
                1,
            )
        }) {
            eprintln!(
                "SKIP uinput F24 verification: cannot exclusively grab own event node: {error}; no input emitted"
            );
            drop(keyboard);
            ensure!(removed(sysfs), "Virtual keyboard remained after Drop");
            return Ok(());
        }
        let mut buffer = [0u8; 4096];
        while reader.read(&mut buffer).is_ok_and(|n| n > 0) {}
        // Two owners of the harmless F24 identity must produce only one balanced
        // physical press/release pair. No modifier or desktop shortcut is sent.
        let first = keyboard.press(&[194])?;
        let second = keyboard.press(&[194])?;
        keyboard.release(first)?;
        keyboard.release(second)?;
        let mut keys = Vec::new();
        let mut syns = 0;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(1) && (keys.len() < 2 || syns < 2) {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(bytes) => {
                    ensure!(
                        bytes % size_of::<libc::input_event>() == 0,
                        "Truncated evdev input_event"
                    );
                    for frame in buffer[..bytes]
                        .as_chunks::<{ size_of::<libc::input_event>() }>()
                        .0
                    {
                        let field16 =
                            |offset| u16::from_ne_bytes([frame[offset], frame[offset + 1]]);
                        let kind = field16(offset_of!(libc::input_event, type_));
                        let code = field16(offset_of!(libc::input_event, code));
                        let offset = offset_of!(libc::input_event, value);
                        let value =
                            i32::from_ne_bytes(frame[offset..offset + 4].try_into().unwrap());
                        if kind == EV_KEY {
                            keys.push((code, value));
                        }
                        if kind == EV_SYN && code == 0 {
                            syns += 1;
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error.into()),
            }
        }
        drop(keyboard);
        ensure!(removed(sysfs), "Virtual keyboard remained after Drop");
        ensure!(
            keys == vec![(194, 1), (194, 0)] && syns == 2,
            "Unexpected own-device events: keys={keys:?}, syns={syns}"
        );
        eprintln!(
            "PASS: own grabbed {} received F24 down/up and two SYN_REPORT frames; duplicate hold ownership balanced; virtual device removed. This proves kernel event delivery, not compositor shortcuts.",
            path.display()
        );
        Ok(())
    }
}
