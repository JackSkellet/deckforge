//! Resolve plugin-owned layout/image resources once at the display boundary.
use super::*;

pub(super) fn load_layout(root: Option<&Path>, name: &str) -> Result<Value> {
    let layout = if name.starts_with('$') {
        crate::layout::builtin(name)?
    } else {
        let path = safe_file(root.context("Custom layout has no installed plugin")?, name)?;
        if fs::metadata(&path)?.len() > 1024 * 1024 {
            bail!("Layout JSON exceeds 1 MiB");
        }
        serde_json::from_slice(&fs::read(path)?)?
    };
    crate::layout::validate(&layout)?;
    Ok(layout)
}

pub(super) fn dial_image(
    renderer: &Renderer,
    slot: &mut Slot,
    root: Option<&Path>,
) -> Result<DynamicImage> {
    if slot.layout_cache.is_none() {
        slot.layout_cache = Some(load_layout(root, &slot.layout)?);
    }
    let layout = slot.layout_cache.as_ref().unwrap();
    let mut images = HashMap::new();
    if let Some(background) = &slot.background {
        images.insert("icon".to_owned(), background.clone());
    }
    for item in crate::layout::resolved_items(layout, &slot.feedback)? {
        if item["type"] != "pixmap" {
            continue;
        }
        let key = item["key"].as_str().context("Pixmap key missing")?;
        if let Some(source) = item["value"].as_str() {
            if source.is_empty() {
                images.remove(key);
                continue;
            }
            if slot
                .image_cache
                .get(key)
                .is_none_or(|(cached, _)| cached != source)
            {
                let image = match load_image(root.unwrap_or_else(|| Path::new(".")), source) {
                    Ok(image) => image,
                    Err(error) => {
                        slot.display_warnings.push(format!("Layout image {source} for {key} unavailable: {error}; displaying that item blank"));
                        DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))
                    }
                };
                slot.image_cache
                    .insert(key.to_owned(), (source.to_owned(), image));
            }
            if let Some((_, image)) = slot.image_cache.get(key) {
                images.insert(key.to_owned(), image.clone());
            }
        }
    }
    renderer.dial_layout(&slot.title, &slot.feedback, layout, &images)
}

/// SDK scalar shorthand changes an item's value without erasing its properties.
pub(super) fn merge_feedback(current: &mut Value, update: &Value) {
    if let Some(properties) = update.as_object() {
        if !current.is_object() {
            let old = std::mem::replace(current, json!({}));
            if !old.is_null() {
                current["value"] = old;
            }
        }
        for (key, value) in properties {
            if matches!(key.as_str(), "font" | "range") && value.is_object() {
                if !current[key].is_object() {
                    current[key] = json!({});
                }
                current[key]
                    .as_object_mut()
                    .unwrap()
                    .extend(value.as_object().unwrap().clone());
            } else {
                current[key] = value.clone();
            }
        }
    } else if current.is_object() {
        current["value"] = update.clone();
    } else {
        *current = update.clone();
    }
}

pub(super) fn decode_image(data: &[u8], svg: bool) -> Result<DynamicImage> {
    if !svg {
        return Ok(image::load_from_memory(data)?);
    }
    let mut options = resvg::usvg::Options::default();
    // Resource paths are resolved by safe_file; an SVG cannot bypass that owner.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    static FONTS: std::sync::OnceLock<Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    options.fontdb = FONTS
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone();
    let tree = resvg::usvg::Tree::from_data(data, &options)?;
    let size = tree.size();
    let scale = (2048.0 / size.width().max(size.height())).min(1.0);
    let width = (size.width() * scale).ceil().max(1.) as u32;
    let height = (size.height() * scale).ceil().max(1.) as u32;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).context("SVG canvas too large")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3] as u16;
        for channel in &mut pixel[..3] {
            *channel = (*channel as u16 * 255)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    Ok(DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(width, height, rgba).context("Invalid SVG pixel buffer")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_shorthand_preserves_custom_range_and_style() {
        let mut value = json!({"value":40,"range":{"min":-50,"max":50},"bar_fill_c":"red"});
        merge_feedback(&mut value, &json!(12));
        assert_eq!(value["value"], 12);
        assert_eq!(value["range"]["min"], -50);
        assert_eq!(value["bar_fill_c"], "red");
        merge_feedback(&mut value, &json!({"range":{"max":100}}));
        assert_eq!(value["range"], json!({"min":-50,"max":100}));
        assert_eq!(value["value"], 12);
    }
    #[test]
    fn svg_alpha_is_unpremultiplied_and_data_urls_decode() {
        let image=decode_image(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><rect width="10" height="8" fill="red" opacity="0.5"/></svg>"#,true).unwrap().to_rgba8();
        let pixel = image.get_pixel(5, 4);
        assert_eq!(pixel[0], 255);
        assert!((126..=129).contains(&pixel[3]));
        let image=load_image(Path::new("."),"data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='3'%20height='4'/%3E").unwrap();
        assert_eq!((image.width(), image.height()), (3, 4));
    }
}
