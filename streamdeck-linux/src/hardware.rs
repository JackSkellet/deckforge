//! Model-aware HID transport written against Elgato's official protocol documentation.
//! Sources: https://docs.elgato.com/streamdeck/hid/general/
//! Family pages under that HID reference: mini/, stream-deck-classic/,
//! stream-deck-xl/, stream-deck-plus/, stream-deck-plus-xl/, stream-deck-neo/.
//! This module never resets the device or writes firmware.

mod legacy2017;
use crate::device_model::Model;
use anyhow::{Context, Result, bail, ensure};
use hidapi::{HidApi, HidDevice};
use image::{
    DynamicImage,
    codecs::{bmp::BmpEncoder, jpeg::JpegEncoder},
    imageops::FilterType,
};

const VENDOR: u16 = 0x0fd9;
const REPORT_SIZE: usize = 1024;
const IMAGE_CHUNK_SIZE: usize = REPORT_SIZE - 8;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: String,
    pub serial: Option<String>,
    pub product: Option<String>,
    pub model: Model,
    pub product_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key { index: u8, pressed: bool },
    Sensor { index: u8, pressed: bool },
    DialPress { index: u8, pressed: bool },
    DialRotate { index: u8, ticks: i8 },
    Touch { x: u16, y: u16, long: bool },
    Swipe { start: (u16, u16), end: (u16, u16) },
}

struct InputState {
    model: Model,
    keys: Vec<bool>,
    dials: Vec<bool>,
}
impl Default for InputState {
    fn default() -> Self {
        Self::new(Model::Plus)
    }
}
impl InputState {
    fn new(model: Model) -> Self {
        Self {
            model,
            keys: vec![false; usize::from(model.key_count() + model.sensor_count())],
            dials: vec![false; usize::from(model.dial_count())],
        }
    }
}

pub struct Deck {
    device: HidDevice,
    serial: String,
    model: Model,
    legacy_firmware: Option<legacy2017::Firmware>,
    input: InputState,
    trace: bool,
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
        "no matching supported Stream Deck found"
    );
    let exact: Vec<_> = serials
        .iter()
        .enumerate()
        .filter(|(_, serial)| requested.is_none_or(|wanted| **serial == Some(wanted)))
        .map(|(index, _)| index)
        .collect();
    ensure!(
        exact.len() <= 1,
        "multiple Stream Deck devices found; select a unique serial number"
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
                    "multiple Stream Deck devices have the selected serial"
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
        return Err(error.context("Could not identify a Stream Deck with the selected serial"));
    }
    bail!("no matching supported Stream Deck found")
}

impl Deck {
    pub fn enumerate() -> Result<Vec<DeviceInfo>> {
        let api = HidApi::new().context("initialize HID enumeration")?;
        Ok(api
            .device_list()
            .filter(|d| d.vendor_id() == VENDOR && Model::from_product(d.product_id()).is_some())
            .map(|d| DeviceInfo {
                path: d.path().to_string_lossy().into_owned(),
                serial: d
                    .serial_number()
                    .filter(|serial| !serial.is_empty())
                    .map(str::to_owned),
                product: d.product_string().map(str::to_owned),
                model: Model::from_product(d.product_id()).unwrap(),
                product_id: d.product_id(),
            })
            .collect())
    }

