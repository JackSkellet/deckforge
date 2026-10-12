//! Our own minimal renderer. Generic font/image libraries only.
use crate::appearance::TitleStyle;
use anyhow::{Context, Result, ensure};
use fontdue::{Font, FontSettings};
use image::{DynamicImage, Rgba, RgbaImage};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Debug, Serialize)]
pub struct FontInfo {
    pub requested_family: String,
    pub requested_style: String,
    pub resolved_family: String,
    pub resolved_style: String,
    pub fallback: bool,
    pub reason: Option<String>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
}
#[derive(Clone)]
struct ResolvedFont {
    font: Arc<Font>,
    info: FontInfo,
    bytes: usize,
}
#[derive(Default)]
struct FontCache {
    entries: VecDeque<((String, String), ResolvedFont)>,
}
#[derive(Clone)]
pub struct Renderer {
    font: Arc<Font>,
    bold_font: Option<Arc<Font>>,
    fonts: Arc<Mutex<FontCache>>,
}
// Native Linux design scaling, not a recovered Windows point-to-pixel formula.
const TITLE_SCALE: f32 = 120.0 / 72.0;
const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FONT_CACHE_BYTES: usize = 64 * 1024 * 1024;

impl Renderer {
    pub fn new() -> Result<Self> {
        // Font environment overrides are process-startup configuration. Preview
        // renderers share parsed default fonts and resolved title fonts with the
        // live device renderer, rather than probing again on each preview.
        static SHARED: OnceLock<std::result::Result<Renderer, String>> = OnceLock::new();
        SHARED
            .get_or_init(|| Self::load().map_err(|e| format!("{e:#}")))
            .clone()
            .map_err(anyhow::Error::msg)
    }
    fn load() -> Result<Self> {
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
        let (font, _) = load_font(Path::new(&path), 0)?;
        let bold_font = std::env::var("DECKFORGE_BOLD_FONT")
            .ok()
            .into_iter()
            .chain([
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf".to_owned(),
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".to_owned(),
                "/usr/share/fonts/noto/NotoSans-Bold.ttf".to_owned(),
            ])
            .find_map(|p| load_font(Path::new(&p), 0).ok().map(|(f, _)| Arc::new(f)));
        Ok(Self {
            font: Arc::new(font),
            bold_font,
            fonts: Arc::new(Mutex::new(FontCache::default())),
        })
    }
    pub fn font_info(&self, style: &TitleStyle) -> FontInfo {
        self.resolve_font(style).info
    }
    fn resolve_font(&self, style: &TitleStyle) -> ResolvedFont {
        let key = (style.font_family.clone(), style.font_style.clone());
        let mut cache = self.fonts.lock().unwrap();
        if let Some(index) = cache.entries.iter().position(|(k, _)| *k == key) {
            let entry = cache.entries.remove(index).unwrap();
            let result = entry.1.clone();
            cache.entries.push_back(entry);
            return result;
        }
        // A cold preview must not hold up cached device rendering during font
        // discovery or parsing. Concurrent misses may load independently.
        drop(cache);
        let result = style
            .validate()
            .and_then(|_| {
                let pattern = crate::appearance::font_pattern(style);
                let bytes = crate::appearance::fontconfig(
                    "fc-match",
                    &[
                        "--format",
                        "%{file}\n%{index}\n%{family}\n%{style}\n",
                        "--",
                        &pattern,
                    ],
                    16384,
                    Duration::from_millis(350),
                )?;
                let text = String::from_utf8(bytes).context("Font match is not UTF-8")?;
                let mut lines = text.lines();
                let path = lines
                    .next()
                    .filter(|s| !s.is_empty())
                    .context("Font path missing")?;
                let index: u32 = lines
                    .next()
                    .context("Font index missing")?
                    .parse()
                    .context("Invalid font index")?;
                ensure!(
                    index <= 0xffff,
                    "Variable-font named instances are not supported by this rasterizer"
                );
                let family = lines.next().context("Matched family missing")?.to_owned();
                let actual_style = lines.next().unwrap_or("Regular").to_owned();
                let (font, bytes) = load_font(Path::new(path), index)?;
                let synthetic_bold = style.bold() && !is_bold(&actual_style);
                let synthetic_italic = style.italic() && !is_italic(&actual_style);
                let requested = if style.font_family.is_empty() {
                    "Arial"
                } else {
                    &style.font_family
                };
                let family_exact = family
                    .split(',')
                    .any(|name| name.trim().eq_ignore_ascii_case(requested));
                let style_exact = equivalent_style(&key.1, &actual_style);
                let fallback = !family_exact || !style_exact || synthetic_bold || synthetic_italic;
                let reason = fallback.then(|| {
                    format!(
                        "Requested {} {}; using {} {}{}{}",
                        if style.font_family.is_empty() {
                            "default (Arial)"
                        } else {
                            &style.font_family
                        },
                        key.1,
                        family,
                        actual_style,
                        if synthetic_bold {
                            " with synthetic bold"
                        } else {
                            ""
                        },
                        if synthetic_italic {
                            " with synthetic italic"
                        } else {
                            ""
                        }
                    )
                });
                Ok(ResolvedFont {
                    font: Arc::new(font),
                    bytes,
                    info: FontInfo {
                        requested_family: style.font_family.clone(),
                        requested_style: style.font_style.clone(),
                        resolved_family: family,
                        resolved_style: actual_style,
                        fallback,
                        reason,
                        synthetic_bold,
                        synthetic_italic,
                    },
                })
            })
            .unwrap_or_else(|error| {
                let font = if style.bold() {
                    self.bold_font.as_ref().unwrap_or(&self.font).clone()
                } else {
                    self.font.clone()
                };
                let family = font.name().unwrap_or("configured fallback font").to_owned();
                let actual_bold = is_bold(&family);
                let actual_italic = is_italic(&family);
                ResolvedFont {
                    font,
                    bytes: 0,
                    info: FontInfo {
                        requested_family: style.font_family.clone(),
                        requested_style: style.font_style.clone(),
                        resolved_family: family,
                        resolved_style: if actual_bold { "Bold" } else { "Regular" }.into(),
                        fallback: true,
                        reason: Some(format!("{error:#}; using the configured fallback font")),
                        synthetic_bold: style.bold() && !actual_bold,
                        synthetic_italic: style.italic() && !actual_italic,
                    },
                }
            });
        let mut cache = self.fonts.lock().unwrap();
        // Reuse a result another caller inserted while this font was loading.
        if let Some(index) = cache.entries.iter().position(|(k, _)| *k == key) {
            let entry = cache.entries.remove(index).unwrap();
            let result = entry.1.clone();
            cache.entries.push_back(entry);
            return result;
        }
        while !cache.entries.is_empty()
            && (cache.entries.len() >= 12
                || cache.entries.iter().map(|(_, f)| f.bytes).sum::<usize>() + result.bytes
                    > MAX_FONT_CACHE_BYTES)
        {
            cache.entries.pop_front();
        }
        cache.entries.push_back((key, result.clone()));
        result
    }
    pub fn key(&self, title: &str, background: Option<&DynamicImage>) -> DynamicImage {
        self.key_styled(title, background, &TitleStyle::default())
    }
    pub fn key_styled(
        &self,
        title: &str,
        background: Option<&DynamicImage>,
        style: &TitleStyle,
    ) -> DynamicImage {
        self.key_styled_size(title, background, style, (120, 120))
    }
    /// Rectangular keys need a rectangular text canvas, not a stretched square.
    /// Keep title sizes relative to key height, matching the existing square policy.
    pub fn key_styled_size(
        &self,
        title: &str,
        background: Option<&DynamicImage>,
        style: &TitleStyle,
        size: (u32, u32),
    ) -> DynamicImage {
        let (width, height) = size;
        let scale = height as f32 / 120.0;
        let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
        if let Some(background) = background {
            let image = background
                .resize_exact(width, height, image::imageops::FilterType::Triangle)
                .to_rgba8();
            image::imageops::overlay(&mut canvas, &image, 0, 0);
        }
        let defaults = TitleStyle::default();
        let style = if style.validate().is_ok() {
            style
        } else {
            &defaults
        };
        if style.show_title && !title.is_empty() {
            let font = self.resolve_font(style);
            paint_title(
                &mut canvas,
                title,
                style,
                &font,
                style.font_size * TITLE_SCALE * scale,
                4. * scale,
            );
        }
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(canvas).to_rgb8())
    }

    /// Draw a validated dial layout. Images are decoded by the host and keyed by
    /// item key, so filesystem access remains outside the renderer.
    #[allow(dead_code)] // Compatibility entry point for callers without title overrides.
    pub fn dial_layout(
        &self,
        title: &str,
        feedback: &Value,
        layout: &Value,
        images: &HashMap<String, DynamicImage>,
    ) -> Result<DynamicImage> {
        self.dial_layout_styled(title, feedback, layout, images, None)
    }
    pub fn dial_layout_styled(
        &self,
        title: &str,
        feedback: &Value,
        layout: &Value,
        images: &HashMap<String, DynamicImage>,
        style: Option<&TitleStyle>,
    ) -> Result<DynamicImage> {
        if let Some(style) = style {
            style.validate()?;
        }
        let items = crate::layout::resolved_items(layout, feedback)?;
        let (width, height) = crate::layout::canvas_size(layout);
        let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
        for item in items {
            if item["enabled"].as_bool() == Some(false) {
                continue;
            }
            let [x, y, w, h] = if width == 200 {
                crate::layout::rect(&item)?
            } else {
                crate::layout::rect_in(&item, width, height)?
            };
            let opacity = number(&item["opacity"]).unwrap_or(1.).clamp(0., 1.) as f32;
            let mut layer = RgbaImage::new(w, h);
            if let Some(background) = item["background"].as_str() {
                fill(&mut layer, &Paint::parse(background)?);
            }
            let key = item["key"].as_str().unwrap();
            match item["type"].as_str().unwrap() {
                "pixmap" => {
                    if let Some(image) = images.get(key) {
                        let image = image
                            .resize_exact(w, h, image::imageops::FilterType::Triangle)
                            .to_rgba8();
                        image::imageops::overlay(&mut layer, &image, 0, 0);
                    }
                }
                "text" => {
                    let text = item
                        .get("value")
                        .and_then(display_value)
                        .unwrap_or_else(|| {
                            if key == "title" {
                                title.to_owned()
                            } else {
                                String::new()
                            }
                        });
                    if key == "title"
                        && let Some(style) = style
                    {
                        if style.show_title {
                            let font = self.resolve_font(style);
                            paint_title(
                                &mut layer,
                                &text,
                                style,
                                &font,
                                style.font_size * TITLE_SCALE,
                                1.,
                            );
                        }
                    } else {
                        self.layout_text(&mut layer, &text, &item)?;
                    }
                }
                "bar" | "gbar" => draw_bar(&mut layer, &item)?,
                _ => unreachable!("layout validation rejects unknown types"),
            }
            for pixel in layer.pixels_mut() {
                pixel.0[3] = (pixel.0[3] as f32 * opacity).round() as u8;
            }
            image::imageops::overlay(&mut canvas, &layer, x.into(), y.into());
        }
        Ok(DynamicImage::ImageRgb8(
            DynamicImage::ImageRgba8(canvas).to_rgb8(),
        ))
    }

    fn layout_text(&self, layer: &mut RgbaImage, text: &str, item: &Value) -> Result<()> {
        // Fontdue supplies glyph rasterization only. Alignment and clipping belong
        // to our renderer, and cannot leak into a neighboring dial or layout item.
        let font = if number(&item["font"]["weight"]).unwrap_or(400.) >= 600. {
            self.bold_font.as_ref().unwrap_or(&self.font)
        } else {
            &self.font
        };
        let size = number(&item["font"]["size"]).unwrap_or(16.).clamp(1., 200.) as f32;
        let color = Paint::parse(item["color"].as_str().unwrap_or("white"))?;
        let width = |s: &str| {
            s.chars()
                .map(|c| font.metrics(c, size).advance_width)
                .sum::<f32>()
        };
        let lines: Vec<_> = text.lines().take(100).collect();
        let line_height = font
            .horizontal_line_metrics(size)
            .map(|m| m.new_line_size)
            .unwrap_or(size);
        let ascent = font
            .horizontal_line_metrics(size)
            .map(|m| m.ascent)
            .unwrap_or(size);
        let height = line_height * lines.len() as f32;
        let top = (layer.height() as f32 - height) / 2.;
        for (line_number, line) in lines.into_iter().enumerate() {
            // Limit work for untrusted plugin strings; the visible item is <=200px.
            let mut line: String = line.chars().take(4096).collect();
            let overflow = width(&line) > layer.width() as f32;
            if overflow && item["text-overflow"] == "ellipsis" {
                while !line.is_empty() && width(&line) + width("…") > layer.width() as f32 {
                    line.pop();
                }
                line.push('…');
            }
            let text_width = width(&line);
            let mut x = match item["alignment"].as_str().unwrap_or("center") {
                "left" => 0.,
                "right" => layer.width() as f32 - text_width,
                _ => (layer.width() as f32 - text_width) / 2.,
            };
            let baseline = top + line_number as f32 * line_height + ascent;
            for ch in line.chars() {
                let (m, bitmap) = font.rasterize(ch, size);
                for row in 0..m.height {
                    for col in 0..m.width {
                        let px = x.floor() as i32 + m.xmin + col as i32;
                        let py = baseline.round() as i32 + row as i32 - m.ymin - m.height as i32;
                        if px < 0
                            || py < 0
                            || px >= layer.width() as i32
                            || py >= layer.height() as i32
                        {
                            continue;
                        }
                        let mut rgba = color.at(px as f64 / layer.width().max(1) as f64);
                        let mut alpha = bitmap[row * m.width + col] as f32 / 255.;
                        if overflow && item["text-overflow"] == "fade" {
                            alpha *= ((layer.width() as f32 - px as f32) / size).clamp(0., 1.);
                        }
                        rgba[3] = (rgba[3] as f32 * alpha).round() as u8;
                        blend(layer, px as u32, py as u32, rgba);
                    }
                }
                x += m.advance_width;
            }
        }
        Ok(())
    }
}

