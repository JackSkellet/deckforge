//! Our own minimal renderer. Generic font/image libraries only.
use anyhow::{Context, Result};
use fontdue::{Font, FontSettings};
use image::{DynamicImage, Rgb, RgbImage};
use serde_json::Value;

pub struct Renderer {
    font: Font,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        let path = std::env::var("DECKFORGE_FONT")
            .ok()
            .or_else(|| {
                [
                    "/usr/share/fonts/TTF/DejaVuSans.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                ]
                .into_iter()
                .find(|p| std::path::Path::new(p).is_file())
                .map(str::to_owned)
            })
            .context("No font found; set DECKFORGE_FONT to a readable TTF file")?;
        let bytes = std::fs::read(&path)
            .with_context(|| format!("Reading font {path}; set DECKFORGE_FONT to a TTF file"))?;
        let font = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|e| anyhow::anyhow!("Font: {e}"))?;
        Ok(Self { font })
    }

    fn text(&self, img: &mut RgbImage, text: &str, y: i32, size: f32, color: [u8; 3]) {
        let lines: Vec<_> = text.lines().take(4).collect();
        for (line, text) in lines.iter().enumerate() {
            let text: String = text.chars().take(32).collect();
            let width: f32 = text
                .chars()
                .map(|c| self.font.metrics(c, size).advance_width)
                .sum();
            let mut x = (img.width() as f32 - width) / 2.;
            for ch in text.chars() {
                let (m, bitmap) = self.font.rasterize(ch, size);
                for row in 0..m.height {
                    for col in 0..m.width {
                        let px = x as i32 + m.xmin + col as i32;
                        let py = y + line as i32 * (size as i32 + 3) + row as i32
                            - m.ymin
                            - m.height as i32;
                        if px >= 0 && py >= 0 && px < img.width() as i32 && py < img.height() as i32
                        {
                            let a = bitmap[row * m.width + col] as f32 / 255.;
                            let old = img.get_pixel(px as u32, py as u32).0;
                            img.put_pixel(
                                px as u32,
                                py as u32,
                                Rgb(std::array::from_fn(|i| {
                                    (old[i] as f32 * (1. - a) + color[i] as f32 * a) as u8
                                })),
                            );
                        }
                    }
                }
                x += m.advance_width;
            }
        }
    }

    pub fn key(&self, title: &str, background: Option<&DynamicImage>) -> DynamicImage {
        let mut img = background
            .map(|b| {
                b.resize_exact(120, 120, image::imageops::FilterType::Triangle)
                    .to_rgb8()
            })
            .unwrap_or_else(|| RgbImage::from_pixel(120, 120, Rgb([22, 29, 39])));
        let size = if title.chars().count() > 10 { 15. } else { 23. };
        self.text(&mut img, title, 68, size, [236, 243, 250]);
        DynamicImage::ImageRgb8(img)
    }

    /// Implements the title/value/indicator subset of the official $B1 layout.
    pub fn dial(&self, title: &str, feedback: &Value) -> DynamicImage {
        let mut img = RgbImage::from_pixel(200, 100, Rgb([14, 21, 29]));
        let label = feedback
            .get("title")
            .and_then(text_value)
            .unwrap_or_else(|| title.to_owned());
        self.text(&mut img, &label, 24, 14., [160, 181, 199]);
        let value = feedback
            .get("value")
            .map(|v| text_value(v).unwrap_or_else(|| v.to_string()))
            .unwrap_or_default();
        self.text(&mut img, &value, 61, 25., [245, 247, 250]);
        if let Some(indicator) = feedback.get("indicator") {
            let v = indicator
                .as_f64()
                .or_else(|| indicator.get("value").and_then(Value::as_f64))
                .unwrap_or(0.);
            let min = indicator
                .get("range")
                .and_then(|v| v.get("min"))
                .and_then(Value::as_f64)
                .unwrap_or(0.);
            let max = indicator
                .get("range")
                .and_then(|v| v.get("max"))
                .and_then(Value::as_f64)
                .unwrap_or(100.);
            let fraction = if max > min {
                ((v - min) / (max - min)).clamp(0., 1.)
            } else {
                0.
            };
            for y in 78..85 {
                for x in 14..186 {
                    img.put_pixel(
                        x,
                        y,
                        Rgb(if x - 14 < (172. * fraction) as u32 {
                            [75, 211, 176]
                        } else {
                            [39, 56, 67]
                        }),
                    );
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }
}

fn text_value(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned).or_else(|| {
        value
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}