    /// Refuses ambiguous selection instead of choosing an arbitrary connected deck.
    pub fn connect(serial: Option<&str>) -> Result<Self> {
        Self::connect_with_model(serial, None)
    }
    pub fn connect_with_model(serial: Option<&str>, model: Option<Model>) -> Result<Self> {
        ensure!(
            model.is_none_or(|m| m.capabilities().transport_supported),
            "HID protocol for the requested model is not implemented"
        );
        let api = HidApi::new().context("initialize HID")?;
        let devices: Vec<_> = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == VENDOR
                    && Model::from_product(d.product_id())
                        .is_some_and(|m| model.is_none_or(|wanted| wanted == m))
            })
            .collect();
        let serials: Vec<_> = devices
            .iter()
            .map(|d| d.serial_number().filter(|s| !s.is_empty()))
            .collect();
        select_candidate(serial, &serials, |index| {
            let info = devices[index];
            let model = Model::from_product(info.product_id()).unwrap();
            let device = info
                .open_device(&api)
                .context("open Stream Deck; check hidraw permissions")?;
            let serial = match serials[index] {
                Some(serial) => serial.to_owned(),
                None => {
                    let mut report = [0u8; 32];
                    report[0] = if matches!(model, Model::Mini | Model::Classic2017) {
                        0x03
                    } else {
                        0x06
                    };
                    let size = device
                        .get_feature_report(&mut report)
                        .context("read Stream Deck serial")?;
                    ensure!(size <= report.len(), "oversized serial response");
                    decode_serial(model, &report[..size])?
                }
            };
            let legacy_firmware = if model == Model::Classic2017 {
                let mut report = [0u8; 32];
                report[0] = 4;
                let size = device
                    .get_feature_report(&mut report)
                    .context("read original 2017 firmware")?;
                ensure!(size <= report.len(), "oversized 2017 firmware response");
                Some(legacy2017::decode_firmware(&report[..size])?)
            } else {
                None
            };
            Ok((
                serial.clone(),
                Self {
                    device,
                    serial,
                    model,
                    legacy_firmware,
                    input: InputState::new(model),
                    trace: std::env::var_os("DECKFORGE_TRACE_HID").is_some(),
                },
            ))
        })
    }

    pub fn model(&self) -> Model {
        self.model
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Reads the primary AP2 firmware version, without changing device state.
    pub fn firmware_version(&self) -> Result<String> {
        if let Some(firmware) = &self.legacy_firmware {
            return Ok(firmware.version().to_owned());
        }
        let mut report = [0u8; 32];
        report[0] = if self.model == Model::Mini {
            0xa1
        } else {
            0x05
        };
        let size = self
            .device
            .get_feature_report(&mut report)
            .context("read AP2 firmware version")?;
        ensure!(size <= report.len(), "oversized firmware response");
        decode_firmware(self.model, &report[..size])
    }

    /// The 2017 firmware has a proven percentage-to-backlight curve.
    pub fn legacy_brightness_value(&self, percent: u8) -> Result<u8> {
        self.legacy_firmware
            .as_ref()
            .context("device is not original 2017")?
            .brightness_value(percent)
    }
    /// HID acceptance only: the protocol documents no brightness getter.
    pub fn set_brightness(&self, brightness: u8) -> Result<()> {
        self.device
            .send_feature_report(&model_brightness_report(self.model, brightness)?)
            .context("set Stream Deck backlight brightness")
    }

    /// Read the documented idle sleep duration. Zero disables device sleep.
    pub fn sleep_seconds(&self) -> Result<u32> {
        ensure!(
            self.model.capabilities().sleep,
            "model has no documented sleep timer"
        );
        let mut report = [0u8; 32];
        report[0] = if self.model == Model::Mini {
            0xa3
        } else {
            0x0a
        };
        let size = self
            .device
            .get_feature_report(&mut report)
            .context("read Stream Deck sleep duration")?;
        ensure!(size <= report.len(), "oversized sleep duration response");
        decode_model_sleep(self.model, &report[..size])
    }

    /// Set and verify the documented sleep duration. Never changes firmware.
    pub fn set_sleep_seconds(&self, seconds: u32) -> Result<()> {
        self.device
            .send_feature_report(&model_sleep_report(self.model, seconds)?)
            .context("set Stream Deck sleep duration")?;
        verify_sleep_readback(seconds, || self.sleep_seconds())
    }

    /// Returns state transitions only. Timeout returns an empty vector.
    /// A 50 ms timeout is recommended by Elgato. Negative timeouts are rejected.
    pub fn read_events(&mut self, timeout_ms: i32) -> Result<Vec<InputEvent>> {
        Ok(self.read_packet(timeout_ms, true)?.unwrap_or_default())
    }

    /// None means timeout. Some means a physical report arrived, even if decoding
    /// produces no transitions. Sleeping reads bypass decoding and retain InputState.
    pub fn read_packet(
        &mut self,
        timeout_ms: i32,
        decode: bool,
    ) -> Result<Option<Vec<InputEvent>>> {
        ensure!(timeout_ms >= 0, "input timeout must be nonnegative");
        let mut report = [0u8; 512];
        let size = self
            .device
            .read_timeout(&mut report, timeout_ms)
            .context("read Stream Deck input")?;
        ensure!(size <= report.len(), "oversized input report");
        self.input.packet(&report[..size], decode)
    }

    /// Encode at the model's physical key dimensions and required orientation.
    pub fn set_key_image(&self, key: u8, image: &DynamicImage) -> Result<()> {
        ensure!(
            self.model.capabilities().key_images,
            "This model has no key displays"
        );
        ensure!(
            key < self.model.key_count(),
            "key index outside device layout"
        );
        if let Some(firmware) = &self.legacy_firmware {
            let reports = firmware.key_reports(key, image)?;
            for (index, report) in reports.iter().enumerate() {
                if index > 0 {
                    std::thread::sleep(legacy2017::INTER_REPORT_DELAY);
                }
                let written = self
                    .device
                    .write(report)
                    .context("write original 2017 image chunk")?;
                ensure!(
                    written == legacy2017::REPORT_SIZE,
                    "short original 2017 image write: {written}"
                );
            }
            Ok(())
        } else if self.model == Model::Mini {
            let bmp = encode_mini_bmp(image)?;
            self.write_reports(&mini_image_reports(key, &bmp)?)
        } else {
            let jpeg = encode_model_jpeg(
                self.model,
                image,
                self.model.key_width(),
                self.model.key_height(),
            )?;
            self.write_image(0x07, key, &jpeg)
        }
    }
    /// Complete display window: a touch strip on Plus, non-touch infobar on Neo.
    pub fn set_touchscreen_image(&self, image: &DynamicImage) -> Result<()> {
        ensure!(
            self.model.capabilities().strip,
            "This model has no display window"
        );
        let jpeg = encode_model_jpeg(
            self.model,
            image,
            self.model.strip_width(),
            self.model.strip_height(),
        )?;
        self.write_image(0x0b, 0, &jpeg)
    }
    /// SDK Neo layouts are 232x50; its HID window is 248x58. Center the logical
    /// canvas with an 8x4 black gutter (Linux policy; native offset unverified).
    pub fn set_infobar_image(&self, image: &DynamicImage) -> Result<()> {
        ensure!(
            self.model == Model::Neo,
            "Infobar output requires Stream Deck Neo"
        );
        self.set_touchscreen_image(&neo_infobar_frame(image)?)
    }
    /// Neo appends its two 96x16 sensor displays at hardware indices8/9.
    /// They are deliberately separate from the eight SDK keypad positions.
    pub fn set_sensor_image(&self, sensor: u8, image: &DynamicImage) -> Result<()> {
        ensure!(
            self.model == Model::Neo && sensor < 2,
            "invalid sensor display"
        );
        let jpeg = encode_model_jpeg(self.model, image, 96, 16)?;
        self.write_image(0x07, self.model.key_count() + sensor, &jpeg)
    }
    pub fn set_lcd_image(&self, image: &DynamicImage) -> Result<()> {
        ensure!(
            self.model.capabilities().full_lcd,
            "Single-transfer full LCD images are unsupported for this model"
        );
        let jpeg = encode_model_jpeg(
            self.model,
            image,
            self.model.lcd_width(),
            self.model.lcd_height(),
        )?;
        self.write_image(0x08, 0, &jpeg)
    }
    fn write_image(&self, command: u8, key: u8, jpeg: &[u8]) -> Result<()> {
        let reports = model_image_reports(self.model, command, key, jpeg)?;
        self.write_reports(&reports)?;
        if self.trace {
            eprintln!(
                "HID image accepted: model={} command=0x{command:02x} key={key} chunks={} jpeg_bytes={}",
                self.model.id(),
                reports.len(),
                jpeg.len()
            );
        }
        Ok(())
    }
    fn write_reports(&self, reports: &[[u8; REPORT_SIZE]]) -> Result<()> {
        for report in reports {
            let written = self.device.write(report).context("write image chunk")?;
            ensure!(
                written == REPORT_SIZE,
                "short HID image write: {written}/{REPORT_SIZE}"
            );
        }
        Ok(())
    }
}

