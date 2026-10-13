//! Bounded, immutable frames for animated key artwork. Playback clocks live in the host.
use anyhow::{Context, Result, ensure};
use image::{AnimationDecoder, DynamicImage, ImageDecoder};
use std::{io::Cursor, time::Duration};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_FRAMES: usize = 1024;
pub struct Clip {
    frames: Vec<DynamicImage>,
    ends: Vec<Duration>,
    period: Duration,
    pub bytes: usize,
    cycles: Option<u32>,
}
impl Clip {
    pub fn decode(data: &[u8]) -> Result<Option<Self>> {
        if !data.starts_with(b"GIF87a") && !data.starts_with(b"GIF89a") {
            return Ok(None);
        }
        ensure!(
            data.len() <= 8 * 1024 * 1024,
            "Animated icon exceeds 8 MiB input limit"
        );
        let cycles = match gif::Decoder::new(Cursor::new(data))?.repeat() {
            gif::Repeat::Infinite => None,
            gif::Repeat::Finite(n) => Some(u32::from(n).max(1)),
        };
        let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(data))?;
        let (width, height) = decoder.dimensions();
        ensure!(
            u64::from(width) * u64::from(height) <= 16 * 1024 * 1024,
            "Animated icon exceeds 16 megapixels"
        );
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(64 * 1024 * 1024);
        decoder.set_limits(limits)?;
        let mut frames = Vec::new();
        let mut ends = Vec::new();
        let mut bytes = 0usize;
        let mut period = Duration::ZERO;
        for frame in decoder.into_frames() {
            let frame = frame?;
            ensure!(
                u64::from(width) * u64::from(height) * (frames.len() as u64 + 1)
                    <= 64 * 1024 * 1024,
                "Animated icon exceeds 64 megapixels of decoded frame work"
            );
            ensure!(
                frames.len() < MAX_FRAMES,
                "Animated icon exceeds 1024 frames"
            );
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let delay =
                Duration::from_nanos(u64::from(numerator) * 1_000_000 / u64::from(denominator))
                    .max(Duration::from_millis(25));
            let image = DynamicImage::ImageRgba8(frame.into_buffer());
            let image = if image.width() > 256 || image.height() > 256 {
                image.thumbnail(256, 256)
            } else {
                image
            };
            bytes = bytes
                .checked_add(image.as_bytes().len())
                .context("Animated icon size overflow")?;
            ensure!(
                bytes <= MAX_BYTES,
                "Animated icon exceeds 32 MiB retained frames"
            );
            period = period
                .checked_add(delay)
                .context("Animated icon duration overflow")?;
            frames.push(image);
            ends.push(period);
        }
        ensure!(!frames.is_empty(), "GIF has no frames");
        if frames.len() == 1 {
            return Ok(None);
        }
        Ok(Some(Self {
            frames,
            ends,
            period,
            bytes,
            cycles,
        }))
    }
    pub fn frame(&self, elapsed: Duration) -> &DynamicImage {
        &self.frames[self.index(elapsed)]
    }
    fn index(&self, elapsed: Duration) -> usize {
        if self.finished(elapsed) {
            return self.frames.len() - 1;
        }
        let phase = elapsed.as_nanos() % self.period.as_nanos();
        self.ends.partition_point(|end| end.as_nanos() <= phase)
    }
    fn finished(&self, elapsed: Duration) -> bool {
        self.cycles
            .is_some_and(|cycles| elapsed.as_nanos() >= self.period.as_nanos() * u128::from(cycles))
    }
    pub fn next_in(&self, elapsed: Duration) -> Option<Duration> {
        if self.finished(elapsed) {
            return None;
        }
        let phase = elapsed.as_nanos() % self.period.as_nanos();
        Some(Duration::from_nanos(
            (self.ends[self.index(elapsed)].as_nanos() - phase) as u64,
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn gif() -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut data);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for (color, delay) in [([255, 0, 0, 255], 40), ([0, 0, 255, 255], 80)] {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(4, 4, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        data
    }
    #[test]
    fn frame_boundaries_loop_and_keep_nonuniform_delays() {
        let clip = Clip::decode(&gif()).unwrap().unwrap();
        assert_eq!(
            clip.frame(Duration::from_millis(39))
                .to_rgba8()
                .get_pixel(0, 0)[0],
            255
        );
        assert_eq!(
            clip.frame(Duration::from_millis(40))
                .to_rgba8()
                .get_pixel(0, 0)[2],
            255
        );
        assert_eq!(
            clip.next_in(Duration::from_millis(40)),
            Some(Duration::from_millis(80))
        );
        assert_eq!(
            clip.frame(Duration::from_millis(120))
                .to_rgba8()
                .get_pixel(0, 0)[0],
            255
        );
        assert_eq!(clip.bytes, 128);
    }
    #[test]
    fn finite_cycles_stop_on_last_frame_and_short_frames_use_native_minimum() {
        for repeat in [gif::Repeat::Finite(0), gif::Repeat::Finite(2)] {
            let mut data = Vec::new();
            {
                let mut encoder =
                    gif::Encoder::new(&mut data, 1, 1, &[255, 0, 0, 0, 0, 255]).unwrap();
                encoder.set_repeat(repeat).unwrap();
                for index in [0, 1] {
                    let frame = gif::Frame {
                        width: 1,
                        height: 1,
                        delay: 0,
                        buffer: std::borrow::Cow::Owned(vec![index]),
                        ..Default::default()
                    };
                    encoder.write_frame(&frame).unwrap();
                }
            }
            let clip = Clip::decode(&data).unwrap().unwrap();
            assert_eq!(
                clip.next_in(Duration::ZERO),
                Some(Duration::from_millis(25))
            );
            let cycles = if repeat == gif::Repeat::Finite(2) {
                2
            } else {
                1
            };
            let end = Duration::from_millis(50 * cycles);
            assert!(clip.next_in(end - Duration::from_nanos(1)).is_some());
            assert_eq!(clip.next_in(end), None);
            assert_eq!(
                clip.frame(end + Duration::from_secs(10))
                    .to_rgba8()
                    .get_pixel(0, 0)[2],
                255
            );
        }
    }
    #[test]
    fn disposal_and_partial_frames_keep_transparency() {
        let mut data = Vec::new();
        {
            let mut encoder =
                gif::Encoder::new(&mut data, 2, 1, &[255, 0, 0, 0, 0, 255, 0, 255, 0]).unwrap();
            for (left, width, buffer, dispose) in [
                (0, 2, vec![0, 0], gif::DisposalMethod::Keep),
                (0, 1, vec![1], gif::DisposalMethod::Background),
                (1, 1, vec![2], gif::DisposalMethod::Keep),
            ] {
                encoder
                    .write_frame(&gif::Frame {
                        left,
                        width,
                        height: 1,
                        delay: 4,
                        buffer: std::borrow::Cow::Owned(buffer),
                        dispose,
                        ..Default::default()
                    })
                    .unwrap();
            }
        }
        let clip = Clip::decode(&data).unwrap().unwrap();
        let middle = clip.frame(Duration::from_millis(40)).to_rgba8();
        assert_eq!(middle.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(middle.get_pixel(1, 0).0, [255, 0, 0, 255]);
        let last = clip.frame(Duration::from_millis(80)).to_rgba8();
        assert_eq!(last.get_pixel(0, 0)[3], 0);
        assert_eq!(last.get_pixel(1, 0).0, [0, 255, 0, 255]);
    }
    #[test]
    fn rejects_excessive_frames_without_returning_a_truncated_clip() {
        let mut data = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut data, 1, 1, &[255, 0, 0]).unwrap();
            for _ in 0..=MAX_FRAMES {
                encoder
                    .write_frame(&gif::Frame {
                        width: 1,
                        height: 1,
                        delay: 3,
                        buffer: std::borrow::Cow::Owned(vec![0]),
                        ..Default::default()
                    })
                    .unwrap();
            }
        }
        assert!(
            Clip::decode(&data)
                .err()
                .unwrap()
                .to_string()
                .contains("1024 frames")
        );
    }
    #[test]
    fn static_and_invalid_inputs_are_distinct() {
        assert!(Clip::decode(b"not a gif").unwrap().is_none());
        assert!(Clip::decode(b"GIF89a").is_err());
    }
}
