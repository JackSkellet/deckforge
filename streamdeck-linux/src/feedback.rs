//! Independently drawn status-badge geometry. Timing and controller policy belong to the host.
use image::{DynamicImage, GenericImage, GenericImageView, Pixel, Rgba};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Ok,
    Alert,
}

const GREEN: [u8; 3] = [34, 168, 94];
const YELLOW: [u8; 3] = [255, 200, 48];
const WHITE: [u8; 3] = [255, 255, 255];
const DARK: [u8; 3] = [37, 31, 18];

/// Paint a centered badge over the existing image without resizing or clearing it.
/// A 4×4 coverage grid smooths edges, including on 56-pixel child-action icons.
/// Pixels outside the badge remain untouched, including their original alpha.
pub fn paint(image: &mut DynamicImage, kind: Kind) {
    paint_with_opacity(image, kind, 1.0);
}

pub fn paint_with_opacity(image: &mut DynamicImage, kind: Kind, opacity: f64) {
    paint_sweep(image, kind, opacity, 1.0);
}

/// Reveal the success artwork clockwise from twelve o'clock, as in the native
/// Multi Action progress image. Completion uses the ordinary timed full badge.
pub fn paint_progress(image: &mut DynamicImage, percent: u8) {
    if percent > 0 {
        paint_sweep(image, Kind::Ok, 1.0, f64::from(percent.min(100)) / 100.0);
    }
}

fn paint_sweep(image: &mut DynamicImage, kind: Kind, opacity: f64, sweep: f64) {
    if !opacity.is_finite() || opacity <= 0.0 {
        return;
    }
    let opacity = opacity.min(1.0);
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return;
    }
    let size = f64::from(width.min(height)) * 0.64;
    let left = (f64::from(width) - size) / 2.0;
    let top = (f64::from(height) - size) / 2.0;
    for y in top.floor() as u32..((top + size).ceil() as u32).min(height) {
        for x in left.floor() as u32..((left + size).ceil() as u32).min(width) {
            let mut sum = [0u32; 3];
            let mut covered = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (f64::from(x) + (f64::from(sx) + 0.5) / 4.0 - left) / size;
                    let py = (f64::from(y) + (f64::from(sy) + 0.5) / 4.0 - top) / size;
                    if sweep < 1.0
                        && (px - 0.5).atan2(0.5 - py).rem_euclid(std::f64::consts::TAU)
                            > sweep * std::f64::consts::TAU
                    {
                        continue;
                    }
                    if let Some(color) = sample(kind, px, py) {
                        for (total, channel) in sum.iter_mut().zip(color) {
                            *total += u32::from(channel);
                        }
                        covered += 1;
                    }
                }
            }
            if let Some(covered) = std::num::NonZeroU32::new(covered) {
                // Average covered colors before source-over composition. Averaging
                // transparent black into RGB would leave dark antialiased edges.
                let mut badge = [0; 4];
                for i in 0..3 {
                    badge[i] = ((sum[i] + covered.get() / 2) / covered.get()) as u8;
                }
                badge[3] = (((covered.get() * 255 + 8) / 16) as f64 * opacity).round() as u8;
                let mut pixel = image.get_pixel(x, y);
                pixel.blend(&Rgba(badge));
                image.put_pixel(x, y, pixel);
            }
        }
    }
}

fn sample(kind: Kind, x: f64, y: f64) -> Option<[u8; 3]> {
    match kind {
        Kind::Ok => {
            if (x - 0.5).powi(2) + (y - 0.5).powi(2) > 0.25 {
                return None;
            }
            let check = segment_distance(x, y, [0.24, 0.51], [0.43, 0.69]) < 0.055
                || segment_distance(x, y, [0.43, 0.69], [0.77, 0.32]) < 0.055;
            Some(if check { WHITE } else { GREEN })
        }
        Kind::Alert => {
            // Vertices (0.5, 0), (0, 1), (1, 1), expressed as half-plane tests.
            if !(0.0..=1.0).contains(&y) || (x - 0.5).abs() > y * 0.5 {
                return None;
            }
            let stem = segment_distance(x, y, [0.5, 0.37], [0.5, 0.65]) < 0.055;
            let dot = (x - 0.5).powi(2) + (y - 0.82).powi(2) < 0.055f64.powi(2);
            Some(if stem || dot { DARK } else { YELLOW })
        }
    }
}

