//! Small, bounded thumbnails for the native action library.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use image::DynamicImage;
use std::{
    fs::OpenOptions,
    io::{Cursor, Read},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const SIDE: u32 = 64;

pub fn load(root: &Path, icon: &str) -> Result<egui::ColorImage> {
    ensure!(!icon.is_empty(), "No icon declared");
    let root = root.canonicalize()?;
    let mut last = None;
    for suffix in ["", ".svg", ".png", ".jpg", ".jpeg", ".webp"] {
        let path = root.join(format!("{icon}{suffix}"));
        if !path.exists() {
            continue;
        }
        let result = (|| {
            let path = path.canonicalize()?;
            ensure!(
                path.starts_with(&root),
                "Action icon escapes its plugin folder"
            );
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
                .open(&path)?;
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file() && metadata.len() <= MAX_BYTES,
                "Icon is not a bounded regular file"
            );
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() as u64 <= MAX_BYTES, "Icon exceeds 2 MiB");
            decode(
                &bytes,
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("svg")),
            )
        })();
        match result {
            Ok(image) => return Ok(image),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("Action icon was not found")))
}

pub fn png(root: &Path, icon: &str) -> Result<Vec<u8>> {
    let thumbnail = load(root, icon)?;
    let bytes: Vec<u8> = thumbnail
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    let image =
        image::RgbaImage::from_raw(thumbnail.size[0] as u32, thumbnail.size[1] as u32, bytes)
            .context("Invalid action icon dimensions")?;
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image).write_to(&mut encoded, image::ImageFormat::Png)?;
    Ok(encoded.into_inner())
}

fn raster(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}

fn decode(bytes: &[u8], svg: bool) -> Result<egui::ColorImage> {
    if svg {
        let mut options = resvg::usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        // Embedded raster images obey the same decode limits. Nested SVGs and
        // external references are not needed for the static icon thumbnail.
        options.image_href_resolver.resolve_data = Box::new(|_, data, _| {
            let image = raster(&data).ok()?;
            let mut png = Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png).ok()?;
            Some(resvg::usvg::ImageKind::PNG(std::sync::Arc::new(
                png.into_inner(),
            )))
        });
        let tree = resvg::usvg::Tree::from_data(bytes, &options)?;
        let size = tree.size();
        let scale = SIDE as f32 / size.width().max(size.height());
        let mut pixels =
            resvg::tiny_skia::Pixmap::new(SIDE, SIDE).context("Icon allocation failed")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
                (SIDE as f32 - size.width() * scale) / 2.0,
                (SIDE as f32 - size.height() * scale) / 2.0,
            ),
            &mut pixels.as_mut(),
        );
        return Ok(egui::ColorImage::from_rgba_premultiplied(
            [SIDE as usize, SIDE as usize],
            pixels.data(),
        ));
    }
    let image = raster(bytes)?.thumbnail(SIDE, SIDE).to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn extensionless_svg_thumbnail_preserves_transparency_and_geometry() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("icon.svg"), br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="#ff0000"/></svg>"##).unwrap();
        let icon = load(root.path(), "icon").unwrap();
        assert_eq!(icon.size, [64, 64]);
        assert_eq!(icon.pixels[0].a(), 0);
        assert_eq!(icon.pixels[32 * 64 + 32], egui::Color32::RED);
    }
    #[test]
    fn webp_library_thumbnail_preserves_transparency() {
        let root = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(16, 8, image::Rgba([220, 30, 50, 128]))
            .save(root.path().join("action.icon.webp"))
            .unwrap();
        let icon = load(root.path(), "action.icon").unwrap();
        assert_eq!(icon.size, [64, 32]);
        assert_eq!(icon.pixels[0].a(), 128);
    }

    #[test]
    fn icon_paths_cannot_read_outside_the_plugin_or_special_files() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]))
            .save(outside.path().join("icon.png"))
            .unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("icon.png"),
            root.path().join("escape.png"),
        )
        .unwrap();
        assert!(load(root.path(), "escape.png").is_err());
        assert!(
            load(
                root.path(),
                outside.path().join("icon.png").to_str().unwrap()
            )
            .is_err()
        );
        fs::create_dir(root.path().join("directory.png")).unwrap();
        assert!(load(root.path(), "directory.png").is_err());
    }
    #[test]
    fn raster_thumbnail_and_limits() {
        let root = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(120, 60, image::Rgba([0, 255, 0, 128]))
            .save(root.path().join("icon.png"))
            .unwrap();
        let icon = load(root.path(), "icon").unwrap();
        assert_eq!(icon.size, [64, 32]);
        assert_eq!(icon.pixels[0].a(), 128);
        image::GrayImage::new(4097, 1)
            .save(root.path().join("wide.png"))
            .unwrap();
        assert!(load(root.path(), "wide.png").is_err());
        fs::write(
            root.path().join("large.svg"),
            vec![b' '; MAX_BYTES as usize + 1],
        )
        .unwrap();
        assert!(load(root.path(), "large.svg").is_err());
    }
}
