//! SDK resource paths and portable profile bindings, independent of plugin settings.
use super::*;
use std::collections::BTreeMap;

fn resource_paths(payload: Value) -> Result<BTreeMap<String, String>> {
    let input: BTreeMap<String, String> =
        serde_json::from_value(payload).context("Resources must map keys to file paths")?;
    if input.len() > 1024 {
        bail!("Too many resource bindings");
    }
    let mut paths = BTreeMap::new();
    for (key, path) in input {
        // The original setter drops empty values before replacing the entire map.
        if path.is_empty() {
            continue;
        }
        if key.len() > 4096
            || path.len() > 16384
            || path.contains('\0')
            || !Path::new(&path).is_absolute()
        {
            bail!("Resource paths must be nonempty absolute local paths");
        }
        paths.insert(key, path);
    }
    Ok(paths)
}
impl Host {
    pub(super) fn resources_event(&self, slot: &Slot, id: Option<&Value>) -> Value {
        let device = self
            .context_device(&slot.context)
            .unwrap_or_else(|| self.device.clone());
        let mut reply = json!({"event":"didReceiveResources","action":slot.uuid,"context":slot.context,"device":device,"payload":action_payload(slot)});
        if let Some(id) = id {
            reply["id"] = id.clone();
        }
        reply
    }
    pub(super) fn set_resources(&mut self, slot: &Slot, payload: Value) -> Result<Slot> {
        if let Some(device) = self
            .context_device(&slot.context)
            .filter(|device| device != &self.device)
        {
            return self.with_device(&device, |host| host.set_resources(slot, payload));
        }
        let paths = resource_paths(payload)?;
        let mut changed = slot.clone();
        changed.resources = Some(paths);
        self.store_slot(changed.clone());
        self.persist()?;
        Ok(changed)
    }
    fn imported_resources(&mut self, slot: &Slot) -> Result<BTreeMap<String, String>> {
        let Some(raw) = slot.raw.get("Resources").filter(|value| !value.is_null()) else {
            return Ok(BTreeMap::new());
        };
        let refs: BTreeMap<String, String> = serde_json::from_value(raw.clone())
            .context("Imported Resources must map keys to resource IDs")?;
        if refs.is_empty() {
            return Ok(BTreeMap::new());
        }
        let origin = self
            .asset_origin(slot)
            .context("Resource archive missing")?;
        let profile_id = origin.profile;
        let source = origin.source;
        if !self.profile_assets.contains_key(&source) {
            self.profile_assets
                .insert(source.clone(), profile::load_bundle(&source)?);
        }
        let directory = self
            .state_path
            .parent()
            .context("State directory missing")?
            .canonicalize()
            .context("Resolve state directory")?
            .join("plugin-resources");
        let mut paths = BTreeMap::new();
        for (key, id) in refs {
            let (name, bytes) = self.profile_assets[&source].resource(&id)?;
            let path = crate::resources::materialize(&directory, &profile_id, &id, &name, &bytes)?;
            paths.insert(
                key,
                path.to_str()
                    .context("Resource path must be UTF-8")?
                    .to_owned(),
            );
        }
        Ok(paths)
    }
    pub(super) fn hydrate_resources(&mut self) {
        let slots: Vec<_> = self
            .library
            .profiles
            .iter()
            .flat_map(|p| &p.pages)
            .flat_map(|p| &p.slots)
            .flat_map(Slot::flatten)
            .filter(|s| s.resources.is_none())
            .filter(|s| {
                self.actions
                    .iter()
                    .any(|a| a.uuid == s.uuid && !a.plugin.starts_with("deckforge."))
            })
            .cloned()
            .collect();
        for slot in slots {
            let result = if let Some(device) = self
                .context_device(&slot.context)
                .filter(|device| device != &self.device)
            {
                self.with_device(&device, |host| host.hydrate_resource_slot(slot.clone()))
            } else {
                self.hydrate_resource_slot(slot.clone())
            };
            match result {
                Ok(()) => {}
                Err(error) => {
                    let message = format!("Resources for {} are unavailable: {error:#}", slot.name);
                    if !self.logs.contains(&message) {
                        self.log(message);
                    }
                }
            }
        }
    }
    fn hydrate_resource_slot(&mut self, mut slot: Slot) -> Result<()> {
        slot.resources = Some(self.imported_resources(&slot)?);
        self.store_slot(slot);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_replacement_omits_empty_paths_but_does_not_require_files_to_exist() {
        let paths =
            resource_paths(json!({"retained":"/nonexistent/demo.wav","removed":""})).unwrap();
        assert_eq!(
            paths,
            BTreeMap::from([("retained".into(), "/nonexistent/demo.wav".into())])
        );
        assert!(resource_paths(json!({})).unwrap().is_empty());
        for invalid in [
            json!([]),
            Value::Null,
            json!({"x":null}),
            json!({"x":"relative/path"}),
            json!({"x":"/a\u{0}b"}),
        ] {
            assert!(resource_paths(invalid).is_err());
        }
    }
    #[test]
    fn plugin_and_inspector_notifications_go_only_to_opposite_side_and_get_echoes_id() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let (plugin_tx, mut plugin_rx) = mpsc::unbounded_channel();
        let (pi_tx, mut pi_rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(plugin_tx);
        host.inspectors.insert(
            "pi".into(),
            Inspector {
                context: "test.context".into(),
                plugin: 0,
                child: None,
                sender: Some(pi_tx),
            },
        );
        host.plugin_message(0,json!({"event":"setResources","context":"test.context","payload":{"audio":"/first.wav"},"id":"not-an-ack"})).unwrap();
        let event = pi_rx.try_recv().unwrap();
        assert_eq!(event["event"], "didReceiveResources");
        assert_eq!(event["payload"]["resources"], json!({"audio":"/first.wav"}));
        assert!(event.get("id").is_none());
        assert!(plugin_rx.try_recv().is_err());
        host.inspector_message(
            "pi",
            json!({"event":"setResources","context":"pi","payload":{"second":"/second.wav"}}),
        )
        .unwrap();
        let event = plugin_rx.try_recv().unwrap();
        assert_eq!(
            event["payload"]["resources"],
            json!({"second":"/second.wav"})
        );
        assert!(event.get("id").is_none());
        assert!(pi_rx.try_recv().is_err());
        host.plugin_message(
            0,
            json!({"event":"getResources","context":"test.context","id":"plugin-request"}),
        )
        .unwrap();
        assert_eq!(plugin_rx.try_recv().unwrap()["id"], "plugin-request");
        assert!(pi_rx.try_recv().is_err());
        host.inspector_message(
            "pi",
            json!({"event":"getResources","context":"pi","id":"pi-request"}),
        )
        .unwrap();
        assert_eq!(pi_rx.try_recv().unwrap()["id"], "pi-request");
        assert!(plugin_rx.try_recv().is_err());
        assert!(
            host.inspector_message(
                "pi",
                json!({"event":"setResources","context":"somebody-else","payload":{}})
            )
            .is_err()
        );
        assert_eq!(host.slots[0].resources.as_ref().unwrap().len(), 1);
    }
    #[test]
    fn invalid_maps_do_not_partially_replace_saved_resources() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let slot = host.slots[0].clone();
        let saved = host
            .set_resources(&slot, json!({"original":"/not-required-to-exist"}))
            .unwrap();
        assert!(
            host.set_resources(&saved, json!({"valid":"/a","bad":"relative"}))
                .is_err()
        );
        assert_eq!(host.slots[0].resources, saved.resources);
        let event = host.resources_event(&saved, Some(&json!("request-17")));
        assert_eq!(event["id"], "request-17");
        assert_eq!(
            event["payload"]["resources"]["original"],
            "/not-required-to-exist"
        );
        assert_eq!(event["payload"]["coordinates"], json!({"column":0,"row":0}));
        let cleared = host.set_resources(&saved, json!({"original":""})).unwrap();
        assert_eq!(cleared.resources, Some(BTreeMap::new()));
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    #[test]
    fn plugin_resources_export_as_wrapped_files_and_reimport_with_original_basename() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("manifest.json"),
            br#"{"UUID":"test.plugin"}"#,
        )
        .unwrap();
        let source = temp.path().join("my sample.txt");
        fs::write(&source, b"resource body").unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let slot = host.slots[0].clone();
        host.set_resources(&slot, json!({"file":source,"alias":source}))
            .unwrap();
        host.assign("Keypad", 1, "test.action").unwrap();
        let sibling = host
            .slots
            .iter()
            .find(|s| s.controller == "Keypad" && s.index == 1)
            .unwrap()
            .clone();
        host.set_resources(&sibling, json!({"shared":source}))
            .unwrap();
        let output = temp.path().join("resources.streamDeckProfile");
        host.export_profile(&output).unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let page = bundle.page(&bundle.current_page).unwrap();
        let id = page.slots[0].raw["Resources"]["file"].as_str().unwrap();
        assert_eq!(page.slots[0].raw["Resources"]["alias"], id);
        assert_eq!(page.slots[1].raw["Resources"]["shared"], id);
        let (name, bytes) = bundle.resource(id).unwrap();
        assert!(name.ends_with("/my sample.txt"));
        assert_eq!(bytes, b"resource body");
        let package: Value = serde_json::from_slice(
            bundle
                .files()
                .find(|(name, _)| *name == "package.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(package["FormatVersion"], 1);
        assert_eq!(package["RequiredPlugins"], json!(["test.plugin"]));
        fs::remove_file(source).unwrap();
        host.import_profile(&output).unwrap();
        let imported = &host.slots[0];
        let local = &imported.resources.as_ref().unwrap()["file"];
        assert_eq!(Path::new(local).file_name().unwrap(), "my sample.txt");
        assert_eq!(fs::read(local).unwrap(), b"resource body");
        let second = temp.path().join("unchanged.streamDeckProfile");
        host.export_profile(&second).unwrap();
        let second = profile::load_bundle(&second).unwrap();
        let second_id =
            second.page(&second.current_page).unwrap().slots[0].raw["Resources"]["file"]
                .as_str()
                .unwrap();
        assert_eq!(second_id, id);
        assert_eq!(
            bundle
                .files()
                .find(|(n, _)| *n == "Resources/manifest.json")
                .unwrap()
                .1,
            second
                .files()
                .find(|(n, _)| *n == "Resources/manifest.json")
                .unwrap()
                .1
        );
    }
    #[test]
    fn missing_registered_file_fails_export_without_replacing_existing_archive() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let slot = host.slots[0].clone();
        host.set_resources(&slot, json!({"file":temp.path().join("missing.wav")}))
            .unwrap();
        let output = temp.path().join("existing.streamDeckProfile");
        fs::write(&output, b"keep this archive").unwrap();
        assert!(host.export_profile(&output).is_err());
        assert_eq!(fs::read(output).unwrap(), b"keep this archive");
    }
    #[test]
    fn parked_device_resources_update_its_cached_slot_and_report_its_owner() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.slots[0].index = 5;
        host.sync_visible();
        let a = host.slots[0].clone();
        host.add_simulated_device("device-b", Model::Mini).unwrap();
        host.select_device("device-b").unwrap();
        host.assign("Keypad", 0, "test.action").unwrap();
        let b = host.slots[0].context.clone();
        let changed = host
            .set_resources(&a, json!({"file":temp.path().join("resource.dat")}))
            .unwrap();
        assert_eq!(host.device, "device-b");
        assert_eq!(
            host.resources_event(&changed, Some(&json!("req")))["device"],
            "test-device"
        );
        assert_eq!(
            host.library.slot(&a.context).unwrap().resources,
            changed.resources
        );
        host.with_device("test-device", |h| {
            assert_eq!(h.slots[0].resources, changed.resources);
            Ok(())
        })
        .unwrap();
        assert_eq!(host.slots[0].context, b);
        assert_ne!(host.slots[0].resources, changed.resources);
    }
}