fn decode_serial(model: Model, report: &[u8]) -> Result<String> {
    if model == Model::Classic2017 {
        return legacy2017::decode_serial(report);
    }
    if model == Model::Mini {
        ensure!(
            (6..=32).contains(&report.len()) && report[0] == 0x03,
            "invalid Mini serial response"
        );
        ascii_string(&report[5..])
    } else {
        ensure!(
            (2..=32).contains(&report.len()) && report[0] == 0x06,
            "invalid serial response"
        );
        let len = usize::from(report[1]);
        ensure!(
            len > 0 && len <= report.len() - 2,
            "truncated serial response"
        );
        ascii_string(&report[2..2 + len])
    }
}
fn decode_firmware(model: Model, report: &[u8]) -> Result<String> {
    if model == Model::Classic2017 {
        return legacy2017::firmware_string(report);
    }
    if model == Model::Mini {
        ensure!(
            (17..=32).contains(&report.len()) && report[0] == 0xa1,
            "invalid Mini firmware response"
        );
        ascii_string(&report[5..17])
    } else {
        ensure!(
            (14..=32).contains(&report.len()) && report[0] == 0x05 && report[1] == 0x0c,
            "invalid firmware response"
        );
        ascii_string(&report[6..14])
    }
}
fn model_brightness_report(model: Model, brightness: u8) -> Result<[u8; 32]> {
    ensure!(
        model.capabilities().brightness,
        "model has no brightness control"
    );
    if model == Model::Classic2017 {
        legacy2017::brightness_report(brightness)
    } else if model == Model::Mini {
        ensure!(brightness <= 100, "brightness must be between 0 and 100");
        let mut report = [0; 32];
        report[..6].copy_from_slice(&[0x05, 0x55, 0xaa, 0xd1, 0x01, brightness]);
        Ok(report)
    } else {
        brightness_report(brightness)
    }
}
fn model_sleep_report(model: Model, seconds: u32) -> Result<[u8; 32]> {
    ensure!(
        model.capabilities().sleep,
        "model sleep control unavailable"
    );
    let mut report = sleep_report(seconds)?;
    if model == Model::Mini {
        report[..2].copy_from_slice(&[0x0b, 0xa2]);
    }
    Ok(report)
}
fn decode_model_sleep(model: Model, report: &[u8]) -> Result<u32> {
    if model == Model::Mini {
        ensure!(
            (6..=32).contains(&report.len()) && report[0] == 0xa3,
            "invalid Mini sleep response"
        );
        let mut normalized = report.to_vec();
        normalized[0] = 0x0a;
        decode_sleep_duration(&normalized)
    } else {
        decode_sleep_duration(report)
    }
}
fn neo_infobar_frame(image: &DynamicImage) -> Result<DynamicImage> {
    ensure!(image.width() > 0 && image.height() > 0, "image is empty");
    let mut frame = image::RgbaImage::from_pixel(248, 58, image::Rgba([0, 0, 0, 255]));
    let logical = image.resize_exact(232, 50, FilterType::Lanczos3).to_rgba8();
    image::imageops::overlay(&mut frame, &logical, 8, 4);
    Ok(DynamicImage::ImageRgba8(frame))
}
fn encode_model_jpeg(
    model: Model,
    image: &DynamicImage,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    ensure!(
        image.width() > 0 && image.height() > 0 && width > 0 && height > 0,
        "empty image dimensions"
    );
    let image = image.resize_exact(width, height, FilterType::Lanczos3);
    let image = match model {
        Model::Classic | Model::Xl | Model::Neo => image.rotate180(),
        Model::PlusXl => image.rotate270(),
        Model::Plus => image,
        _ => bail!("model does not use JPEG images"),
    };
    encode_jpeg(&image, image.width(), image.height())
}
fn encode_mini_bmp(image: &DynamicImage) -> Result<Vec<u8>> {
    ensure!(image.width() > 0 && image.height() > 0, "image is empty");
    let image = image
        .resize_exact(80, 80, FilterType::Lanczos3)
        .rotate90()
        .to_rgb8();
    let mut bytes = Vec::new();
    BmpEncoder::new(&mut bytes).encode(image.as_raw(), 80, 80, image::ExtendedColorType::Rgb8)?;
    Ok(bytes)
}
fn mini_image_reports(key: u8, bmp: &[u8]) -> Result<Vec<[u8; REPORT_SIZE]>> {
    ensure!(key < 6, "Mini key index must be between 0 and 5");
    ensure!(!bmp.is_empty(), "empty image transfer");
    const CHUNK: usize = REPORT_SIZE - 16;
    ensure!(
        bmp.len().div_ceil(CHUNK) <= 256,
        "Mini image exceeds chunk index range"
    );
    let count = bmp.len().div_ceil(CHUNK);
    Ok(bmp
        .chunks(CHUNK)
        .enumerate()
        .map(|(index, chunk)| {
            let mut report = [0; REPORT_SIZE];
            // Windows7.6 ESDHW20GAI9901Imp direct-key writer0x14062e920:
            // zero-based chunks; Show Image is enabled only on the final chunk.
            report[..6].copy_from_slice(&[
                0x02,
                0x01,
                index as u8,
                0,
                u8::from(index + 1 == count),
                key,
            ]);
            report[16..16 + chunk.len()].copy_from_slice(chunk);
            report
        })
        .collect())
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

#[cfg(test)]
fn image_reports(command: u8, key: u8, jpeg: &[u8]) -> Result<Vec<[u8; REPORT_SIZE]>> {
    model_image_reports(Model::Plus, command, key, jpeg)
}
fn model_image_reports(
    model: Model,
    command: u8,
    key: u8,
    jpeg: &[u8],
) -> Result<Vec<[u8; REPORT_SIZE]>> {
    ensure!(
        !matches!(model, Model::Mini | Model::Classic2017)
            && model.capabilities().transport_supported,
        "Model does not use the main image protocol"
    );
    ensure!(
        (command == 0x07
            && model.capabilities().key_images
            && key < model.key_count() + model.sensor_count())
            || (command == 0x08 && model.capabilities().full_lcd && key == 0)
            || (command == 0x0b && model.capabilities().strip && key == 0),
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
    fn key_events(&mut self, data: &[u8]) -> Result<Vec<InputEvent>> {
        ensure!(
            data.len() == self.keys.len(),
            "unexpected key/sensor payload length"
        );
        ensure!(data.iter().all(|&v| v <= 1), "invalid key or sensor state");
        let mut events = Vec::new();
        for (index, (&value, old)) in data.iter().zip(&mut self.keys).enumerate() {
            let pressed = value == 1;
            if pressed != *old {
                let index = index as u8;
                events.push(if index < self.model.key_count() {
                    InputEvent::Key { index, pressed }
                } else {
                    InputEvent::Sensor {
                        index: index - self.model.key_count(),
                        pressed,
                    }
                });
                *old = pressed;
            }
        }
        Ok(events)
    }
    fn packet(&mut self, report: &[u8], decode: bool) -> Result<Option<Vec<InputEvent>>> {
        if report.is_empty() {
            return Ok(None);
        }
        Ok(Some(if decode {
            self.decode(report)?
        } else {
            Vec::new()
        }))
    }
    fn decode(&mut self, report: &[u8]) -> Result<Vec<InputEvent>> {
        if self.model == Model::Classic2017 {
            return self.key_events(&legacy2017::decode_keys(report)?);
        }
        if self.model == Model::Mini {
            ensure!(
                report.len() == 65 && report[0] == 1,
                "invalid Mini input report"
            );
            return self.key_events(&report[1..7]);
        }
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
                return self.key_events(data);
            }
            0x03 => {
                ensure!(
                    !self.dials.is_empty() && size == self.dials.len() + 1,
                    "unexpected encoder payload length"
                );
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
                ensure!(
                    self.model.capabilities().touchscreen,
                    "model has no touch screen"
                );
                ensure!(!data.is_empty(), "missing touch subtype");
                match data[0] {
                    0x01 | 0x02 => {
                        // Plus XL's published TAP length is 0x10; long PRESS 0x0a.
                        let expected = if self.model == Model::PlusXl && data[0] == 1 {
                            0x10
                        } else {
                            0x0a
                        };
                        ensure!(size == expected, "invalid tap payload length");
                        let (x, y) = touch_position(self.model, data, 2)?;
                        events.push(InputEvent::Touch {
                            x,
                            y,
                            long: data[0] == 0x02,
                        });
                    }
                    0x03 => {
                        ensure!(size == 0x0e, "invalid swipe payload length");
                        events.push(InputEvent::Swipe {
                            start: touch_position(self.model, data, 2)?,
                            end: touch_position(self.model, data, 6)?,
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

fn touch_position(model: Model, data: &[u8], offset: usize) -> Result<(u16, u16)> {
    let x = u16::from_le_bytes([data[offset], data[offset + 1]]);
    let y = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
    ensure!(
        u32::from(x) < model.strip_width() && u32::from(y) < model.strip_height(),
        "touch coordinates outside display window"
    );
    Ok((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pedal_shared_metadata_and_three_inputs_but_no_display_or_controls() {
        // Native7.6 Pedal vtable uses the same serial/fw/input functions as2019.
        let mut state = InputState::new(Model::Pedal);
        assert_eq!(
            state.decode(&input(0, &[1, 0, 1])).unwrap(),
            vec![
                InputEvent::Key {
                    index: 0,
                    pressed: true
                },
                InputEvent::Key {
                    index: 2,
                    pressed: true
                }
            ]
        );
        assert_eq!(
            state.decode(&input(0, &[0, 1, 0])).unwrap(),
            vec![
                InputEvent::Key {
                    index: 0,
                    pressed: false
                },
                InputEvent::Key {
                    index: 1,
                    pressed: true
                },
                InputEvent::Key {
                    index: 2,
                    pressed: false
                }
            ]
        );
        assert!(state.decode(&input(0, &[0; 8])).is_err());
        assert!(state.decode(&input(3, &[1, 0, 0, 0, 0])).is_err());
        assert_eq!(
            decode_serial(Model::Pedal, b"\x06\x0ePEDAL123456789\0").unwrap(),
            "PEDAL123456789"
        );
        let mut fw = [0; 32];
        fw[0] = 5;
        fw[1] = 12;
        fw[6..14].copy_from_slice(b"1.2.3.4\0");
        assert_eq!(decode_firmware(Model::Pedal, &fw).unwrap(), "1.2.3.4");
        assert!(model_brightness_report(Model::Pedal, 50).is_err());
        assert!(model_sleep_report(Model::Pedal, 0).is_err());
        for command in [7, 8, 0x0b] {
            assert!(model_image_reports(Model::Pedal, command, 0, &[1]).is_err());
        }
    }
    #[test]
    fn neo_infobar_centers_logical_canvas_without_stretching_into_gutter() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            232,
            50,
            image::Rgb([255, 255, 255]),
        ));
        let frame = neo_infobar_frame(&image).unwrap();
        let rgb = frame.to_rgb8();
        assert_eq!(rgb.dimensions(), (248, 58));
        for (x, y, p) in rgb.enumerate_pixels() {
            assert_eq!(
                p.0,
                if (8..240).contains(&x) && (4..54).contains(&y) {
                    [255; 3]
                } else {
                    [0; 3]
                }
            );
        }
        let jpeg = encode_model_jpeg(Model::Neo, &frame, 248, 58).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (248, 58));
        let reports = model_image_reports(Model::Neo, 0x0b, 0, &jpeg).unwrap();
        assert!(reports.iter().all(|r| r[0..3] == [2, 0x0b, 0]));
        let mut rebuilt = Vec::new();
        for report in reports {
            let len = u16::from_le_bytes([report[4], report[5]]) as usize;
            assert!(len <= 1016);
            rebuilt.extend_from_slice(&report[8..8 + len]);
        }
        assert_eq!(rebuilt, jpeg);
    }
    #[test]
    fn neo_sensor_images_use_extra_hardware_positions_and_short_jpeg_canvas() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(96, 16, |x, y| {
            image::Rgb([x as u8, y as u8, 200])
        }));
        let jpeg = encode_model_jpeg(Model::Neo, &image, 96, 16).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (96, 16));
        assert!(decoded.get_pixel(0, 0)[0] > 80); // 180-degree physical orientation.
        for sensor in 0..2 {
            let reports = model_image_reports(Model::Neo, 7, 8 + sensor, &jpeg).unwrap();
            let mut rebuilt = Vec::new();
            for (index, report) in reports.iter().enumerate() {
                assert_eq!(&report[..3], &[2, 7, 8 + sensor]);
                assert_eq!(u16::from_le_bytes([report[6], report[7]]) as usize, index);
                assert_eq!(report[3], u8::from(index + 1 == reports.len()));
                let length = u16::from_le_bytes([report[4], report[5]]) as usize;
                rebuilt.extend_from_slice(&report[8..8 + length]);
            }
            assert_eq!(rebuilt, jpeg);
        }
        assert!(model_image_reports(Model::Neo, 7, 10, &jpeg).is_err());
        assert!(model_image_reports(Model::Plus, 7, 8, &jpeg).is_err());
        assert!(model_image_reports(Model::Classic2017, 7, 0, &jpeg).is_err());
    }
    #[test]
    fn original_2017_inputs_use_logical_rows_and_preserve_edges() {
        let mut state = InputState::new(Model::Classic2017);
        let mut report = [0; 64];
        report[0] = 1;
        report[5] = 1;
        report[11] = 1;
        assert_eq!(
            state.decode(&report).unwrap(),
            [
                InputEvent::Key {
                    index: 0,
                    pressed: true
                },
                InputEvent::Key {
                    index: 14,
                    pressed: true
                }
            ]
        );
        assert!(state.decode(&report).unwrap().is_empty());
        report[2] = 3;
        assert!(state.decode(&report).is_err());
        report.fill(0);
        assert_eq!(
            state.decode(&report).unwrap(),
            [
                InputEvent::Key {
                    index: 0,
                    pressed: false
                },
                InputEvent::Key {
                    index: 14,
                    pressed: false
                }
            ]
        );
    }

    #[test]
    fn all_key_payloads_and_neo_sensors_are_separate_and_validated_atomically() {
        for model in [
            Model::Classic,
            Model::Xl,
            Model::Plus,
            Model::PlusXl,
            Model::Neo,
        ] {
            let mut state = InputState::new(model);
            let count = usize::from(model.key_count() + model.sensor_count());
            let mut data = vec![0; count];
            data[usize::from(model.key_count() - 1)] = 1;
            assert_eq!(
                state.decode(&input(0, &data)).unwrap(),
                vec![InputEvent::Key {
                    index: model.key_count() - 1,
                    pressed: true
                }]
            );
            assert!(state.decode(&input(0, &data)).unwrap().is_empty());
            let before = state.keys.clone();
            data[0] = 2;
            assert!(state.decode(&input(0, &data)).is_err());
            assert_eq!(state.keys, before);
            if model == Model::Neo {
                data[0] = 0;
                data[8] = 1;
                data[9] = 1;
                assert_eq!(
                    state.decode(&input(0, &data)).unwrap(),
                    vec![
                        InputEvent::Sensor {
                            index: 0,
                            pressed: true
                        },
                        InputEvent::Sensor {
                            index: 1,
                            pressed: true
                        }
                    ]
                );
                assert!(state.decode(&input(3, &[1, 1, 0, 0, 0])).is_err());
            }
        }
    }
    #[test]
    fn mini_input_is_fixed_65_bytes_with_six_states_at_offset_one() {
        let mut state = InputState::new(Model::Mini);
        let mut report = [0; 65];
        report[0] = 1;
        report[6] = 1;
        report[64] = 99;
        assert_eq!(
            state.decode(&report).unwrap(),
            vec![InputEvent::Key {
                index: 5,
                pressed: true
            }]
        );
        assert!(state.decode(&report[..7]).is_err());
        report[1] = 2;
        assert!(state.decode(&report).is_err());
    }
    #[test]
    fn plus_xl_six_encoders_and_documented_touch_lengths() {
        let mut state = InputState::new(Model::PlusXl);
        assert_eq!(
            state.decode(&input(3, &[1, 0, 0, 0, 0, 0, 255])).unwrap(),
            vec![InputEvent::DialRotate {
                index: 5,
                ticks: -1
            }]
        );
        let mut tap = vec![0; 16];
        tap[0] = 1;
        tap[2..4].copy_from_slice(&1199u16.to_le_bytes());
        tap[4] = 99;
        assert_eq!(
            state.decode(&input(2, &tap)).unwrap(),
            vec![InputEvent::Touch {
                x: 1199,
                y: 99,
                long: false
            }]
        );
        assert!(state.decode(&input(2, &tap[..10])).is_err());
        tap[2..4].copy_from_slice(&1200u16.to_le_bytes());
        assert!(state.decode(&input(2, &tap)).is_err());
    }
    #[test]
    fn mini_features_are_not_main_protocol_reports() {
        let report = model_brightness_report(Model::Mini, 73).unwrap();
        assert_eq!(&report[..6], &[5, 0x55, 0xaa, 0xd1, 1, 73]);
        assert!(report[6..].iter().all(|&b| b == 0));
        let report = model_sleep_report(Model::Mini, 600).unwrap();
        assert_eq!(&report[..6], &[0x0b, 0xa2, 0x58, 2, 0, 0]);
        assert_eq!(
            decode_model_sleep(Model::Mini, &[0xa3, 4, 0x58, 2, 0, 0]).unwrap(),
            600
        );
        assert!(decode_model_sleep(Model::Plus, &[0xa3, 4, 0, 0, 0, 0]).is_err());
        assert_eq!(
            decode_serial(Model::Mini, b"\x03\0\0\0\0SERIAL123\0").unwrap(),
            "SERIAL123"
        );
        let mut fw = [0; 32];
        fw[0] = 0xa1;
        fw[5..10].copy_from_slice(b"1.2.3");
        assert_eq!(decode_firmware(Model::Mini, &fw).unwrap(), "1.2.3");
        assert!(decode_firmware(Model::Plus, &fw).is_err());
    }
    #[test]
    fn model_images_have_correct_rotation_dimensions_and_chunk_geometry() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(80, 80, |x, y| {
            if x < 40 && y < 40 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 0])
            }
        }));
        let bmp = encode_mini_bmp(&image).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        let decoded = image::load_from_memory(&bmp).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (80, 80));
        assert_eq!(decoded.get_pixel(70, 10).0, [255, 0, 0]);
        assert_eq!(decoded.get_pixel(10, 10).0, [0, 0, 0]);
        let reports = mini_image_reports(5, &bmp).unwrap();
        for (index, report) in reports.iter().enumerate() {
            assert_eq!(
                &report[..6],
                &[
                    2,
                    1,
                    index as u8,
                    0,
                    u8::from(index + 1 == reports.len()),
                    5
                ]
            );
            assert!(report[6..16].iter().all(|&v| v == 0));
        }
        let rebuilt: Vec<u8> = reports
            .iter()
            .flat_map(|r| r[16..].iter().copied())
            .take(bmp.len())
            .collect();
        assert_eq!(rebuilt, bmp);
        assert!(mini_image_reports(6, &bmp).is_err());
        let classic = encode_model_jpeg(Model::Classic, &image, 72, 72).unwrap();
        let classic = image::load_from_memory(&classic).unwrap().to_rgb8();
        assert!(classic.get_pixel(60, 60).0[0] > 240);
        assert!(classic.get_pixel(10, 10).0[0] < 10);
        let xl = encode_model_jpeg(Model::PlusXl, &image, 1200, 100).unwrap();
        let xl = image::load_from_memory(&xl).unwrap().to_rgb8();
        assert_eq!(xl.dimensions(), (100, 1200));
        assert!(xl.get_pixel(10, 1100).0[0] > 240);
        assert!(model_image_reports(Model::PlusXl, 7, 35, &[1]).is_ok());
        assert!(model_image_reports(Model::PlusXl, 7, 36, &[1]).is_err());
        assert!(model_image_reports(Model::Classic, 0x0b, 0, &[1]).is_err());
        assert!(model_image_reports(Model::Neo, 0x0b, 0, &[1]).is_ok());
    }
    #[test]
    fn sleeping_reports_are_activity_without_decoding_or_mutating_button_state() {
        let mut state = InputState::default();
        let down = input(0, &[1, 0, 0, 0, 0, 0, 0, 0]);
        let up = input(0, &[0; 8]);
        assert_eq!(state.packet(&down, true).unwrap().unwrap().len(), 1);
        assert_eq!(state.packet(&up, false).unwrap(), Some(Vec::new()));
        assert_eq!(state.packet(&[0xff], false).unwrap(), Some(Vec::new()));
        assert_eq!(state.packet(&[], false).unwrap(), None);
        assert_eq!(state.packet(&down, true).unwrap(), Some(Vec::new()));
        assert_eq!(
            state.packet(&up, true).unwrap(),
            Some(vec![InputEvent::Key {
                index: 0,
                pressed: false
            }])
        );
        assert!(state.packet(&[0xff], true).is_err());
    }
    #[test]
    fn full_lcd_reports_use_reserved_zero_and_preserve_chunks() {
        let bytes = vec![27; IMAGE_CHUNK_SIZE + 3];
        let reports = image_reports(0x08, 0, &bytes).unwrap();
        assert_eq!(&reports[0][..4], &[2, 8, 0, 0]);
        assert_eq!(&reports[1][..8], &[2, 8, 0, 1, 3, 0, 1, 0]);
        assert_eq!(&reports[1][8..11], &[27; 3]);
        assert!(reports[1][11..].iter().all(|&byte| byte == 0));
        assert!(image_reports(0x08, 1, &bytes).is_err());
        let jpeg = encode_jpeg(&DynamicImage::new_rgb8(2, 2), 800, 480).unwrap();
        let image = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((image.width(), image.height()), (800, 480));
    }
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
