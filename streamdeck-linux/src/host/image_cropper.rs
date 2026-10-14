//! Crop decoding stays outside the engine lock; saving rechecks action ownership.
use super::*;
use anyhow::ensure;
fn check(host: &mut Host, request: &Value) -> Result<(Slot, String)> {
    ensure!(host.running, "Host has stopped");
    let context = request["context"]
        .as_str()
        .context("Action context missing")?;
    let owner = host
        .context_device(context)
        .context("Action no longer exists")?;
    ensure!(
        request.get("device").is_none_or(|d| d == &owner),
        "Action belongs to another device"
    );
    host.with_device(&owner, |host| {
        host.editable_context(context)?;
        let slot = host.slot_any(context).context("Action no longer exists")?;
        ensure!(
            slot.controller == "Encoder" && slot.model.capabilities().strip,
            "Action has no encoder display"
        );
        Ok((slot.clone(), owner.clone()))
    })
}

impl Controller {
    pub fn image_crop(&self, request: Value) -> Result<Value> {
        let save = match request["mode"].as_str() {
            Some("load") => false,
            Some("save") => true,
            _ => bail!("Unknown crop operation"),
        };
        {
            let mut host = self.shared.lock().unwrap();
            check(&mut host, &request)?;
        }
        let path = Path::new(request["path"].as_str().context("Image path missing")?);
        let index = usize::try_from(request["frame"].as_u64().unwrap_or(0))?;
        let source = crate::image_crop::read(path, index)?;
        if let Some(expected) = request.get("source_hash").and_then(Value::as_str) {
            ensure!(
                expected == source.hash,
                "Source image changed; choose it again before saving"
            );
        }
        if !save {
            let preview = source.image.thumbnail(1024, 1024);
            return Ok(
                json!({"preview":display_targets::png(&preview)?,"width":source.image.width(),"height":source.image.height(),"frames":source.frames,"frame":index,"source_hash":source.hash}),
            );
        }
        ensure!(
            request["source_hash"].is_string(),
            "Load the source before saving"
        );
        let rect = serde_json::from_value(request["rect"].clone())?;
        let cropped = crate::image_crop::render(&source.image, rect)?;
        let png = display_targets::png(&cropped)?;
        let mut host = self.shared.lock().unwrap();
        let (_, owner) = check(&mut host, &request)?;
        host.history_edit("Crop artwork", request["history_group"].as_str(), |host| {
            host.with_device(&owner, |host| {
                let (mut slot, _) = check(host, &request)?;
                ensure!(
                    request.get("expected") == Some(&slot.raw["Encoder"]["background"]),
                    "Background changed while editing; reopen Customize Image"
                );
                let previous = slot.clone();
                ensure!(
                    slot.raw["Encoder"].is_null() || slot.raw["Encoder"].is_object(),
                    "Invalid Encoder metadata"
                );
                if slot.raw["Encoder"].is_null() {
                    slot.raw["Encoder"] = json!({});
                }
                slot.raw["Encoder"]["background"] = json!(format!(
                    "Images/encoder-background-{}.png",
                    uuid::Uuid::new_v4()
                ));
                slot.encoder_overrides.insert("background".into(), png);
                host.store_slot(slot);
                if let Err(error) = host.persist() {
                    host.store_slot(previous);
                    return Err(error);
                }
                host.render()?;
                Ok(Value::Null)
            })
        })
    }
}
impl Host {
    pub(super) fn encoder_option(&mut self, request: &Value) -> Result<()> {
        let (mut slot, _) = check(self, request)?;
        let field = request["field"].as_str().context("Encoder field missing")?;
        ensure!(
            matches!(field, "background" | "IconVisible"),
            "Unknown Encoder field"
        );
        let value = request.get("value").context("Encoder value missing")?;
        ensure!(
            value.is_null() || (field == "IconVisible" && value.is_boolean()),
            "Invalid Encoder value"
        );
        ensure!(
            request.get("expected") == Some(&slot.raw["Encoder"][field]),
            "Encoder appearance changed; try again with the current action"
        );
        let previous = slot.clone();
        ensure!(
            slot.raw["Encoder"].is_null() || slot.raw["Encoder"].is_object(),
            "Invalid Encoder metadata"
        );
        if slot.raw["Encoder"].is_null() {
            slot.raw["Encoder"] = json!({});
        }
        if value.is_null() {
            slot.raw["Encoder"].as_object_mut().unwrap().remove(field);
            slot.encoder_overrides.remove(field);
        } else {
            slot.raw["Encoder"][field] = value.clone();
        }
        self.store_slot(slot);
        if let Err(error) = self.persist() {
            self.store_slot(previous);
            return Err(error);
        }
        self.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    #[test]
    fn crop_save_rechecks_source_and_target_and_embeds_portable_pixels() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.actions[0].controllers.push("Encoder".into());
        host.actions[0].raw["Encoder"] = json!({"layout":"$X1"});
        let icon_path = root.path().join("default-icon.png");
        DynamicImage::ImageRgb8(RgbImage::from_pixel(48, 48, Rgb([0, 255, 0])))
            .save(&icon_path)
            .unwrap();
        let icon_bytes = fs::read(&icon_path).unwrap();
        host.plugins[0].root = root.path().to_owned();
        host.actions[0].raw["States"] = json!([{"Image":"default-icon.png","Title":"Keep"}]);
        let uuid = host.actions[0].uuid.clone();
        host.assign("Encoder", 0, &uuid).unwrap();
        let context = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .context
            .clone();
        let device = host.device.clone();
        let controller = Controller {
            shared: Arc::new(Mutex::new(host)),
        };
        let path = root.path().join("source.png");
        DynamicImage::ImageRgb8(RgbImage::from_fn(400, 100, |x, _| {
            if x < 200 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        }))
        .save(&path)
        .unwrap();
        let original = fs::read(&path).unwrap();
        let load = controller
            .image_crop(json!({"mode":"load","context":context,"device":device,"path":path}))
            .unwrap();
        assert_eq!(
            (load["width"].as_u64(), load["height"].as_u64()),
            (Some(400), Some(100))
        );
        let request = json!({"mode":"save","context":context,"device":device,"path":path,"frame":0,"source_hash":load["source_hash"],"expected":null,"rect":{"x":-200,"y":0,"width":400,"height":100}});
        {
            controller
                .shared
                .lock()
                .unwrap()
                .library
                .current_mut()
                .unwrap()
                .readonly = true;
        }
        assert!(controller.image_crop(request.clone()).is_err());
        {
            controller
                .shared
                .lock()
                .unwrap()
                .library
                .current_mut()
                .unwrap()
                .readonly = false;
        }
        fs::write(&path, b"changed").unwrap();
        assert!(controller.image_crop(request.clone()).is_err());
        fs::write(&path, &original).unwrap();
        {
            let mut host = controller.shared.lock().unwrap();
            assert!(host.slot_any(&context).unwrap().raw["Encoder"]["background"].is_null());
            host.simulate = true;
            host.add_simulated_device("other", Model::Mini).unwrap();
            host.select_device("other").unwrap();
        }
        controller.image_crop(request.clone()).unwrap();
        assert!(
            controller.image_crop(request.clone()).is_err(),
            "stale save must fail"
        );
        fs::remove_file(&path).unwrap();
        let mut host = controller.shared.lock().unwrap();
        assert_eq!(host.device, "other");
        host.select_device(&device).unwrap();
        let slot = host.slot_any(&context).unwrap();
        let data = STANDARD
            .decode(&slot.encoder_overrides["background"])
            .unwrap();
        let image = image::load_from_memory(&data).unwrap();
        assert_eq!((image.width(), image.height()), (200, 100));
        assert_eq!(image.get_pixel(100, 50).0, [0, 0, 255, 255]);
        let current = slot.raw["Encoder"]["background"].clone();
        host.transfer_command(&json!({"command":"slot_copy","context":context}))
            .unwrap();
        host.library_command(&json!({"command":"profile_create","name":"Copied"}))
            .unwrap();
        host.transfer_command(&json!({"command":"slot_paste","controller":"Encoder","index":1,"expected_target":null})).unwrap();
        let output = root.path().join("crop.streamDeckProfile");
        host.export_profile(&output).unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let page = bundle.pages.iter().find(|p| !p.slots.is_empty()).unwrap();
        let raw = &page.slots[0].raw;
        assert!(raw.get("encoder_overrides").is_none());
        assert_eq!(
            page.asset(raw["States"][0]["Image"].as_str().unwrap())
                .unwrap(),
            icon_bytes
        );

        assert_eq!(
            page.asset(raw["Encoder"]["background"].as_str().unwrap())
                .unwrap(),
            data
        );
        let copied = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .context
            .clone();
        let expected = host.slot_any(&copied).unwrap().raw["Encoder"]["background"].clone();
        assert_eq!(expected, current);
        host.encoder_option(
            &json!({"context":copied,"field":"background","value":null,"expected":expected}),
        )
        .unwrap();
        assert!(host.slot_any(&copied).unwrap().encoder_overrides.is_empty());
    }
}
