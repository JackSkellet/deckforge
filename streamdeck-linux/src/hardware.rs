//! Stream Deck + HID transport, written against Elgato's protocol documentation.
//! Sources: https://docs.elgato.com/streamdeck/hid/general/
//! and https://docs.elgato.com/streamdeck/hid/stream-deck-plus/.
//! This module never resets the device or writes firmware.

use anyhow::{Context, Result, bail, ensure};
use hidapi::{HidApi, HidDevice};
use image::{DynamicImage, codecs::jpeg::JpegEncoder, imageops::FilterType};

const VENDOR: u16 = 0x0fd9;
const PRODUCT: u16 = 0x0084;
const REPORT_SIZE: usize = 1024;
const IMAGE_CHUNK_SIZE: usize = REPORT_SIZE - 8;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: String,
    pub serial: Option<String>,
    pub product: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key { index: u8, pressed: bool },
    DialPress { index: u8, pressed: bool },
    DialRotate { index: u8, ticks: i8 },
    Touch { x: u16, y: u16, long: bool },
    Swipe { start: (u16, u16), end: (u16, u16) },
}

#[derive(Default)]
struct InputState {
    keys: [bool; 8],
    dials: [bool; 4],
}

pub struct Deck {
    device: HidDevice,
    serial: String,
    input: InputState,
}

