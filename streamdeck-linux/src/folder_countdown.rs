//! Independently drawn native folder countdown ring (7.6 painter contract).
use image::{DynamicImage, GenericImage, GenericImageView, Pixel, Rgba};

pub fn paint(image: &mut DynamicImage, percent: u8) {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return;
    }
    let size = f64::from(width.min(height));
    let center = [f64::from((width - 1) / 2), f64::from((height - 1) / 2)];
    let radius = (size * 0.75).round() / 2.0;
    let half_pen = (size / 72.0 * 6.0).floor().max(1.0) / 2.0;
    let sweep = f64::from(percent.min(100)) / 100.0 * std::f64::consts::TAU;
    for y in 0..height {
        for x in 0..width {
            let mut ring = 0u32;
            let mut blue = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = f64::from(x) + (f64::from(sx) + 0.5) / 4.0 - center[0];
                    let dy = f64::from(y) + (f64::from(sy) + 0.5) / 4.0 - center[1];
                    let on_ring = (dx.hypot(dy) - radius).abs() <= half_pen;
                    // Clockwise from twelve o'clock. Qt uses round arc caps.
                    let angle = dx.atan2(-dy).rem_euclid(std::f64::consts::TAU);
                    let cap = dx.hypot(dy + radius) <= half_pen
                        || (dx - radius * sweep.sin()).hypot(dy + radius * sweep.cos()) <= half_pen;
                    let on_blue = percent > 0 && ((on_ring && angle <= sweep) || cap);
                    ring += u32::from(on_ring || on_blue);
                    blue += u32::from(on_blue);
                }
            }
            let mut pixel = image.get_pixel(x, y);
            pixel.blend(&Rgba([0, 0, 0, 128]));
            if ring > 0 {
                let grey = ring - blue;
                pixel.blend(&Rgba([
                    (217 * grey / ring) as u8,
                    ((217 * grey + 102 * blue) / ring) as u8,
                    ((217 * grey + 255 * blue) / ring) as u8,
                    ((ring * 255 + 8) / 16) as u8,
                ]));
            }
            image.put_pixel(x, y, pixel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_sweeps_clockwise_over_a_half_darkened_key() {
        let original = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            72,
            72,
            Rgba([100, 80, 60, 255]),
        ));
        let mut start = original.clone();
        paint(&mut start, 0);
        assert_eq!(start.get_pixel(35, 35)[0], 49);
        assert_eq!(start.get_pixel(62, 35).0, [217, 217, 217, 255]);
        let mut half = original.clone();
        paint(&mut half, 50);
        assert_eq!(half.get_pixel(62, 35).0, [0, 102, 255, 255]);
        assert_eq!(half.get_pixel(8, 35).0, [217, 217, 217, 255]);
        let mut finish = original;
        paint(&mut finish, 100);
        assert_eq!(finish.get_pixel(8, 35).0, [0, 102, 255, 255]);
    }
}
