//! Original 2017 protocol (USB 0fd9:0060), independently recovered from the
//! Windows 7.6 artifact. See docs/LEGACY_HID.md.
//! Pure codecs only: no USB access, resets, firmware writes, or delays here.
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, Rgba, RgbaImage, imageops::FilterType};
use std::time::Duration;

pub const REPORT_SIZE: usize = 8191;
pub const INTER_REPORT_DELAY: Duration = Duration::from_millis(1);
const KEY_SIDE: u32 = 72;
const BMP_SIZE: usize = 54 + 72 * 72 * 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transform {
    Rotate180,
    FlipHorizontal,
}
#[derive(Clone, Debug)]
pub struct Firmware {
    version: String,
    build: u32,
    transform: Transform,
}
impl Firmware {
    /// Native initialization has explicit image-transform branches for 1.0.x.
    /// Other version families fail closed rather than using an assumed default.
    pub fn parse(version: &str) -> Result<Self> {
        ensure!(
            !version.is_empty() && version.len() <= 12,
            "invalid 2017 firmware version length"
        );
        let parts: Vec<&str> = version.split('.').collect();
        ensure!(
            (3..=4).contains(&parts.len()),
            "2017 firmware must have three or four numeric components"
        );
        let numbers: Vec<u32> = parts
            .iter()
            .map(|part| {
                ensure!(
                    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()),
                    "invalid 2017 firmware component"
                );
                part.parse()
                    .context("2017 firmware component exceeds integer range")
            })
            .collect::<Result<_>>()?;
        ensure!(
            numbers[0] == 1 && numbers[1] == 0,
            "unsupported 2017 firmware family {version}; expected proven 1.0.x behavior"
        );
        let build = numbers[2];
        // 0x140618f80 checks build 170134, the 170143 exception, then 191203.
        let transform = if (170134..191203).contains(&build) && build != 170143 {
            Transform::FlipHorizontal
        } else {
            Transform::Rotate180
        };
        Ok(Self {
            version: version.to_owned(),
            build,
            transform,
        })
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Map the host's logical percentage to the native firmware-specific range.
    /// Raw brightness packet construction below deliberately does not apply this twice.
    pub fn brightness_value(&self, logical: u8) -> Result<u8> {
        ensure!(logical <= 100, "brightness must be between 0 and 100");
        let (min, max): (u16, u16) = if self.build >= 170133 {
            (3, 100)
        } else {
            (10, 70)
        };
        Ok((min + u16::from(logical) * (max - min) / 100) as u8)
    }
    /// Two fixed 8191-byte reports, with 7803 bytes of BMP data in each. Caller
    /// writes the first report, waits INTER_REPORT_DELAY, then writes the second.
    pub fn key_reports(&self, key: u8, image: &DynamicImage) -> Result<[Vec<u8>; 2]> {
        let device_key = image_key(key)?;
        let bmp = self.key_bmp(image)?;
        let half = BMP_SIZE / 2;
        let mut reports = [vec![0; REPORT_SIZE], vec![0; REPORT_SIZE]];
        for (index, report) in reports.iter_mut().enumerate() {
            report[..6].copy_from_slice(&[2, 1, (index + 1) as u8, 0, index as u8, device_key]);
            report[16..16 + half].copy_from_slice(&bmp[index * half..(index + 1) * half]);
        }
        Ok(reports)
    }
    fn key_bmp(&self, image: &DynamicImage) -> Result<Vec<u8>> {
        ensure!(image.width() > 0 && image.height() > 0, "image is empty");
        let image = image
            .resize_exact(KEY_SIDE, KEY_SIDE, FilterType::Lanczos3)
            .to_rgba8();
        let mut opaque = RgbaImage::from_pixel(KEY_SIDE, KEY_SIDE, Rgba([0, 0, 0, 255]));
        image::imageops::overlay(&mut opaque, &image, 0, 0);
        let image = DynamicImage::ImageRgba8(opaque);
        let image = match self.transform {
            Transform::Rotate180 => image.rotate180(),
            Transform::FlipHorizontal => image.fliph(),
        }
        .to_rgb8();
        // The native serializer 0x14079ac9e writes a 54-byte BMP header, no row
        // padding for 72 pixels, BGR888 pixels, bottom-up rows, zero biSizeImage.
        let mut bmp = vec![0; BMP_SIZE];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&(BMP_SIZE as u32).to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&KEY_SIDE.to_le_bytes());
        bmp[22..26].copy_from_slice(&KEY_SIDE.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        let mut at = 54;
        for y in (0..KEY_SIDE).rev() {
            for x in 0..KEY_SIDE {
                let [r, g, b] = image.get_pixel(x, y).0;
                bmp[at..at + 3].copy_from_slice(&[b, g, r]);
                at += 3;
            }
        }
        Ok(bmp)
    }
}
/// Logical left-to-right key index -> one-based, row-reversed device image key.
fn image_key(key: u8) -> Result<u8> {
    ensure!(key < 15, "2017 key index must be between 0 and 14");
    Ok(key / 5 * 5 + 4 - key % 5 + 1)
}
pub fn decode_serial(report: &[u8]) -> Result<String> {
    metadata(report, 3)
}
pub fn firmware_string(report: &[u8]) -> Result<String> {
    metadata(report, 4)
}
pub fn decode_firmware(report: &[u8]) -> Result<Firmware> {
    Firmware::parse(&firmware_string(report)?)
}
fn metadata(report: &[u8], id: u8) -> Result<String> {
    ensure!(
        report.len() == 32 && report[0] == id,
        "invalid 2017 metadata response"
    );
    let bytes = &report[5..17];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let bytes = &bytes[..end];
    ensure!(
        !bytes.is_empty() && bytes.iter().all(u8::is_ascii_graphic),
        "empty or non-ASCII 2017 metadata"
    );
    Ok(String::from_utf8(bytes.to_vec())?)
}
/// Native reader 0x1406185e0 ignores the first byte and copies 15 states starting at 1.
/// Linux requires enough bytes and valid boolean values before changing state.
pub fn decode_keys(report: &[u8]) -> Result<[u8; 15]> {
    ensure!(
        (16..=64).contains(&report.len()),
        "invalid 2017 input report size"
    );
    let data = &report[1..16];
    ensure!(data.iter().all(|&b| b <= 1), "invalid 2017 key state");
    Ok(std::array::from_fn(|index| {
        data[index / 5 * 5 + 4 - index % 5]
    }))
}
/// Raw 0..100 report, without a native logical-percentage conversion.
pub fn brightness_report(raw: u8) -> Result<[u8; 32]> {
    ensure!(raw <= 100, "brightness must be between 0 and 100");
    let mut report = [0; 32];
    report[..6].copy_from_slice(&[5, 0x55, 0xaa, 0xd1, 1, raw]);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn colors() -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(72, 72, |x, y| {
            match (x < 36, y < 36) {
                (true, true) => image::Rgb([255, 0, 0]),
                (false, true) => image::Rgb([0, 255, 0]),
                (true, false) => image::Rgb([0, 0, 255]),
                (false, false) => image::Rgb([255, 255, 0]),
            }
        }))
    }
    fn bmp_from_reports(reports: &[Vec<u8>; 2]) -> Vec<u8> {
        reports
            .iter()
            .flat_map(|r| r[16..16 + BMP_SIZE / 2].iter().copied())
            .collect()
    }
    #[test]
    fn synthetic_qt_reference_bmps_match_byte_for_byte() {
        use sha2::{Digest, Sha256};
        // Generated by our independent Qt6 QImage/QTransform fixture program,
        // evidence/device-protocols/legacy2017-qt-reference.cpp; no vendor code.
        for (version, expected) in [
            (
                "1.0.170134",
                "52d8b2fb72247894d29252880203e59cc03b8ece7214d97c1a24004eb3d8cb6c",
            ),
            (
                "1.0.191203",
                "84aeb2ce6d8cb35705491f0a113ef3dadb7444cd4aa753bc862a8aa7c715eb95",
            ),
        ] {
            let reports = Firmware::parse(version)
                .unwrap()
                .key_reports(0, &colors())
                .unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(bmp_from_reports(&reports))),
                expected
            );
        }
    }
    #[test]
    fn firmware_branch_boundaries_and_exception_change_real_pixels() {
        for (version, top_left, bottom_right) in [
            ("1.0.170132", [255, 255, 0], [255, 0, 0]),
            ("1.0.170133", [255, 255, 0], [255, 0, 0]),
            ("1.0.170134", [0, 255, 0], [0, 0, 255]),
            ("1.0.170142", [0, 255, 0], [0, 0, 255]),
            ("1.0.170143", [255, 255, 0], [255, 0, 0]),
            ("1.0.170144", [0, 255, 0], [0, 0, 255]),
            ("1.0.191202", [0, 255, 0], [0, 0, 255]),
            ("1.0.191203", [255, 255, 0], [255, 0, 0]),
        ] {
            let fw = Firmware::parse(version).unwrap();
            let reports = fw.key_reports(0, &colors()).unwrap();
            let bmp = bmp_from_reports(&reports);
            let decoded = image::load_from_memory(&bmp).unwrap().to_rgb8();
            assert_eq!(decoded.dimensions(), (72, 72));
            assert_eq!(decoded.get_pixel(5, 5).0, top_left, "{version}");
            assert_eq!(decoded.get_pixel(65, 65).0, bottom_right, "{version}");
        }
    }
    #[test]
    fn metadata_offsets_lengths_and_unknown_firmware_fail_closed() {
        let mut report = [0; 32];
        report[0] = 4;
        report[5..15].copy_from_slice(b"1.0.191203");
        assert_eq!(decode_firmware(&report).unwrap().version(), "1.0.191203");
        report[0] = 3;
        report[5..17].copy_from_slice(b"AL01A1234567");
        assert_eq!(decode_serial(&report).unwrap(), "AL01A1234567");
        assert!(decode_serial(&report[..31]).is_err());
        assert!(decode_firmware(&report).is_err());
        for v in [
            "2.0.191203",
            "1.1.170134",
            "1.0",
            "1.0.-1",
            "1.0.19120x",
            "1.0.170134.123",
        ] {
            assert!(Firmware::parse(v).is_err(), "{v}");
        }
        assert!(Firmware::parse("1.0.191203.1").is_ok());
    }
    #[test]
    fn logical_keys_map_to_native_rows_and_input_roundtrips() {
        for key in 0..15 {
            let wire = image_key(key).unwrap();
            assert!((1..=15).contains(&wire));
            let mut report = [0; 64];
            report[usize::from(wire)] = 1;
            let keys = decode_keys(&report).unwrap();
            assert_eq!(keys[usize::from(key)], 1);
            assert_eq!(keys.iter().sum::<u8>(), 1);
        }
        assert_eq!(
            (
                image_key(0).unwrap(),
                image_key(4).unwrap(),
                image_key(10).unwrap(),
                image_key(14).unwrap()
            ),
            (5, 1, 15, 11)
        );
        assert!(image_key(15).is_err());
        assert!(decode_keys(&[0; 15]).is_err());
        let mut invalid = [0; 64];
        invalid[2] = 2;
        assert!(decode_keys(&invalid).is_err());
    }
    #[test]
    fn bmp_header_chunks_padding_and_brightness_match_native_framing() {
        let fw = Firmware::parse("1.0.191203").unwrap();
        let reports = fw.key_reports(14, &colors()).unwrap();
        assert_eq!(
            &reports[0][..16],
            &[2, 1, 1, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            &reports[1][..16],
            &[2, 1, 2, 0, 1, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        for report in &reports {
            assert_eq!(report.len(), 8191);
            assert!(report[7819..].iter().all(|&b| b == 0));
        }
        let bmp = bmp_from_reports(&reports);
        assert_eq!(bmp.len(), 15606);
        assert_eq!(&bmp[..6], &[0x42, 0x4d, 0xf6, 0x3c, 0, 0]);
        assert!(bmp[30..54].iter().all(|&b| b == 0));
        // Rotate180 gives green as the first bottom-up pixel; wire format is BGR.
        assert_eq!(&bmp[54..57], &[0, 255, 0]);
        assert_eq!(fw.brightness_value(0).unwrap(), 3);
        assert_eq!(fw.brightness_value(50).unwrap(), 51);
        assert_eq!(fw.brightness_value(100).unwrap(), 100);
        let old = Firmware::parse("1.0.170132").unwrap();
        assert_eq!(old.brightness_value(0).unwrap(), 10);
        assert_eq!(old.brightness_value(100).unwrap(), 70);
        assert_eq!(
            &brightness_report(51).unwrap()[..6],
            &[5, 0x55, 0xaa, 0xd1, 1, 51]
        );
        assert!(brightness_report(101).is_err());
    }
}
