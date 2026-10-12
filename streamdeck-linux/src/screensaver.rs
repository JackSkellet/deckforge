//! Stream Deck+ full-LCD screensaver frames, without device or host access.
//!
//! Geometry: https://docs.elgato.com/img/streamdeck/hid/sd-plus-pixel-layout.svg
//! PNG/JPEG/GIF input is capped at 32 MiB and 4096 pixels per dimension. The
//! decoder allocation budget and retained RGB-frame budget are each 128 MiB;
//! these are not a total-process memory limit. Oversized animations fail whole.
//! Images retain their aspect ratio against black borders. GIF disposal and
//! nonzero delays come from image's decoder. Screensavers loop continuously;
//! zero-delay frames use 10 ms to avoid a busy timer, a Linux playback policy.
use anyhow::{Context, Result, ensure};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::{
    fs::OpenOptions,
    io::{Cursor, Read},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::Duration,
};

pub const WIDTH: u32 = 800;
pub const HEIGHT: u32 = 480;
const MAX_INPUT: u64 = 32 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4096;
const MAX_FRAME_BYTES: usize = 128 * 1024 * 1024;
const FRAME_BYTES: usize = WIDTH as usize * HEIGHT as usize * 3;

pub struct Animation {
    frames: Vec<DynamicImage>,
    ends: Vec<Duration>,
    period: Duration,
}

impl Animation {
    pub fn load(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .with_context(|| format!("Open screensaver {}", path.display()))?;
        ensure!(
            file.metadata()?.is_file(),
            "Screensaver must be a regular file"
        );
        ensure!(
            file.metadata()?.len() <= MAX_INPUT,
            "Screensaver exceeds 32 MiB input limit"
        );
        let mut bytes = Vec::new();
        file.take(MAX_INPUT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_INPUT,
            "Screensaver exceeds 32 MiB input limit"
        );
        Self::decode(&bytes, MAX_FRAME_BYTES)
            .with_context(|| format!("Decode screensaver {}", path.display()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() as u64 <= MAX_INPUT,
            "Screensaver exceeds32MiB input limit"
        );
        Self::decode(bytes, MAX_FRAME_BYTES)
    }
    pub fn default_logo() -> Result<Self> {
        let tree = resvg::usvg::Tree::from_data(
            include_bytes!("../assets/deckforge.svg"),
            &resvg::usvg::Options::default(),
        )?;
        let size = tree.size();
        let mut canvas =
            resvg::tiny_skia::Pixmap::new(256, 256).context("Allocate Deckforge logo")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(256.0 / size.width(), 256.0 / size.height()),
            &mut canvas.as_mut(),
        );
        // tiny-skia stores premultiplied RGBA. Demultiply before image's alpha
        // compositing so antialiased edges are not darkened twice.
        for pixel in canvas.data_mut().as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
        let image = image::RgbaImage::from_raw(256, 256, canvas.take())
            .context("Decode Deckforge logo pixels")?;
        Ok(Self::still(DynamicImage::ImageRgba8(image)))
    }

    fn still(image: DynamicImage) -> Self {
        Self {
            frames: vec![contain(image)],
            ends: Vec::new(),
            period: Duration::ZERO,
        }
    }

    fn decode(bytes: &[u8], budget: usize) -> Result<Self> {
        let format = image::guess_format(bytes)?;
        ensure!(
            matches!(
                format,
                ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif
            ),
            "Screensavers support PNG, JPEG and GIF"
        );
        ensure!(
            budget >= FRAME_BYTES,
            "Screensaver frame exceeds retained-frame memory budget"
        );
        if format != ImageFormat::Gif {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(limits());
            return Ok(Self::still(reader.decode()?));
        }
        let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))?;
        decoder.set_limits(limits())?;
        let mut frames = Vec::new();
        let mut ends = Vec::new();
        let mut period = Duration::ZERO;
        for decoded in decoder.into_frames() {
            let decoded = decoded?;
            ensure!(
                frames.len() < budget / FRAME_BYTES,
                "Screensaver animation exceeds retained-frame memory budget; no frames were truncated"
            );
            let (numerator, denominator) = decoded.delay().numer_denom_ms();
            let delay = if numerator == 0 {
                Duration::from_millis(10)
            } else {
                Duration::from_nanos(u64::from(numerator) * 1_000_000 / u64::from(denominator))
            };
            period = period
                .checked_add(delay)
                .context("Screensaver duration overflow")?;
            frames.push(contain(DynamicImage::ImageRgba8(decoded.into_buffer())));
            ends.push(period);
        }
        ensure!(!frames.is_empty(), "Screensaver GIF contains no frames");
        Ok(Self {
            frames,
            ends,
            period,
        })
    }

    pub fn frame(&self, elapsed: Duration) -> &DynamicImage {
        &self.frames[self.position(elapsed).0]
    }

    pub fn next_frame_in(&self, elapsed: Duration) -> Option<Duration> {
        if self.frames.len() <= 1 {
            return None;
        }
        let (index, phase) = self.position(elapsed);
        Some(Duration::from_nanos(
            (self.ends[index].as_nanos() - phase) as u64,
        ))
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    fn position(&self, elapsed: Duration) -> (usize, u128) {
        if self.frames.len() <= 1 {
            return (0, 0);
        }
        let phase = elapsed.as_nanos() % self.period.as_nanos();
        (
            self.ends.partition_point(|end| end.as_nanos() <= phase),
            phase,
        )
    }
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_FRAME_BYTES as u64);
    limits
}