fn segment_distance(x: f64, y: f64, start: [f64; 2], end: [f64; 2]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let position =
        (((x - start[0]) * dx + (y - start[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (x - start[0] - position * dx).hypot(y - start[1] - position * dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, RgbaImage};

    #[test]
    fn progress_reveals_clockwise_sector_and_full_image_matches_success() {
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(120, 120, Rgba([12, 20, 35, 150])));
        let mut empty = base.clone();
        paint_progress(&mut empty, 0);
        assert_eq!(empty.to_rgba8(), base.to_rgba8());
        for (percent, limit_y) in [(25, 60), (50, 120)] {
            let mut progress = base.clone();
            paint_progress(&mut progress, percent);
            let mut changed = 0;
            for y in 0..120 {
                for x in 0..120 {
                    if x < 60 || y >= limit_y {
                        assert_eq!(progress.get_pixel(x, y), base.get_pixel(x, y));
                    }
                    if progress.get_pixel(x, y) != base.get_pixel(x, y) {
                        changed += 1;
                    }
                }
            }
            assert!(changed > 100);
        }
        let mut full = base.clone();
        let mut success = base;
        paint_progress(&mut full, 100);
        paint(&mut success, Kind::Ok);
        assert_eq!(full.to_rgba8(), success.to_rgba8());
    }

    #[test]
    fn requested_sizes_keep_dimensions_and_outside_pixels_exact() {
        for (width, height) in [(120, 120), (200, 100), (56, 56)] {
            for kind in [Kind::Ok, Kind::Alert] {
                let original = RgbaImage::from_fn(width, height, |x, y| {
                    Rgba([x as u8, y as u8, 73, (x + y) as u8])
                });
                let mut painted = DynamicImage::ImageRgba8(original.clone());
                paint(&mut painted, kind);
                assert_eq!(painted.dimensions(), (width, height));
                let size = f64::from(width.min(height)) * 0.64;
                let left = (f64::from(width) - size) / 2.0;
                let top = (f64::from(height) - size) / 2.0;
                let mut changed = 0;
                for (x, y, pixel) in original.enumerate_pixels() {
                    if x < left.floor() as u32
                        || x >= (left + size).ceil() as u32
                        || y < top.floor() as u32
                        || y >= (top + size).ceil() as u32
                    {
                        assert_eq!(painted.get_pixel(x, y), *pixel);
                    }
                    changed += usize::from(painted.get_pixel(x, y) != *pixel);
                }
                assert!(changed > (width.min(height) * width.min(height) / 8) as usize);
            }
        }
    }

    #[test]
    fn both_symbols_contain_their_background_and_mark_colors_at_all_sizes() {
        for (width, height) in [(120, 120), (200, 100), (56, 56)] {
            for (kind, background, mark) in [(Kind::Ok, GREEN, WHITE), (Kind::Alert, YELLOW, DARK)]
            {
                let mut image = DynamicImage::new_rgba8(width, height);
                paint(&mut image, kind);
                let colors: Vec<_> = image.pixels().map(|(_, _, p)| p.0).collect();
                assert!(colors.contains(&[background[0], background[1], background[2], 255]));
                assert!(colors.contains(&[mark[0], mark[1], mark[2], 255]));
                assert!(colors.iter().any(|p| p[3] > 0 && p[3] < 255));
            }
        }
    }

    #[test]
    fn coverage_alpha_composites_without_black_fringe() {
        let mut transparent = DynamicImage::new_rgba8(56, 56);
        paint(&mut transparent, Kind::Ok);
        let (x, y, edge) = transparent
            .pixels()
            .find(|(_, _, p)| p[3] > 0 && p[3] < 255)
            .unwrap();
        assert_eq!(&edge.0[..3], &GREEN);
        let base = Rgba([12, 31, 87, 90]);
        let mut translucent = DynamicImage::ImageRgba8(RgbaImage::from_pixel(56, 56, base));
        paint(&mut translucent, Kind::Ok);
        let mut expected = base;
        expected.blend(&edge);
        assert_eq!(translucent.get_pixel(x, y), expected);
        assert!(translucent.get_pixel(x, y)[3] > 90);
        assert_eq!(translucent.get_pixel(0, 0), base);
    }

    #[test]
    fn rgb_storage_opaque_alpha_and_tiny_empty_images_are_supported() {
        let mut image = DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 100, Rgb([8, 9, 10])));
        paint(&mut image, Kind::Alert);
        assert!(image.as_rgb8().is_some());
        assert!(image.pixels().all(|(_, _, p)| p[3] == 255));
        assert_eq!(image.get_pixel(0, 0), Rgba([8, 9, 10, 255]));
        for (width, height) in [(0, 0), (0, 4), (4, 0), (1, 1), (1, 9), (2, 2)] {
            for kind in [Kind::Ok, Kind::Alert] {
                let mut image = DynamicImage::new_rgba8(width, height);
                paint(&mut image, kind);
                assert_eq!(image.dimensions(), (width, height));
            }
        }
    }
}