fn load_font(path: &Path, index: u32) -> Result<(Font, usize)> {
    let resolved = path.canonicalize().context("Resolve font path")?;
    let file = std::fs::File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&resolved)
        .context("Open font file")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_FONT_BYTES as u64,
        "Font must be a regular file no larger than 32 MiB"
    );
    let mut bytes = vec![];
    file.take(MAX_FONT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_FONT_BYTES, "Font exceeds 32 MiB");
    let length = bytes.len();
    let font = Font::from_bytes(
        bytes,
        FontSettings {
            collection_index: index,
            ..FontSettings::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("Font: {e}"))?;
    ensure!(
        font.chars().keys().take(512).any(|&c| {
            let m = font.metrics(c, 16.);
            m.width > 0 && m.height > 0
        }),
        "Font has no supported outline glyphs"
    );
    Ok((font, length))
}
fn is_bold(style: &str) -> bool {
    let s = style.to_ascii_lowercase();
    ["bold", "black", "heavy", "demi"]
        .iter()
        .any(|part| s.contains(part))
}
fn is_italic(style: &str) -> bool {
    let s = style.to_ascii_lowercase();
    s.contains("italic") || s.contains("oblique")
}
fn equivalent_style(requested: &str, actual: &str) -> bool {
    let regular = |s: &str| {
        ["", "regular", "book", "normal", "roman"].contains(&s.to_ascii_lowercase().as_str())
    };
    requested.eq_ignore_ascii_case(actual) || (regular(requested) && regular(actual))
}
fn paint_title(
    layer: &mut RgbaImage,
    text: &str,
    style: &TitleStyle,
    resolved: &ResolvedFont,
    size: f32,
    margin: f32,
) {
    // Keep blank lines and deterministic line boxes. Work is bounded independently
    // of the visible clipping; off-canvas glyphs are never rasterized.
    let text: String = text.chars().take(4096).collect();
    let lines: Vec<&str> = text.split('\n').take(128).collect();
    let font = &resolved.font;
    let metrics = font.horizontal_line_metrics(size);
    let line_height = metrics.map(|m| m.new_line_size).unwrap_or(size * 1.2);
    let ascent = metrics.map(|m| m.ascent).unwrap_or(size);
    let height = line_height * lines.len() as f32;
    let top = match style.title_alignment.as_str() {
        "top" => margin,
        "middle" => (layer.height() as f32 - height) / 2.,
        _ => layer.height() as f32 - margin - height,
    };
    let color = style.rgba();
    let bold = if resolved.info.synthetic_bold {
        (size / 18.).round().clamp(1., 3.) as i32
    } else {
        0
    };
    for (line_index, line) in lines.iter().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut width = 0.;
        let mut previous = None;
        for ch in line.chars() {
            if let Some(prev) = previous {
                width += font.horizontal_kern(prev, ch, size).unwrap_or(0.);
            }
            width += font.metrics(ch, size).advance_width;
            previous = Some(ch);
        }
        let baseline = top + line_index as f32 * line_height + ascent;
        if baseline + size < 0. || baseline - size * 2. > layer.height() as f32 {
            continue;
        }
        let mut pen = (layer.width() as f32 - width - bold as f32) / 2.;
        let start = pen;
        previous = None;
        for ch in line.chars() {
            if let Some(prev) = previous {
                pen += font.horizontal_kern(prev, ch, size).unwrap_or(0.);
            }
            previous = Some(ch);
            let m = font.metrics(ch, size);
            let left = pen.floor() as i32 + m.xmin;
            let top = baseline.round() as i32 - m.ymin - m.height as i32;
            let shear_extent = if resolved.info.synthetic_italic {
                size * 0.3
            } else {
                0.
            };
            if left as f32 + m.width as f32 + bold as f32 + shear_extent >= 0.
                && left as f32 - shear_extent < layer.width() as f32
                && top + m.height as i32 >= 0
                && top < layer.height() as i32
            {
                let (_, bitmap) = font.rasterize(ch, size);
                for row in 0..m.height {
                    let y = top + row as i32;
                    if y < 0 || y >= layer.height() as i32 {
                        continue;
                    }
                    let shear = if resolved.info.synthetic_italic {
                        ((baseline - y as f32) * 0.2).round() as i32
                    } else {
                        0
                    };
                    for col in 0..m.width {
                        let alpha = bitmap[row * m.width + col];
                        if alpha == 0 {
                            continue;
                        }
                        for offset in 0..=bold {
                            let x = left + col as i32 + shear + offset;
                            if x >= 0 && x < layer.width() as i32 {
                                blend(
                                    layer,
                                    x as u32,
                                    y as u32,
                                    [color[0], color[1], color[2], alpha],
                                );
                            }
                        }
                    }
                }
            }
            pen += m.advance_width;
        }
        if style.font_underline && width > 0. {
            let y = (baseline + size * 0.08).round() as i32;
            let thickness = (size / 16.).round().clamp(1., 5.) as i32;
            for row in y..y + thickness {
                if row < 0 || row >= layer.height() as i32 {
                    continue;
                }
                for col in (start.floor() as i32).max(0)
                    ..((start + width + bold as f32).ceil() as i32).min(layer.width() as i32)
                {
                    blend(layer, col as u32, row as u32, color);
                }
            }
        }
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn display_value(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        Some(s.to_owned())
    } else if value.is_number() || value.is_boolean() {
        Some(value.to_string())
    } else {
        None
    }
}