fn contain(image: DynamicImage) -> DynamicImage {
    let resized = image.resize(WIDTH, HEIGHT, image::imageops::FilterType::Lanczos3);
    let mut canvas = DynamicImage::new_rgb8(WIDTH, HEIGHT);
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((WIDTH - resized.width()) / 2),
        i64::from((HEIGHT - resized.height()) / 2),
    );
    canvas
}

pub fn key_crop(frame: &DynamicImage, index: u8) -> Result<DynamicImage> {
    ensure!(index < 8, "Screensaver key index must be below 8");
    ensure!(
        frame.width() == WIDTH && frame.height() == HEIGHT,
        "Screensaver frame must be 800×480"
    );
    Ok(frame.crop_imm(
        13 + 219 * u32::from(index % 4),
        12 + 160 * u32::from(index / 4),
        120,
        120,
    ))
}

/// Accepts a full-LCD frame returned by Animation::frame.
pub fn strip_crop(frame: &DynamicImage) -> DynamicImage {
    frame.crop_imm(0, 380, WIDTH, 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Delay, Frame, GenericImageView, Rgba, RgbaImage};
    use std::fs::File;

    fn gif(first_delay: u32, transparent_second: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            let first = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
            let mut second = RgbaImage::from_pixel(
                2,
                2,
                if transparent_second {
                    Rgba([0, 0, 0, 0])
                } else {
                    Rgba([0, 255, 0, 255])
                },
            );
            second.put_pixel(0, 0, Rgba([0, 255, 0, 255]));
            encoder
                .encode_frames([
                    Frame::from_parts(first, 0, 0, Delay::from_numer_denom_ms(first_delay, 1)),
                    Frame::from_parts(second, 0, 0, Delay::from_numer_denom_ms(200, 1)),
                ])
                .unwrap();
        }
        bytes
    }

    fn rgb(animation: &Animation, millis: u64, x: u32, y: u32) -> [u8; 4] {
        animation
            .frame(Duration::from_millis(millis))
            .get_pixel(x, y)
            .0
    }

    #[test]
    fn gif_colors_delays_loop_boundaries_and_zero_delay_policy() {
        let animation = Animation::decode(&gif(100, false), MAX_FRAME_BYTES).unwrap();
        assert_eq!(animation.len(), 2);
        for time in [0, 99, 300, 399] {
            assert_eq!(rgb(&animation, time, 400, 240), [255, 0, 0, 255]);
        }
        for time in [100, 299, 400] {
            assert_eq!(rgb(&animation, time, 400, 240), [0, 255, 0, 255]);
        }
        assert_eq!(
            animation.next_frame_in(Duration::from_millis(99)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            animation.next_frame_in(Duration::from_millis(100)),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            animation.next_frame_in(Duration::from_millis(301)),
            Some(Duration::from_millis(99))
        );
        let zero = Animation::decode(&gif(0, false), MAX_FRAME_BYTES).unwrap();
        assert_eq!(
            zero.next_frame_in(Duration::ZERO),
            Some(Duration::from_millis(10))
        );
        assert_eq!(rgb(&zero, 10, 400, 240), [0, 255, 0, 255]);
    }

    #[test]
    fn gif_disposal_is_composited_before_resizing() {
        let bytes = gif(100, true);
        let background = Animation::decode(&bytes, MAX_FRAME_BYTES).unwrap();
        assert_eq!(rgb(&background, 100, 500, 350), [0, 0, 0, 255]);
        let mut keep = bytes;
        // Change the first Graphic Control Extension from background disposal
        // to keep. The encoded image data and second frame are unchanged.
        let gce = keep
            .windows(3)
            .position(|bytes| bytes == [0x21, 0xf9, 0x04])
            .unwrap();
        keep[gce + 3] = (keep[gce + 3] & !0x1c) | 0x04;
        let kept = Animation::decode(&keep, MAX_FRAME_BYTES).unwrap();
        let retained = rgb(&kept, 100, 500, 350);
        // Lanczos may mix a small amount of the neighboring green pixel.
        assert!(retained[0] > 250 && retained[1] < 5 && retained[2] == 0);
        assert_eq!(rgb(&kept, 100, 220, 60), [0, 255, 0, 255]);
    }

    #[test]
    fn static_formats_keep_aspect_and_flatten_transparency_on_black() {
        let directory = tempfile::tempdir().unwrap();
        for format in [ImageFormat::Png, ImageFormat::Jpeg] {
            let path = directory.path().join(format!("still.{format:?}"));
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(10, 10, image::Rgb([255, 0, 0])))
                .save_with_format(&path, format)
                .unwrap();
            let animation = Animation::load(&path).unwrap();
            assert_eq!(animation.len(), 1);
            assert_eq!(animation.next_frame_in(Duration::ZERO), None);
            assert_eq!(rgb(&animation, 0, 0, 240), [0, 0, 0, 255]);
            assert!(rgb(&animation, 0, 400, 240)[0] > 250);
        }
        let alpha = Animation::still(DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            1,
            1,
            Rgba([200, 100, 50, 128]),
        )));
        assert_eq!(rgb(&alpha, 0, 400, 240), [100, 50, 25, 255]);
    }

    #[test]
    fn geometry_crops_the_documented_key_and_touch_coordinates() {
        let frame = DynamicImage::ImageRgb8(image::RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, (x / 256) as u8])
        }));
        for index in 0..8 {
            let key = key_crop(&frame, index).unwrap();
            assert_eq!((key.width(), key.height()), (120, 120));
            let x = 13 + 219 * u32::from(index % 4);
            let y = 12 + 160 * u32::from(index / 4);
            assert_eq!(key.get_pixel(0, 0), frame.get_pixel(x, y));
            assert_eq!(key.get_pixel(119, 119), frame.get_pixel(x + 119, y + 119));
        }
        assert!(key_crop(&frame, 8).is_err());
        assert!(key_crop(&DynamicImage::new_rgb8(120, 120), 0).is_err());
        let strip = strip_crop(&frame);
        assert_eq!((strip.width(), strip.height()), (800, 100));
        assert_eq!(strip.get_pixel(0, 0), frame.get_pixel(0, 380));
        assert_eq!(strip.get_pixel(799, 99), frame.get_pixel(799, 479));
    }

    #[test]
    fn rejects_size_limits_and_whole_animation_instead_of_truncating() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.bin");
        File::create(&path).unwrap().set_len(MAX_INPUT + 1).unwrap();
        assert!(Animation::load(&path).is_err());
        let path = directory.path().join("wide.png");
        DynamicImage::new_rgb8(MAX_DIMENSION + 1, 1)
            .save(&path)
            .unwrap();
        assert!(Animation::load(&path).is_err());
        assert!(Animation::decode(&gif(100, false), FRAME_BYTES).is_err());
        assert!(Animation::decode(b"not an image", MAX_FRAME_BYTES).is_err());
    }

    #[test]
    fn embedded_default_is_a_static_full_lcd_deckforge_logo() {
        let animation = Animation::default_logo().unwrap();
        assert_eq!(animation.len(), 1);
        assert_eq!(
            animation.frame(Duration::ZERO).dimensions(),
            (WIDTH, HEIGHT)
        );
        assert_eq!(animation.next_frame_in(Duration::from_secs(100)), None);
        assert_eq!(rgb(&animation, 0, 0, 0), [0, 0, 0, 255]);
        assert!(
            animation
                .frame(Duration::ZERO)
                .to_rgb8()
                .pixels()
                .any(|pixel| pixel[1] > 150)
        );
    }
}
