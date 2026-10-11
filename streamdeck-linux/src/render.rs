//! Our own minimal renderer. Generic font/image libraries only.
use anyhow::{Context, Result};
use fontdue::{Font, FontSettings};
use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::Value;
use std::collections::HashMap;

pub struct Renderer {
    font: Font,
    bold_font: Option<Font>,
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
        let bold_font = std::env::var("DECKFORGE_BOLD_FONT")
            .ok()
            .into_iter()
            .chain([
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf".to_owned(),
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".to_owned(),
                "/usr/share/fonts/noto/NotoSans-Bold.ttf".to_owned(),
            ])
            .find_map(|p| {
                std::fs::read(p)
                    .ok()
                    .and_then(|b| Font::from_bytes(b, FontSettings::default()).ok())
            });
        Ok(Self { font, bold_font })
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

    /// Draw a validated dial layout. Images are decoded by the host and keyed by
    /// item key, so filesystem access remains outside the renderer.
    pub fn dial_layout(
        &self,
        title: &str,
        feedback: &Value,
        layout: &Value,
        images: &HashMap<String, DynamicImage>,
    ) -> Result<DynamicImage> {
        let items = crate::layout::resolved_items(layout, feedback)?;
        let mut canvas = RgbaImage::from_pixel(200, 100, Rgba([0, 0, 0, 255]));
        for item in items {
            if item["enabled"].as_bool() == Some(false) {
                continue;
            }
            let [x, y, w, h] = crate::layout::rect(&item)?;
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
                    self.layout_text(&mut layer, &text, &item)?;
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
    use serde_json::json;

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
            "../../evidence/plugins/com.elgato.layouts.sdPlugin/layouts/custom-layout-4.json"
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
