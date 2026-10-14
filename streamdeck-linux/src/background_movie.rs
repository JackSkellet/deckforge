//! Streaming WebP backgrounds: encoded bytes plus one decoder canvas, not all frames.
use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use image_webp::{LoopCount, WebPDecoder};
use std::{io::Cursor, sync::Arc, time::Duration};
type Decoder = WebPDecoder<Cursor<Arc<[u8]>>>;
fn open(data: Arc<[u8]>) -> Result<Decoder> {
    ensure!(
        data.len() <= 16 * 1024 * 1024,
        "Animated background exceeds 16 MiB"
    );
    let mut decoder = WebPDecoder::new(Cursor::new(data))?;
    let (w, h) = decoder.dimensions();
    ensure!(
        w > 0 && h > 0 && w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= 16 * 1024 * 1024,
        "Animated background exceeds image limits"
    );
    decoder.set_memory_limit(64 * 1024 * 1024);
    if decoder.is_animated() {
        decoder.set_background_color([0, 0, 0, 0])?;
    }
    Ok(decoder)
}
pub fn is_animated(data: &[u8]) -> Result<bool> {
    ensure!(
        data.len() <= 16 * 1024 * 1024,
        "Animated background exceeds 16 MiB"
    );
    Ok(open(Arc::from(data))?.is_animated())
}
pub struct Movie {
    data: Arc<[u8]>,
    decoder: Decoder,
    index: u32,
    cycles: Option<u32>,
    due: Option<Duration>,
}
impl Movie {
    pub fn new(data: Arc<[u8]>, now: Duration) -> Result<(Self, DynamicImage)> {
        let decoder = open(data.clone())?;
        ensure!(
            decoder.is_animated() && decoder.num_frames() > 0,
            "WebP has no animation frames"
        );
        let cycles = match decoder.loop_count() {
            LoopCount::Forever => None,
            LoopCount::Times(n) => Some(u32::from(n.get())),
        };
        let mut movie = Self {
            data,
            decoder,
            index: 0,
            cycles,
            due: Some(now),
        };
        let frame = movie.read(now)?;
        Ok((movie, frame))
    }
    fn read(&mut self, now: Duration) -> Result<DynamicImage> {
        let (w, h) = self.decoder.dimensions();
        let mut bytes = vec![
            0;
            self.decoder
                .output_buffer_size()
                .context("WebP output size overflow")?
        ];
        let delay = self.decoder.read_frame(&mut bytes)?;
        self.index += 1;
        self.due = Some(now.saturating_add(Duration::from_millis(u64::from(delay))));
        Ok(if self.decoder.has_alpha() {
            DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(w, h, bytes).context("WebP RGBA size")?,
            )
        } else {
            DynamicImage::ImageRgb8(
                image::RgbImage::from_raw(w, h, bytes).context("WebP RGB size")?,
            )
        })
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.due
    }
    pub fn advance(&mut self, now: Duration) -> Result<Option<DynamicImage>> {
        if self.due.is_none_or(|due| now < due) {
            return Ok(None);
        }
        if self.index == self.decoder.num_frames() {
            if self.cycles == Some(1) {
                self.due = None;
                return Ok(None);
            }
            if let Some(cycles) = &mut self.cycles {
                *cycles -= 1;
            }
            // A new decoder also clears untouched canvas regions at the loop boundary.
            self.decoder = open(self.data.clone())?;
            self.index = 0;
        }
        match self.read(now) {
            Ok(frame) => Ok(Some(frame)),
            Err(error) => {
                self.due = None;
                Err(error)
            }
        }
    }
    pub fn stop(&mut self) {
        self.due = None;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::GenericImageView;
    fn chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = name.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            out.push(0)
        }
        out
    }
    fn u24(value: u32) -> [u8; 3] {
        let b = value.to_le_bytes();
        [b[0], b[1], b[2]]
    }
    pub fn fixture(loops: u16) -> Vec<u8> {
        fixture_size(loops, 4, 4)
    }
    pub fn fixture_size(loops: u16, width: u32, height: u32) -> Vec<u8> {
        let mut header = vec![0x12, 0, 0, 0];
        header.extend(u24(width - 1));
        header.extend(u24(height - 1));
        let mut data = chunk(b"VP8X", &header);
        let mut anim = vec![0, 0, 0, 0];
        anim.extend(loops.to_le_bytes());
        data.extend(chunk(b"ANIM", &anim));
        for (x, y, w, h, color, delay, flags) in [
            (0, 0, width, height, [255, 0, 0, 255], 40, 2),
            (0, 0, width / 2, height / 2, [0, 0, 255, 128], 80, 1),
            (
                width / 2,
                height / 2,
                width / 2,
                height / 2,
                [0, 255, 0, 255],
                120,
                2,
            ),
        ] {
            let mut encoded = Vec::new();
            image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
                .encode(
                    image::RgbaImage::from_pixel(w, h, image::Rgba(color)).as_raw(),
                    w,
                    h,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
            let mut payload = Vec::new();
            for v in [x / 2, y / 2, w - 1, h - 1, delay] {
                payload.extend(u24(v));
            }
            payload.push(flags);
            payload.extend_from_slice(&encoded[12..]);
            data.extend(chunk(b"ANMF", &payload));
        }
        let mut out = b"RIFF".to_vec();
        out.extend(((data.len() + 4) as u32).to_le_bytes());
        out.extend(b"WEBP");
        out.extend(data);
        out
    }
    #[test]
    #[ignore = "requires owned Qt reader built by tests/background_movie_reference.py"]
    fn qt_webp_reference() {
        let root =
            std::path::PathBuf::from(std::env::var_os("DECKFORGE_WEBP_REFERENCE_DIR").unwrap());
        let reader = std::env::var_os("DECKFORGE_WEBP_REFERENCE_READER").unwrap();
        let input = root.join("input.webp");
        let data = fixture(2);
        std::fs::write(&input, &data).unwrap();
        assert!(
            std::process::Command::new(reader)
                .arg(&input)
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("qt.json")).unwrap()).unwrap();
        assert_eq!(metadata["loop_count"], 1);
        assert_eq!(metadata["delays"], serde_json::json!([40, 80, 120]));
        let (mut movie, first) = Movie::new(Arc::from(data), Duration::ZERO).unwrap();
        let mut frames = vec![first];
        frames.push(movie.advance(Duration::from_millis(40)).unwrap().unwrap());
        frames.push(movie.advance(Duration::from_millis(120)).unwrap().unwrap());
        let mut delta = 0;
        for (i, frame) in frames.iter().enumerate() {
            let reference = image::open(root.join(format!("frame-{i}.png")))
                .unwrap()
                .to_rgba8();
            let actual = frame.to_rgba8();
            assert_eq!(actual.dimensions(), reference.dimensions());
            for (a, b) in actual.as_raw().iter().zip(reference.as_raw()) {
                delta = delta.max(a.abs_diff(*b));
            }
        }
        assert!(delta <= 1, "Qt/Rust channel delta {delta}");
        std::fs::write(root.join("comparison.json"),serde_json::to_vec_pretty(&serde_json::json!({"qt":metadata,"frames":3,"max_channel_delta":delta,"hardware_access":false})).unwrap()).unwrap();
    }
    #[test]
    fn zero_delay_yields_one_frame_per_poll_and_decode_failure_stops() {
        let mut zero = fixture(1);
        let mut cursor = 12;
        let mut frames = Vec::new();
        while cursor < zero.len() {
            let size =
                u32::from_le_bytes(zero[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
            if &zero[cursor..cursor + 4] == b"ANMF" {
                frames.push(cursor);
                zero[cursor + 20..cursor + 23].fill(0);
            }
            cursor += 8 + size + (size % 2);
        }
        let (mut movie, _) = Movie::new(Arc::from(zero), Duration::ZERO).unwrap();
        assert_eq!(movie.next_due(), Some(Duration::ZERO));
        assert!(movie.advance(Duration::ZERO).unwrap().is_some());
        assert!(movie.advance(Duration::ZERO).unwrap().is_some());
        assert!(movie.advance(Duration::ZERO).unwrap().is_none());
        assert!(movie.next_due().is_none());
        let mut corrupt = fixture(1);
        corrupt[frames[1] + 8 + 16 + 8] = 0;
        let (mut movie, _) = Movie::new(Arc::from(corrupt), Duration::ZERO).unwrap();
        assert!(movie.advance(Duration::from_millis(40)).is_err());
        assert!(movie.next_due().is_none());
    }
    #[test]
    fn streaming_frames_honor_delays_disposal_and_finite_loops() {
        let data = fixture(2);
        let (mut movie, first) = Movie::new(Arc::from(data), Duration::ZERO).unwrap();
        assert_eq!(first.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(movie.next_due(), Some(Duration::from_millis(40)));
        assert!(movie.advance(Duration::from_millis(39)).unwrap().is_none());
        let second = movie.advance(Duration::from_millis(40)).unwrap().unwrap();
        let p = second.get_pixel(0, 0).0;
        assert!(p[0] >= 126 && p[0] <= 128 && p[2] >= 127 && p[2] <= 129);
        assert_eq!(movie.next_due(), Some(Duration::from_millis(120)));
        let third = movie.advance(Duration::from_millis(120)).unwrap().unwrap();
        assert_eq!(third.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(third.get_pixel(3, 3).0, [0, 255, 0, 255]);
        assert_eq!(
            movie
                .advance(Duration::from_millis(240))
                .unwrap()
                .unwrap()
                .get_pixel(0, 0)
                .0,
            [255, 0, 0, 255]
        );
        movie.advance(Duration::from_millis(280)).unwrap();
        movie.advance(Duration::from_millis(360)).unwrap();
        assert!(movie.advance(Duration::from_millis(480)).unwrap().is_none());
        assert!(movie.next_due().is_none());
        let (mut infinite, _) = Movie::new(Arc::from(fixture(0)), Duration::ZERO).unwrap();
        for _ in 0..10 {
            let due = infinite.next_due().unwrap();
            assert!(infinite.advance(due).unwrap().is_some());
        }
        assert!(infinite.next_due().is_some());
    }
}