struct Paint(Vec<(f64, [u8; 4])>);

impl Paint {
    fn parse(input: &str) -> Result<Self> {
        if input.contains(':') {
            let mut stops = Vec::new();
            for stop in input.split(',') {
                let (offset, color) = stop.split_once(':').context("Invalid gradient stop")?;
                let offset: f64 = offset.trim().parse().context("Invalid gradient offset")?;
                anyhow::ensure!(
                    offset.is_finite() && (0. ..=1.).contains(&offset),
                    "Gradient offset must be between 0 and 1"
                );
                stops.push((offset, parse_color(color)?));
            }
            stops.sort_by(|a, b| a.0.total_cmp(&b.0));
            anyhow::ensure!(!stops.is_empty(), "Gradient needs a color stop");
            Ok(Self(stops))
        } else {
            Ok(Self(vec![(0., parse_color(input)?)]))
        }
    }

    fn at(&self, position: f64) -> [u8; 4] {
        if position <= self.0[0].0 {
            return self.0[0].1;
        }
        for pair in self.0.windows(2) {
            let [(start, a), (end, b)] = pair else {
                unreachable!()
            };
            if position <= *end {
                let t = if end > start {
                    ((position - start) / (end - start)).clamp(0., 1.)
                } else {
                    1.
                };
                return std::array::from_fn(|i| {
                    (a[i] as f64 * (1. - t) + b[i] as f64 * t).round() as u8
                });
            }
        }
        self.0.last().unwrap().1
    }
}

