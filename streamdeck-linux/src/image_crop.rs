//! Still-frame crop output for the native per-action 200x100 background editor.
use anyhow::{Context, Result, ensure};
use image::{AnimationDecoder, DynamicImage, ImageDecoder};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
pub struct Source {
    pub image: DynamicImage,
    pub frames: usize,
    pub hash: String,
}
fn dimensions(width: u32, height: u32) -> Result<()> {
    ensure!(
        width > 0
            && height > 0
            && width <= 8192
            && height <= 8192
            && u64::from(width) * u64::from(height) <= 16 * 1024 * 1024,
        "Crop source exceeds image limits"
    );
    Ok(())
}
pub fn read(path: &Path, index: usize) -> Result<Source> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    ensure!(file.metadata()?.is_file(), "Choose a regular image file");
    let mut data = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut data)?;
    ensure!(data.len() <= 16 * 1024 * 1024, "Crop source exceeds 16 MiB");
    let hash = blake3::hash(&data).to_hex().to_string();
    let (image, frames) = if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        let mut options = gif::DecodeOptions::new();
        options.skip_frame_decoding(true);
        let mut metadata = options.read_info(Cursor::new(&data))?;
        let mut frames = 0;
        while metadata.read_next_frame()?.is_some() {
            frames += 1;
            ensure!(frames <= 1024, "Crop source exceeds 1024 frames");
        }
        ensure!(index < frames, "Frame outside image");
        let decoder = image::codecs::gif::GifDecoder::new(Cursor::new(&data))?;
        let (w, h) = decoder.dimensions();
        dimensions(w, h)?;
        ensure!(
            u64::from(w) * u64::from(h) * (index as u64 + 1) <= 256 * 1024 * 1024,
            "Selected frame exceeds decode work limit"
        );
        (
            DynamicImage::ImageRgba8(
                decoder
                    .into_frames()
                    .nth(index)
                    .context("Missing GIF frame")??
                    .into_buffer(),
            ),
            frames,
        )
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        let mut decoder = image_webp::WebPDecoder::new(Cursor::new(&data))?;
        let (w, h) = decoder.dimensions();
        dimensions(w, h)?;
        decoder.set_memory_limit(64 * 1024 * 1024);
        let frames = (decoder.num_frames() as usize).max(1);
        ensure!(
            frames <= 1024 && index < frames,
            "Frame outside supported image range"
        );
        ensure!(
            u64::from(w) * u64::from(h) * (index as u64 + 1) <= 256 * 1024 * 1024,
            "Selected frame exceeds decode work limit"
        );
        let mut bytes = vec![0; decoder.output_buffer_size().context("Image too large")?];
        if decoder.is_animated() {
            decoder.set_background_color([0, 0, 0, 0])?;
            for _ in 0..=index {
                decoder.read_frame(&mut bytes)?;
            }
        } else {
            decoder.read_image(&mut bytes)?;
        }
        let image = if decoder.has_alpha() {
            DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(w, h, bytes).context("Invalid RGBA frame")?,
            )
        } else {
            DynamicImage::ImageRgb8(
                image::RgbImage::from_raw(w, h, bytes).context("Invalid RGB frame")?,
            )
        };
        (image, frames)
    } else {
        ensure!(index == 0, "Still image has only one frame");
        let svg = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        let image = if svg {
            crate::host::decode_crop_svg(&data)?
        } else {
            let mut reader = image::ImageReader::new(Cursor::new(&data)).with_guessed_format()?;
            let (w, h) = image::ImageReader::new(Cursor::new(&data))
                .with_guessed_format()?
                .into_dimensions()?;
            dimensions(w, h)?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(8192);
            limits.max_image_height = Some(8192);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            reader.decode()?
        };
        dimensions(image.width(), image.height())?;
        (image, 1)
    };
    Ok(Source {
        image,
        frames,
        hash,
    })
}
pub fn render(source: &DynamicImage, rect: Rect) -> Result<DynamicImage> {
    ensure!(
        [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|v| v.is_finite()),
        "Crop values must be finite"
    );
    ensure!(
        rect.width >= 1.
            && rect.height >= 1.
            && rect.width <= 4000.
            && rect.height <= 4000.
            && rect.x.abs() <= 8000.
            && rect.y.abs() <= 8000.,
        "Crop outside supported range"
    );
    let scaled = source.resize_exact(
        rect.width.round() as u32,
        rect.height.round() as u32,
        image::imageops::FilterType::Triangle,
    );
    let mut image = DynamicImage::ImageRgba8(image::RgbaImage::new(200, 100));
    image::imageops::overlay(
        &mut image,
        &scaled,
        rect.x.round() as i64,
        rect.y.round() as i64,
    );
    Ok(image)
}
#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    #[test]
    fn crop_uses_position_scale_and_transparent_outside_pixels() {
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(400, 100, |x, _| {
            if x < 200 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        }));
        let crop = render(
            &source,
            Rect {
                x: -200.,
                y: 0.,
                width: 400.,
                height: 100.,
            },
        )
        .unwrap();
        assert_eq!(crop.get_pixel(100, 50).0, [0, 0, 255, 255]);
        let crop = render(
            &source,
            Rect {
                x: 100.,
                y: 50.,
                width: 100.,
                height: 25.,
            },
        )
        .unwrap();
        assert_eq!(crop.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(crop.get_pixel(110, 60).0, [255, 0, 0, 255]);
        assert!(
            render(
                &source,
                Rect {
                    x: f64::NAN,
                    y: 0.,
                    width: 200.,
                    height: 100.
                }
            )
            .is_err()
        );
        assert!(
            render(
                &source,
                Rect {
                    x: 0.,
                    y: 0.,
                    width: 5000.,
                    height: 100.
                }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use image::GenericImageView;
    #[test]
    fn animated_sources_select_a_still_frame() {
        let root = tempfile::tempdir().unwrap();
        let gif = root.path().join("source.gif");
        std::fs::write(&gif, crate::key_animation::tests::gif()).unwrap();
        let frame = read(&gif, 1).unwrap();
        assert_eq!(frame.frames, 2);
        assert_eq!(frame.image.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert!(read(&gif, 2).is_err());
        let webp = root.path().join("source.webp");
        std::fs::write(&webp, crate::background_movie::tests::fixture(1)).unwrap();
        let frame = read(&webp, 2).unwrap();
        assert_eq!(frame.frames, 3);
        assert_eq!(frame.image.get_pixel(3, 3).0, [0, 255, 0, 255]);
    }
}