// Firmware2.0.3.4 can return the previous value immediately after a setter.
// Retry only readback, not the write, with a bounded100ms settling window.
fn verify_sleep_readback(expected: u32, mut read: impl FnMut() -> Result<u32>) -> Result<()> {
    let mut observed = 0;
    for attempt in 0..=20 {
        observed = read()?;
        if observed == expected {
            return Ok(());
        }
        if attempt < 20 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    bail!("sleep duration readback mismatch: requested {expected}, observed {observed}")
}

/// Prefer a descriptor match; only probe descriptor-less devices when needed to
/// identify a requested serial. Probes read serial metadata, never images/input.
fn select_candidate<T>(
    requested: Option<&str>,
    serials: &[Option<&str>],
    mut open: impl FnMut(usize) -> Result<(String, T)>,
) -> Result<T> {
    ensure!(
        !serials.is_empty(),
        "no matching Stream Deck + found (USB 0fd9:0084)"
    );
    let exact: Vec<_> = serials
        .iter()
        .enumerate()
        .filter(|(_, serial)| requested.is_none_or(|wanted| **serial == Some(wanted)))
        .map(|(index, _)| index)
        .collect();
    ensure!(
        exact.len() <= 1,
        "multiple Stream Deck + devices found; select a unique serial number"
    );
    if let Some(index) = exact.first() {
        let (serial, device) = open(*index)?;
        ensure!(
            !serial.is_empty() && requested.is_none_or(|wanted| wanted == serial),
            "device serial does not match selection"
        );
        return Ok(device);
    }
    let wanted = requested.context("No device selected")?;
    let mut matched = None;
    let mut first_error = None;
    for (index, _) in serials
        .iter()
        .enumerate()
        .filter(|(_, serial)| serial.is_none())
    {
        match open(index) {
            Ok((serial, device)) if serial == wanted => {
                ensure!(
                    matched.is_none(),
                    "multiple Stream Deck + devices have the selected serial"
                );
                matched = Some(device);
            }
            Ok(_) => {}
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(device) = matched {
        return Ok(device);
    }
    if let Some(error) = first_error {
        return Err(error.context("Could not identify a Stream Deck + with the selected serial"));
    }
    bail!("no matching Stream Deck + found (USB 0fd9:0084)")
}

impl Deck {
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        let api = HidApi::new().context("initialize HID enumeration")?;
        Ok(api
            .device_list()
            .filter(|d| d.vendor_id() == VENDOR && d.product_id() == PRODUCT)
            .map(|d| DeviceInfo {
                path: d.path().to_string_lossy().into_owned(),
                serial: d
                    .serial_number()
                    .filter(|serial| !serial.is_empty())
                    .map(str::to_owned),
                product: d.product_string().map(str::to_owned),
            })
            .collect())
    }

    /// Refuses ambiguous selection instead of choosing an arbitrary connected deck.
    pub fn connect(serial: Option<&str>) -> Result<Self> {
        let api = HidApi::new().context("initialize HID")?;
        let devices: Vec<_> = api
            .device_list()
            .filter(|d| d.vendor_id() == VENDOR && d.product_id() == PRODUCT)
            .collect();
        let serials: Vec<_> = devices
            .iter()
            .map(|d| d.serial_number().filter(|s| !s.is_empty()))
            .collect();
        select_candidate(serial, &serials, |index| {
            let info = devices[index];
            let device = info
                .open_device(&api)
                .context("open Stream Deck +; check hidraw permissions")?;
            let serial = match serials[index] {
                Some(serial) => serial.to_owned(),
                None => {
                    let mut report = [0u8; 32];
                    report[0] = 0x06;
                    let size = device
                        .get_feature_report(&mut report)
                        .context("read Stream Deck + serial")?;
                    ensure!(size >= 2 && report[0] == 0x06, "invalid serial response");
                    let len = usize::from(report[1]);
                    ensure!(len > 0 && len <= size - 2, "truncated serial response");
                    ascii_string(&report[2..2 + len])?
                }
            };
            Ok((
                serial.clone(),
                Self {
                    device,
                    serial,
                    input: InputState::default(),
                },
            ))
        })
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Reads the primary AP2 firmware version, without changing device state.
    pub fn firmware_version(&self) -> Result<String> {
        let mut report = [0u8; 32];
        report[0] = 0x05;
        let size = self
            .device
            .get_feature_report(&mut report)
            .context("read AP2 firmware version")?;
        ensure!(
            size >= 14 && report[0] == 0x05 && report[1] == 0x0c,
            "invalid firmware response"
        );
        ascii_string(&report[6..14])
    }

    /// HID acceptance only: the protocol documents no brightness getter.
    pub fn set_brightness(&self, brightness: u8) -> Result<()> {
        self.device
            .send_feature_report(&brightness_report(brightness)?)
            .context("set Stream Deck + backlight brightness")
    }

    /// Read the documented idle sleep duration. Zero disables device sleep.
    pub fn sleep_seconds(&self) -> Result<u32> {
        let mut report = [0u8; 32];
        report[0] = 0x0a;
        let size = self
            .device
            .get_feature_report(&mut report)
            .context("read Stream Deck + sleep duration")?;
        ensure!(size <= report.len(), "oversized sleep duration response");
        decode_sleep_duration(&report[..size])
    }

    /// Set and verify the documented sleep duration. Never changes firmware.
    pub fn set_sleep_seconds(&self, seconds: u32) -> Result<()> {
        self.device
            .send_feature_report(&sleep_report(seconds)?)
            .context("set Stream Deck + sleep duration")?;
        verify_sleep_readback(seconds, || self.sleep_seconds())
    }

    /// Returns state transitions only. Timeout returns an empty vector.
    /// A 50 ms timeout is recommended by Elgato. Negative timeouts are rejected.
    pub fn read_events(&mut self, timeout_ms: i32) -> Result<Vec<InputEvent>> {
        ensure!(timeout_ms >= 0, "input timeout must be nonnegative");
        let mut report = [0u8; 512];
        let size = self
            .device
            .read_timeout(&mut report, timeout_ms)
            .context("read Stream Deck + input")?;
        if size == 0 {
            return Ok(Vec::new());
        }
        self.input.decode(&report[..size])
    }

    /// Resizes to 120x120 and encodes RGB JPEG. Stream Deck + needs no rotation.
    pub fn set_key_image(&self, key: u8, image: &DynamicImage) -> Result<()> {
        ensure!(key < 8, "key index must be between 0 and 7");
        let jpeg = encode_jpeg(image, 120, 120)?;
        self.write_image(0x07, key, &jpeg)
    }

    /// Resizes to the complete 800x100 touch strip, without rotating the image.
    pub fn set_touchscreen_image(&self, image: &DynamicImage) -> Result<()> {
        let jpeg = encode_jpeg(image, 800, 100)?;
        self.write_image(0x0b, 0, &jpeg)
    }

    fn write_image(&self, command: u8, key: u8, jpeg: &[u8]) -> Result<()> {
        // Build all packets before writing, so validation cannot fail mid-transfer.
        for report in image_reports(command, key, jpeg)? {
            let written = self.device.write(&report).context("write image chunk")?;
            ensure!(
                written == REPORT_SIZE,
                "short HID image write: {written}/{REPORT_SIZE}"
            );
        }
        Ok(())
    }
}

fn ascii_string(bytes: &[u8]) -> Result<String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    ensure!(
        !bytes.is_empty() && bytes.iter().all(u8::is_ascii_graphic),
        "empty or non-ASCII device string"
    );
    Ok(String::from_utf8(bytes.to_vec())?)
}

// Main protocol setter reports are exactly 32 bytes, padded with zeroes.
fn brightness_report(brightness: u8) -> Result<[u8; 32]> {
    ensure!(brightness <= 100, "brightness must be between 0 and 100");
    let mut report = [0; 32];
    report[..3].copy_from_slice(&[0x03, 0x08, brightness]);
    Ok(report)
}
fn sleep_report(seconds: u32) -> Result<[u8; 32]> {
    let seconds = i32::try_from(seconds).context("sleep duration exceeds signed 32-bit seconds")?;
    let mut report = [0; 32];
    report[..2].copy_from_slice(&[0x03, 0x0d]);
    report[2..6].copy_from_slice(&seconds.to_le_bytes());
    Ok(report)
}
fn decode_sleep_duration(report: &[u8]) -> Result<u32> {
    ensure!(
        (6..=32).contains(&report.len()) && report[0] == 0x0a && report[1] == 4,
        "invalid sleep duration response"
    );
    let seconds = i32::from_le_bytes(report[2..6].try_into().unwrap());
    u32::try_from(seconds).context("device returned a negative sleep duration")
}

fn encode_jpeg(image: &DynamicImage, width: u32, height: u32) -> Result<Vec<u8>> {
    ensure!(image.width() > 0 && image.height() > 0, "image is empty");
    let resized = image
        .resize_exact(width, height, FilterType::Lanczos3)
        .to_rgb8();
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, 90).encode(
        resized.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(bytes)
}

fn image_reports(command: u8, key: u8, jpeg: &[u8]) -> Result<Vec<[u8; REPORT_SIZE]>> {
    ensure!(
        (command == 0x07 && key < 8) || (command == 0x0b && key == 0),
        "invalid image command or key"
    );
    ensure!(!jpeg.is_empty(), "empty image transfer");
    let count = jpeg.len().div_ceil(IMAGE_CHUNK_SIZE);
    ensure!(
        count <= usize::from(u16::MAX) + 1,
        "image exceeds HID chunk index range"
    );
    Ok(jpeg
        .chunks(IMAGE_CHUNK_SIZE)
        .enumerate()
        .map(|(index, chunk)| {
            let mut report = [0u8; REPORT_SIZE];
            report[0] = 0x02;
            report[1] = command;
            report[2] = key;
            report[3] = u8::from(index + 1 == count);
            report[4..6].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
            report[6..8].copy_from_slice(&(index as u16).to_le_bytes());
            report[8..8 + chunk.len()].copy_from_slice(chunk);
            report
        })
        .collect())
}

impl InputState {
    fn decode(&mut self, report: &[u8]) -> Result<Vec<InputEvent>> {
        ensure!(
            report.len() >= 4 && report.len() <= 512,
            "invalid input report size"
        );
        ensure!(report[0] == 0x01, "unexpected input report ID");
        let size = usize::from(u16::from_le_bytes([report[2], report[3]]));
        ensure!(size <= report.len() - 4, "truncated input payload");
        let data = &report[4..4 + size];
        let mut events = Vec::new();
        match report[1] {
            0x00 => {
                ensure!(size == 8, "expected eight key states");
                ensure!(data.iter().all(|&v| v <= 1), "invalid key state");
                for (index, (&value, old)) in data.iter().zip(&mut self.keys).enumerate() {
                    let pressed = value == 1;
                    if pressed != *old {
                        events.push(InputEvent::Key {
                            index: index as u8,
                            pressed,
                        });
                        *old = pressed;
                    }
                }
            }
            0x03 => {
                ensure!(size == 5, "expected four dial states and subtype");
                match data[0] {
                    0x00 => {
                        ensure!(
                            data[1..].iter().all(|&v| v <= 1),
                            "invalid dial button state"
                        );
                        for (index, (&value, old)) in
                            data[1..].iter().zip(&mut self.dials).enumerate()
                        {
                            let pressed = value == 1;
                            if pressed != *old {
                                events.push(InputEvent::DialPress {
                                    index: index as u8,
                                    pressed,
                                });
                                *old = pressed;
                            }
                        }
                    }
                    0x01 => {
                        for (index, &value) in data[1..].iter().enumerate() {
                            if value != 0 {
                                events.push(InputEvent::DialRotate {
                                    index: index as u8,
                                    ticks: value as i8,
                                });
                            }
                        }
                    }
                    other => bail!("unknown dial subtype: {other:#04x}"),
                }
            }
            0x02 => {
                ensure!(!data.is_empty(), "missing touch subtype");
                match data[0] {
                    0x01 | 0x02 => {
                        ensure!(size == 0x0a, "invalid tap payload length");
                        let (x, y) = touch_position(data, 2)?;
                        events.push(InputEvent::Touch {
                            x,
                            y,
                            long: data[0] == 0x02,
                        });
                    }
                    0x03 => {
                        ensure!(size == 0x0e, "invalid swipe payload length");
                        events.push(InputEvent::Swipe {
                            start: touch_position(data, 2)?,
                            end: touch_position(data, 6)?,
                        });
                    }
                    other => bail!("unknown touch subtype: {other:#04x}"),
                }
            }
            other => bail!("unknown input command: {other:#04x}"),
        }
        Ok(events)
    }
}

fn touch_position(data: &[u8], offset: usize) -> Result<(u16, u16)> {
    let x = u16::from_le_bytes([data[offset], data[offset + 1]]);
    let y = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
    ensure!(
        x < 800 && y < 100,
        "touch coordinates outside 800x100 window"
    );
    Ok((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sleep_setter_readback_tolerates_stale_values_but_bounds_mismatch() {
        let mut calls = 0;
        verify_sleep_readback(60, || {
            calls += 1;
            Ok(if calls < 3 { 0 } else { 60 })
        })
        .unwrap();
        assert_eq!(calls, 3);
        let mut calls = 0;
        assert!(
            verify_sleep_readback(60, || {
                calls += 1;
                Ok(0)
            })
            .is_err()
        );
        assert_eq!(calls, 21);
    }
    #[test]
    #[ignore = "Explicit serial gate; read-only metadata query on exclusively owned device"]
    fn real_device_sleep_duration_read_only() {
        let Ok(serial) = std::env::var("DECKFORGE_REAL_DEVICE_SLEEP_SERIAL") else {
            return;
        };
        let deck = Deck::connect(Some(&serial)).unwrap();
        let value = deck.sleep_seconds().unwrap();
        eprintln!(
            "Read-only sleep duration: {value} seconds, device {}",
            deck.serial()
        );
        if let Ok(expected) = std::env::var("DECKFORGE_REAL_DEVICE_SLEEP_EXPECT") {
            assert_eq!(value, expected.parse::<u32>().unwrap());
        }
    }

    #[test]
    #[ignore = "Requires explicit serial gate and exclusive ownership of the real deck"]
    fn real_device_sleep_duration_roundtrip_and_restore() {
        let Ok(serial) = std::env::var("DECKFORGE_REAL_DEVICE_SLEEP_SERIAL") else {
            return;
        };
        let deck = Deck::connect(Some(&serial)).unwrap();
        let original = deck.sleep_seconds().unwrap();
        struct Restore<'a> {
            deck: &'a Deck,
            original: u32,
            active: bool,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                if self.active
                    && let Err(error) = self.deck.set_sleep_seconds(self.original)
                {
                    eprintln!(
                        "Failed to restore original device sleep timer {}: {error:#}",
                        self.original
                    );
                }
            }
        }
        let mut restore = Restore {
            deck: &deck,
            original,
            active: true,
        };
        let temporary = if original <= i32::MAX as u32 - 60 {
            original + 60
        } else {
            original - 60
        };
        deck.set_sleep_seconds(temporary).unwrap();
        assert_eq!(deck.sleep_seconds().unwrap(), temporary);
        deck.set_sleep_seconds(original).unwrap();
        assert_eq!(deck.sleep_seconds().unwrap(), original);
        restore.active = false;
        eprintln!(
            "Device {} sleep timer: {original} -> {temporary} -> {original} seconds, each getter-verified. No brightness/image/reset/firmware writes.",
            deck.serial()
        );
    }

    #[test]
    fn control_feature_packets_match_documented_offsets_padding_and_limits() {
        for brightness in [0, 37, 100] {
            let report = brightness_report(brightness).unwrap();
            assert_eq!(&report[..3], &[3, 8, brightness]);
            assert_eq!(&report[3..], &[0; 29]);
        }
        assert!(brightness_report(101).is_err());
        for seconds in [0, 60, 0x01020304, i32::MAX as u32] {
            let report = sleep_report(seconds).unwrap();
            assert_eq!(&report[..2], &[3, 13]);
            assert_eq!(&report[2..6], &(seconds as i32).to_le_bytes());
            assert_eq!(&report[6..], &[0; 26]);
        }
        assert!(sleep_report(i32::MAX as u32 + 1).is_err());
    }
    #[test]
    fn sleep_duration_decode_rejects_negative_truncated_or_malformed_reports() {
        for seconds in [0i32, 600, i32::MAX] {
            let mut report = [0u8; 32];
            report[..2].copy_from_slice(&[10, 4]);
            report[2..6].copy_from_slice(&seconds.to_le_bytes());
            assert_eq!(decode_sleep_duration(&report).unwrap(), seconds as u32);
            assert_eq!(decode_sleep_duration(&report[..6]).unwrap(), seconds as u32);
            assert!((0..6).all(|length| decode_sleep_duration(&report[..length]).is_err()));
            for length in [0, 3, 5, 30] {
                report[1] = length;
                assert!(decode_sleep_duration(&report).is_err());
            }
        }
        assert!(decode_sleep_duration(&[10, 4, 255, 255, 255, 255]).is_err());
        assert!(decode_sleep_duration(&[11, 4, 0, 0, 0, 0]).is_err());
        assert!(decode_sleep_duration(&[0; 33]).is_err());
    }
    #[test]
    fn descriptor_missing_serial_can_be_selected_after_first_connection() {
        let serials = [None];
        let first =
            select_candidate(None, &serials, |_| Ok(("feature-serial".into(), "opened"))).unwrap();
        assert_eq!(first, "opened");
        let reconnected = select_candidate(Some("feature-serial"), &serials, |_| {
            Ok(("feature-serial".into(), "reopened"))
        })
        .unwrap();
        assert_eq!(reconnected, "reopened");
        assert!(
            select_candidate(Some("different"), &serials, |_| Ok((
                "feature-serial".into(),
                ()
            )))
            .is_err()
        );
    }
    #[test]
    fn exact_descriptor_match_does_not_probe_unrelated_devices() {
        let found = select_candidate(
            Some("wanted"),
            &[Some("other"), Some("wanted"), None],
            |index| {
                assert_eq!(index, 1);
                Ok(("wanted".into(), 17))
            },
        )
        .unwrap();
        assert_eq!(found, 17);
        let found = select_candidate(Some("wanted"), &[Some("other"), None], |index| {
            assert_eq!(index, 1);
            Ok(("wanted".into(), 19))
        })
        .unwrap();
        assert_eq!(found, 19);
    }
    #[test]
    fn ambiguous_selection_and_unidentified_errors_remain_explicit() {
        assert!(
            select_candidate(None, &[None, None], |_| -> Result<(String, ())> {
                panic!("ambiguous devices must not be opened")
            })
            .is_err()
        );
        assert!(
            select_candidate(Some("same"), &[None, None], |_| Ok(("same".into(), ()))).is_err()
        );
        let error = select_candidate(Some("wanted"), &[None], |_| -> Result<(String, ())> {
            bail!("permission denied")
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("permission denied"));
    }

    fn input(command: u8, data: &[u8]) -> Vec<u8> {
        let mut report = vec![1, command, data.len() as u8, 0];
        report.extend_from_slice(data);
        report
    }

    #[test]
    fn key_edges_and_malformed_report_do_not_corrupt_state() {
        let mut state = InputState::default();
        let down = input(0, &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            state.decode(&down).unwrap(),
            vec![InputEvent::Key {
                index: 0,
                pressed: true
            }]
        );
        assert!(state.decode(&down).unwrap().is_empty());
        assert!(state.decode(&input(0, &[0, 0, 0, 0, 0, 0, 0, 2])).is_err());
        assert_eq!(
            state.decode(&input(0, &[0; 8])).unwrap(),
            vec![InputEvent::Key {
                index: 0,
                pressed: false
            }]
        );
    }

    #[test]
    fn dial_ticks_are_signed_and_buttons_are_separate() {
        let mut state = InputState::default();
        assert_eq!(
            state.decode(&input(3, &[1, 255, 0, 2, 128])).unwrap(),
            vec![
                InputEvent::DialRotate {
                    index: 0,
                    ticks: -1
                },
                InputEvent::DialRotate { index: 2, ticks: 2 },
                InputEvent::DialRotate {
                    index: 3,
                    ticks: -128
                },
            ]
        );
        assert_eq!(
            state.decode(&input(3, &[0, 0, 1, 0, 0])).unwrap(),
            vec![InputEvent::DialPress {
                index: 1,
                pressed: true
            }]
        );
        assert_eq!(
            state.decode(&input(3, &[0; 5])).unwrap(),
            vec![InputEvent::DialPress {
                index: 1,
                pressed: false
            }]
        );
    }

    #[test]
    fn touch_and_swipe_decode_little_endian_coordinates() {
        let mut state = InputState::default();
        assert_eq!(
            state
                .decode(&input(2, &[2, 0, 0x1f, 3, 99, 0, 0, 0, 0, 0]))
                .unwrap(),
            vec![InputEvent::Touch {
                x: 799,
                y: 99,
                long: true
            }]
        );
        assert_eq!(
            state
                .decode(&input(
                    2,
                    &[3, 0, 0x2c, 1, 20, 0, 0x58, 2, 30, 0, 0, 0, 0, 0]
                ))
                .unwrap(),
            vec![InputEvent::Swipe {
                start: (300, 20),
                end: (600, 30)
            }]
        );
        assert!(
            state
                .decode(&input(2, &[1, 0, 0x20, 3, 0, 0, 0, 0, 0, 0]))
                .is_err()
        );
    }

    #[test]
    fn malformed_packets_are_rejected_without_panics() {
        let mut state = InputState::default();
        for report in [
            vec![],
            vec![1, 0, 8, 0],
            vec![2, 0, 0, 0],
            input(0, &[0; 7]),
            input(3, &[1]),
            input(3, &[2, 0, 0, 0, 0]),
            input(2, &[3, 0]),
            input(99, &[]),
        ] {
            assert!(state.decode(&report).is_err(), "{report:?}");
        }
        // A padded HID report is valid; only the declared payload is parsed.
        let mut padded = input(0, &[0; 8]);
        padded.resize(512, 0);
        assert!(state.decode(&padded).unwrap().is_empty());
    }

    #[test]
    fn image_chunks_preserve_bytes_and_zero_pad_only_last_report() {
        let bytes: Vec<_> = (0..2500).map(|i| (i % 251) as u8).collect();
        let reports = image_reports(0x07, 7, &bytes).unwrap();
        assert_eq!(reports.len(), 3);
        let mut recovered = Vec::new();
        for (index, report) in reports.iter().enumerate() {
            assert_eq!(&report[..3], &[2, 7, 7]);
            assert_eq!(report[3], u8::from(index == 2));
            assert_eq!(u16::from_le_bytes([report[6], report[7]]), index as u16);
            let len = u16::from_le_bytes([report[4], report[5]]) as usize;
            recovered.extend_from_slice(&report[8..8 + len]);
            assert!(report[8 + len..].iter().all(|&b| b == 0));
        }
        assert_eq!(recovered, bytes);
        let exact = image_reports(0x0b, 0, &[42; IMAGE_CHUNK_SIZE * 2]).unwrap();
        assert_eq!(exact.len(), 2);
        assert_eq!(exact[1][3], 1);
        assert_eq!(&exact[1][4..6], &(IMAGE_CHUNK_SIZE as u16).to_le_bytes());
        assert!(image_reports(0x07, 8, &[1]).is_err());
        assert!(image_reports(0x0b, 0, &[]).is_err());
    }

    #[test]
    fn jpeg_encoder_produces_correct_dimensions_from_rgba_input() {
        let rgba = DynamicImage::new_rgba8(16, 16);
        for (width, height) in [(120, 120), (800, 100)] {
            let jpeg = encode_jpeg(&rgba, width, height).unwrap();
            let decoded = image::load_from_memory(&jpeg).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (width, height));
        }
    }
}