fn parse_color(input: &str) -> Result<[u8; 4]> {
    let color: csscolorparser::Color = input
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid layout color {input}: {e}"))?;
    Ok(color.to_rgba8())
}

fn fill(layer: &mut RgbaImage, paint: &Paint) {
    let divisor = layer.width().saturating_sub(1).max(1) as f64;
    for (x, _, pixel) in layer.enumerate_pixels_mut() {
        *pixel = Rgba(paint.at(x as f64 / divisor));
    }
}

fn blend(layer: &mut RgbaImage, x: u32, y: u32, color: [u8; 4]) {
    use image::Pixel;
    layer.get_pixel_mut(x, y).blend(&Rgba(color));
}

fn draw_bar(layer: &mut RgbaImage, item: &Value) -> Result<()> {
    let background = Paint::parse(item["bar_bg_c"].as_str().unwrap_or("darkgray"))?;
    let fill = Paint::parse(item["bar_fill_c"].as_str().unwrap_or("white"))?;
    let border = Paint::parse(item["bar_border_c"].as_str().unwrap_or("white"))?;
    let subtype = item["subtype"].as_u64().unwrap_or(4);
    anyhow::ensure!(subtype <= 4, "Unknown layout bar subtype {subtype}");
    // The official layouts sample GIF shows DoubleRectangle and
    // DoubleTrapezoid at half fill for value=0 with no explicit range.
    let default_min = if matches!(subtype, 1 | 3) { -100. } else { 0. };
    let min = number(&item["range"]["min"]).unwrap_or(default_min);
    let max = number(&item["range"]["max"]).unwrap_or(100.);
    anyhow::ensure!(
        max > min,
        "Layout bar range.max must be greater than range.min"
    );
    let value = number(&item["value"]).unwrap_or(min);
    let fraction = ((value - min) / (max - min)).clamp(0., 1.);
    let w = layer.width();
    let h = layer.height();
    let gradient = item["type"] == "gbar";
    let bar_h = if gradient {
        number(&item["bar_h"]).unwrap_or(10.).clamp(1., h as f64) as u32
    } else {
        h
    };
    let border_w = number(&item["border_w"])
        .unwrap_or(2.)
        .clamp(0., bar_h.min(w) as f64 / 2.) as u32;
    for y in 0..bar_h {
        for x in 0..w {
            if !bar_shape(x, y, w, bar_h, subtype) {
                continue;
            }
            let edge = border_w > 0
                && (x < border_w
                    || y < border_w
                    || x + border_w >= w
                    || y + border_w >= bar_h
                    || !bar_shape(x.saturating_sub(border_w), y, w, bar_h, subtype)
                    || !bar_shape((x + border_w).min(w - 1), y, w, bar_h, subtype)
                    || !bar_shape(x, y.saturating_sub(border_w), w, bar_h, subtype)
                    || !bar_shape(x, (y + border_w).min(bar_h - 1), w, bar_h, subtype));
            let t = x as f64 / w.saturating_sub(1).max(1) as f64;
            let inside_x = x.saturating_sub(border_w);
            let filled = inside_x < ((w - 2 * border_w) as f64 * fraction).round() as u32;
            let paint = if edge {
                &border
            } else if !gradient && filled {
                &fill
            } else {
                &background
            };
            blend(layer, x, y, paint.at(t));
        }
    }
    if gradient && bar_h < h {
        // The SDK defines a triangle below the bar; fit its tip to the value,
        // and clip both extremes to the item's rectangle.
        let center = ((w - 1) as f64 * fraction).round() as i32;
        let triangle_h = (h - bar_h).min(8);
        for y in 0..triangle_h {
            let half_width = y as i32;
            for dx in -half_width..=half_width {
                let x = center + dx;
                if x >= 0 && x < w as i32 {
                    blend(layer, x as u32, bar_h + y, fill.at(fraction));
                }
            }
        }
    }
    Ok(())
}

