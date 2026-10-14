//! Per-action Encoder overrides are distinct from page Controller backgrounds.
use super::*;
pub(super) const BACKGROUND: &str = "deckforge:encoder-background";
pub(super) const ICON: &str = "deckforge:encoder-icon";
pub(super) fn image<'a>(slot: &'a Slot, key: &str) -> Option<&'a DynamicImage> {
    slot.image_cache.get(key).map(|(_, image)| image)
}
impl Host {
    pub(super) fn sync_encoder_assets(&mut self) {
        let mut requests = Vec::new();
        for slot in self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| s.controller == "Encoder")
        {
            for (field, key) in [("background", BACKGROUND), ("Icon", ICON)] {
                let path = slot.raw["Encoder"][field]
                    .as_str()
                    .filter(|p| !p.is_empty());
                let origin = self.asset_origin(slot);
                let plugin = self
                    .actions
                    .iter()
                    .find(|a| a.uuid == slot.uuid)
                    .and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin))
                    .map(|p| p.root.clone());
                let owned = slot.encoder_overrides.get(field);
                let signature = serde_json::to_string(&(
                    path,
                    owned.is_some(),
                    origin.as_ref().map(|o| (&o.source, &o.manifest)),
                    &plugin,
                ))
                .unwrap();
                if path.is_none() && !slot.image_cache.contains_key(key) {
                    continue;
                }
                if path.is_some()
                    && slot
                        .image_cache
                        .get(key)
                        .is_some_and(|(old, _)| *old == signature)
                {
                    continue;
                }
                requests.push((
                    slot.context.clone(),
                    key,
                    signature,
                    path.map(str::to_owned),
                    origin,
                    plugin,
                    owned.cloned(),
                ));
            }
        }
        for (context, key, signature, path, origin, plugin, owned) in requests {
            let loaded = path.as_ref().map(|path| -> Result<DynamicImage> {
                if let Some(owned) = owned {
                    return display::decode_image(&STANDARD.decode(owned)?, false);
                }
                if let Some(origin) = origin {
                    if !self.profile_assets.contains_key(&origin.source) {
                        self.profile_assets
                            .insert(origin.source.clone(), profile::load_bundle(&origin.source)?);
                    }
                    let page = self.profile_assets[&origin.source]
                        .pages
                        .iter()
                        .find(|p| p.manifest_path() == origin.manifest)
                        .context("Encoder asset page missing")?;
                    return display::decode_image(
                        &page.asset(path)?,
                        path.to_ascii_lowercase().ends_with(".svg"),
                    );
                }
                load_image(plugin.as_deref().unwrap_or_else(|| Path::new(".")), path)
            });
            if let Some(slot) = self.slots.iter_mut().find_map(|s| s.find_mut(&context)) {
                match loaded {
                    None => {
                        slot.image_cache.remove(key);
                    }
                    Some(Ok(image)) => {
                        slot.image_cache.insert(key.into(), (signature, image));
                    }
                    Some(Err(error)) => {
                        slot.display_warnings
                            .push(format!("Encoder asset unavailable: {error:#}"));
                        slot.image_cache.insert(
                            key.into(),
                            (
                                signature,
                                DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1)),
                            ),
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::io::Write;
    fn make(path: &Path, uuid: &str, blue: bool) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(serde_json::to_string(&json!({"Name":"Encoder overrides","Device":{"Model":"20GBD9901"},"Controllers":[{"Type":"Encoder","Background":"Images/page.png","Actions":{"0,0":{"UUID":uuid,"Settings":{"unchanged":7},"States":[{"Title":" "}],"Encoder":{"Icon":"Images/icon.png","IconVisible":false,"background":"Images/back.png","Future":17}}}}]})).unwrap().as_bytes()).unwrap();
        for (name, image) in [
            (
                "Images/page.png",
                DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 100, Rgb([0, 255, 0]))),
            ),
            (
                "Images/icon.png",
                DynamicImage::ImageRgb8(RgbImage::from_pixel(72, 72, Rgb([255, 0, 0]))),
            ),
            (
                "Images/back.png",
                DynamicImage::ImageRgba8(image::RgbaImage::from_fn(200, 100, |x, _| {
                    if blue {
                        image::Rgba([0, 0, 255, if x < 50 { 0 } else { 255 }])
                    } else {
                        image::Rgba([0, 255, 0, 255])
                    }
                })),
            ),
        ] {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes.get_ref()).unwrap();
        }
        zip.finish().unwrap();
    }
    fn preview(host: &Host, index: u8) -> DynamicImage {
        image::load_from_memory(
            &STANDARD
                .decode(
                    &host
                        .slots
                        .iter()
                        .find(|s| s.controller == "Encoder" && s.index == index)
                        .unwrap()
                        .preview,
                )
                .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn separate_encoder_overrides_render_and_rebase_when_assets_conflict() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.actions[0].controllers.push("Encoder".into());
        host.actions[0].raw["Encoder"] = json!({"layout":"$X1"});
        let source = root.path().join("source.streamDeckProfile");
        make(&source, &host.actions[0].uuid, true);
        let id = host.import_library(&source, None, false).unwrap();
        host.select_profile(&id, None).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        host.hardware = Some(tx);
        host.slots[0].feedback = json!({"icon":{"enabled":true}});
        host.render().unwrap();
        let hidden = preview(&host, 0);
        let mut draft = host.slots[0].clone();
        let draft_image =
            display::dial_image(&host.renderer, &mut draft, None, &host.actions).unwrap();
        assert_eq!(&draft_image.get_pixel(100, 64).0[..3], &[0, 0, 255]);

        assert_eq!(&hidden.get_pixel(100, 64).0[..3], &[0, 0, 255]);
        assert_eq!(&hidden.get_pixel(10, 10).0[..3], &[0, 255, 0]);
        let physical = rx
            .try_iter()
            .find_map(|d| {
                if let Draw::Strip(i) = d {
                    Some(i)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            physical.crop_imm(0, 0, 200, 100).to_rgb8(),
            hidden.to_rgb8()
        );
        let icon = image::load_from_memory(&STANDARD.decode(&host.slots[0].software_icon).unwrap())
            .unwrap();
        assert_eq!(&icon.get_pixel(10, 10).0[..3], &[255, 0, 0]);
        assert_eq!(host.slots[0].feedback["icon"]["enabled"], true);
        host.slots[0].raw["Encoder"]["IconVisible"] = json!(true);
        host.render().unwrap();
        assert_eq!(&preview(&host, 0).get_pixel(100, 64).0[..3], &[255, 0, 0]);
        host.slots[0].raw["Encoder"]["IconVisible"] = json!(false);
        let context = host.slots[0].context.clone();
        host.transfer_command(&json!({"command":"slot_copy","context":context}))
            .unwrap();
        let destination = root.path().join("destination.streamDeckProfile");
        make(&destination, &host.actions[0].uuid, false);
        let id = host.import_library(&destination, None, false).unwrap();
        host.select_profile(&id, None).unwrap();
        host.transfer_command(&json!({"command":"slot_paste","controller":"Encoder","index":1,"expected_target":null})).unwrap();
        assert_eq!(&preview(&host, 1).get_pixel(100, 64).0[..3], &[0, 0, 255]);
        let output = root.path().join("export.streamDeckProfile");
        host.export_profile(&output).unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let page = bundle.pages.iter().find(|p| p.slots.len() == 2).unwrap();
        for slot in &page.slots {
            let pixels = image::load_from_memory(
                &page
                    .asset(slot.raw["Encoder"]["background"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                pixels.get_pixel(100, 64).0,
                if slot.column == 1 {
                    [0, 0, 255, 255]
                } else {
                    [0, 255, 0, 255]
                }
            );
            assert_eq!(slot.raw["Encoder"]["Future"], 17);
            assert_eq!(slot.raw["Encoder"]["IconVisible"], false);
        }
    }
}