// These subtype silhouettes implement the SDK's named shapes. Exact native
// antialiasing and groove corner radii are not specified by the public contract.
fn bar_shape(x: u32, y: u32, w: u32, h: u32, subtype: u64) -> bool {
    match subtype {
        2 | 3 => {
            // Official sample layouts.gif: Trapezoid rises from left to right;
            // DoubleTrapezoid falls to the center and rises toward both ends.
            let t = x as f64 / w.saturating_sub(1).max(1) as f64;
            let t = if subtype == 3 { (2. * t - 1.).abs() } else { t };
            let rise = (t * h.saturating_sub(1) as f64).round() as u32;
            y >= h.saturating_sub(rise + 1)
        }
        4 => {
            let radius = (h / 2).min(w / 2) as f64;
            if radius <= 1. {
                return true;
            }
            let cx = (x as f64 + 0.5).clamp(radius, w as f64 - radius);
            let cy = (y as f64 + 0.5).clamp(radius, h as f64 - radius);
            (x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2) <= radius * radius
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;
    use serde_json::json;

    fn ink_bounds(image: &RgbImage) -> (u32, u32, u32, u32) {
        let points: Vec<_> = image
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0 != [0, 0, 0])
            .map(|(x, y, _)| (x, y))
            .collect();
        assert!(!points.is_empty());
        (
            points.iter().map(|p| p.0).min().unwrap(),
            points.iter().map(|p| p.1).min().unwrap(),
            points.iter().map(|p| p.0).max().unwrap(),
            points.iter().map(|p| p.1).max().unwrap(),
        )
    }

    #[test]
    fn font_loading_rejects_nonregular_and_oversized_files() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("font.fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a live NUL-terminated path into our own temporary directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(load_font(&fifo, 0).is_err());
        let path = dir.path().join("large.ttf");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FONT_BYTES as u64 + 1)
            .unwrap();
        assert!(load_font(&path, 0).is_err());
        assert!(load_font(dir.path(), 0).is_err());
    }

    #[test]
    fn key_alpha_and_hidden_title_preserve_background() {
        let renderer = Renderer::new().unwrap();
        let style = TitleStyle {
            show_title: false,
            ..TitleStyle::default()
        };
        let transparent =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 0])));
        let image = renderer
            .key_styled("Hidden", Some(&transparent), &style)
            .to_rgb8();
        assert_eq!(image.dimensions(), (120, 120));
        assert!(image.pixels().all(|p| p.0 == [0, 0, 0]));
        let alpha = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([200, 100, 0, 128])));
        let image = renderer
            .key_styled("Hidden", Some(&alpha), &style)
            .to_rgb8();
        assert!(image.pixels().all(|p| p.0 == [100, 50, 0]));
    }
    #[test]
    fn rectangular_key_titles_keep_glyph_proportions() {
        let renderer = Renderer::new().unwrap();
        let style = TitleStyle {
            title_alignment: "middle".into(),
            ..Default::default()
        };
        let square = renderer
            .key_styled_size("TEST", None, &style, (112, 112))
            .to_rgb8();
        let rectangle = renderer
            .key_styled_size("TEST", None, &style, (144, 112))
            .to_rgb8();
        // Added width only changes centering; it must not stretch glyphs as a
        // square render followed by nonuniform resizing would.
        for y in 0..112 {
            for x in 0..112 {
                assert_eq!(square.get_pixel(x, y), rectangle.get_pixel(x + 16, y));
            }
        }
        assert!(square.pixels().any(|pixel| pixel.0 != [0, 0, 0]));
    }

    #[test]
    fn title_alignment_multiline_color_and_underline_are_visible() {
        let renderer = Renderer::new().unwrap();
        let mut style = TitleStyle {
            font_family: "DejaVu Sans".into(),
            title_color: "#00ff00".into(),
            font_size: 12.,
            ..TitleStyle::default()
        };
        let mut tops = vec![];
        for alignment in ["top", "middle", "bottom"] {
            style.title_alignment = alignment.into();
            let image = renderer.key_styled("TEST", None, &style).to_rgb8();
            let bounds = ink_bounds(&image);
            tops.push(bounds.1);
            assert!((bounds.0 + bounds.2).abs_diff(119) <= 4);
            assert!(image.pixels().all(|p| p.0[0] == 0 && p.0[2] == 0));
        }
        assert!(tops[0] < tops[1] && tops[1] < tops[2]);
        style.title_alignment = "middle".into();
        let single = renderer.key_styled("TEST", None, &style).to_rgb8();
        let two = renderer.key_styled("TEST\nTEST", None, &style).to_rgb8();
        let a = ink_bounds(&single);
        let b = ink_bounds(&two);
        assert!(b.1 < a.1 && b.3 > a.3);
        style.font_underline = true;
        let underlined = renderer.key_styled("TEST", None, &style).to_rgb8();
        assert!(
            underlined.pixels().filter(|p| p.0[1] > 0).count()
                > single.pixels().filter(|p| p.0[1] > 0).count()
        );
        assert!(ink_bounds(&underlined).3 > a.3);
    }

    #[test]
    fn font_styles_render_distinctly_and_missing_family_reports_cached_fallback() {
        let renderer = Renderer::new().unwrap();
        let mut style = TitleStyle {
            font_family: "DejaVu Sans".into(),
            ..TitleStyle::default()
        };
        let regular = renderer.key_styled("Abc", None, &style).to_rgb8();
        style.font_style = "Bold".into();
        assert_ne!(renderer.key_styled("Abc", None, &style).to_rgb8(), regular);
        style.font_style = "Italic".into();
        assert_ne!(renderer.key_styled("Abc", None, &style).to_rgb8(), regular);
        style.font_family = "Deckforge missing family 013ac99".into();
        let first = renderer.resolve_font(&style);
        let second = Renderer::new().unwrap().resolve_font(&style);
        assert!(first.info.fallback && first.info.reason.is_some());
        assert!(!first.info.resolved_family.is_empty());
        assert!(Arc::ptr_eq(&first.font, &second.font));
    }

    #[test]
    fn dial_title_styles_do_not_change_other_feedback_items() {
        let renderer = Renderer::new().unwrap();
        let layout = json!({"id":"styled","items":[
            {"key":"title","type":"text","rect":[0,0,200,40],"font":{"size":13},"color":"blue"},
            {"key":"value","type":"text","rect":[0,50,200,40],"font":{"size":24},"color":"red"}
        ]});
        let feedback = json!({"title":"Plugin title","value":"42"});
        let plain = renderer
            .dial_layout("Fallback", &feedback, &layout, &HashMap::new())
            .unwrap()
            .to_rgb8();
        assert_eq!(
            plain,
            renderer
                .dial_layout_styled("Fallback", &feedback, &layout, &HashMap::new(), None)
                .unwrap()
                .to_rgb8()
        );
        let style = TitleStyle {
            show_title: false,
            ..TitleStyle::default()
        };
        let hidden = renderer
            .dial_layout_styled(
                "Fallback",
                &feedback,
                &layout,
                &HashMap::new(),
                Some(&style),
            )
            .unwrap()
            .to_rgb8();
        assert!(
            plain
                .enumerate_pixels()
                .any(|(_, y, p)| y < 40 && p.0 != [0, 0, 0])
        );
        assert!(
            hidden
                .enumerate_pixels()
                .all(|(_, y, p)| y >= 40 || p.0 == [0, 0, 0])
        );
        for y in 40..100 {
            for x in 0..200 {
                assert_eq!(plain.get_pixel(x, y), hidden.get_pixel(x, y));
            }
        }
    }

    fn render(layout: Value, feedback: Value, images: HashMap<String, DynamicImage>) -> RgbImage {
        Renderer::new()
            .unwrap()
            .dial_layout("", &feedback, &layout, &images)
            .unwrap()
            .to_rgb8()
    }

    #[test]
    fn official_canvas_layers_and_feedback_backgrounds() {
        let image = render(
            crate::layout::builtin("$A0").unwrap(),
            json!({"full-canvas":{"background":"blue"},"canvas":{"background":"#ffffff"}}),
            HashMap::new(),
        );
        assert_eq!(image.dimensions(), (200, 100));
        assert_eq!(image.get_pixel(0, 0).0, [0, 0, 255]);
        assert_eq!(image.get_pixel(16, 34).0, [255, 255, 255]);
        assert_eq!(image.get_pixel(151, 87).0, [255, 255, 255]);
        assert_eq!(image.get_pixel(152, 88).0, [0, 0, 255]);
    }

    #[test]
    fn official_indicator_geometry_numeric_strings_and_range_override() {
        let layout = crate::layout::builtin("$B1").unwrap();
        let image = render(
            layout,
            json!({"indicator":{"value":"0","range":{"min":-50,"max":50},
            "bar_fill_c":"red","bar_bg_c":"blue"}}),
            HashMap::new(),
        );
        assert_eq!(image.get_pixel(76, 80).0, [255, 0, 0]);
        assert_eq!(image.get_pixel(129, 80).0, [255, 0, 0]);
        assert_eq!(image.get_pixel(130, 80).0, [0, 0, 255]);
        assert_eq!(image.get_pixel(184, 80).0, [0, 0, 0]);
        assert_eq!(image.get_pixel(100, 73).0, [0, 0, 0]);
    }

    #[test]
    fn custom_pixmap_clips_scales_blends_and_honors_enabled() {
        let layout = json!({"id":"test","items":[
            {"key":"image","type":"pixmap","rect":[10,10,20,20],"background":"blue"}
        ]});
        let mut images = HashMap::new();
        images.insert(
            "image".into(),
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 128]))),
        );
        let image = render(layout.clone(), json!({}), images.clone());
        for (x, y) in [(10, 10), (29, 29)] {
            let color = image.get_pixel(x, y).0;
            assert!(color[0].abs_diff(128) <= 1 && color[1] == 0 && color[2].abs_diff(127) <= 1);
        }
        assert_eq!(image.get_pixel(30, 30).0, [0, 0, 0]);
        let image = render(layout, json!({"image":{"enabled":false}}), images);
        assert_eq!(image.get_pixel(10, 10).0, [0, 0, 0]);
    }

    #[test]
    fn gradient_background_and_triangle_follow_value() {
        let layout = crate::layout::builtin("$B2").unwrap();
        let image = render(
            layout,
            json!({"indicator":{"value":50,"subtype":0,
            "bar_bg_c":"0:red,1:blue","bar_fill_c":"lime"}}),
            HashMap::new(),
        );
        assert_eq!(image.get_pixel(76, 79).0, [255, 0, 0]);
        assert_eq!(image.get_pixel(183, 79).0, [0, 0, 255]);
        assert_eq!(image.get_pixel(130, 86).0, [0, 255, 0]);
        assert_eq!(image.get_pixel(110, 90).0, [0, 0, 0]);
    }

    #[test]
    fn independent_double_indicator_values_and_clamping() {
        let image = render(
            crate::layout::builtin("$C1").unwrap(),
            json!({"indicator1":-500,"indicator2":500}),
            HashMap::new(),
        );
        assert_eq!(image.get_pixel(80, 52).0, [169, 169, 169]);
        assert_eq!(image.get_pixel(80, 80).0, [255, 255, 255]);
    }

    #[test]
    fn text_is_clipped_to_rect_and_handles_numbers() {
        let layout = json!({"id":"text","items":[
            {"key":"value","type":"text","rect":[40,30,20,20],"font":{"size":16},"alignment":"left"}
        ]});
        let image = render(layout, json!({"value":12345678}), HashMap::new());
        assert!(image.enumerate_pixels().any(|(_, _, p)| p.0 != [0, 0, 0]));
        assert!(
            image
                .enumerate_pixels()
                .all(|(x, y, p)| p.0 == [0, 0, 0]
                    || ((40..60).contains(&x) && (30..50).contains(&y)))
        );
    }

    #[test]
    fn item_opacity_composites_after_drawing_in_z_order() {
        let layout = json!({"id":"layers","items":[
            {"key":"foreground","type":"pixmap","rect":[0,0,20,20],"background":"red","opacity":0.5,"zOrder":2},
            {"key":"background","type":"pixmap","rect":[0,0,20,20],"background":"blue","zOrder":1}
        ]});
        let image = render(layout, json!({}), HashMap::new());
        assert_eq!(image.get_pixel(10, 10).0, [128, 0, 127]);
    }

    #[test]
    fn colors_and_invalid_ranges_report_errors() {
        assert_eq!(parse_color("rebeccapurple").unwrap(), [102, 51, 153, 255]);
        assert_eq!(parse_color("#1234").unwrap(), [17, 34, 51, 68]);
        assert!(Paint::parse("0:red,2:blue").is_err());
        let renderer = Renderer::new().unwrap();
        assert!(
            renderer
                .dial_layout(
                    "",
                    &json!({"indicator":{"range":{"min":1,"max":1}}}),
                    &crate::layout::builtin("$B1").unwrap(),
                    &HashMap::new()
                )
                .is_err()
        );
    }

    #[test]
    fn official_custom_sample_double_bars_are_centered_at_zero() {
        let definition: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/official-layouts/custom-layout-4.json"
        ))
        .unwrap();
        let image = render(definition, json!({}), HashMap::new());
        // Official sample layouts.gif frames 59 and 78 show subtype 1/3 half
        // filled for their declared value=0. They are horizontal bipolar bars,
        // not vertically stacked bars.
        assert_eq!(image.get_pixel(60, 27).0, [255, 255, 255]);
        assert_eq!(image.get_pixel(160, 27).0, [169, 169, 169]);
        assert_eq!(image.get_pixel(60, 82).0, [255, 255, 255]);
        assert_eq!(image.get_pixel(160, 82).0, [169, 169, 169]);
        assert_eq!(image.get_pixel(115, 74).0, [0, 0, 0]);
    }

    #[test]
    fn all_bar_subtypes_render_without_drawing_outside_the_item() {
        for subtype in 0..5 {
            let mut layer = RgbaImage::new(136, 10);
            draw_bar(
                &mut layer,
                &json!({"type":"bar","value":60,"subtype":subtype,"border_w":1}),
            )
            .unwrap();
            assert!(layer.pixels().any(|p| p.0[3] > 0));
        }
    }
}
