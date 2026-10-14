//! Native action sharing is a profile bundle on the $storage / Fake/Storage
//! device. Stage it through the same asset exporter and transactional paste path.
use super::*;
use std::collections::BTreeSet;

pub(super) fn selection(request: &Value) -> Result<Option<usize>> {
    request
        .get("root")
        .map(|v| usize::try_from(v.as_u64().context("Invalid root selection")?).map_err(Into::into))
        .transpose()
}
fn folder_references(raw: &Value, depth: usize, output: &mut HashSet<String>) -> Result<()> {
    ensure!(depth <= 32, "Shared action nesting exceeds the limit");
    let uuid = raw["UUID"].as_str().unwrap_or("");
    if uuid == library::OPEN_FOLDER
        && let Some(id) = raw["Settings"]["ProfileUUID"].as_str()
    {
        output.insert(profile::canonical_page_id(id));
    }
    if uuid.is_empty()
        || multiaction::container(uuid)
        || matches!(
            uuid,
            crate::dial_stack::UUID
                | crate::action_trigger::UUID
                | crate::key_logic::UUID
                | crate::action_wheel::UUID
        )
    {
        for child in raw["Actions"].as_array().into_iter().flatten() {
            folder_references(child, depth + 1, output)?;
        }
    }
    Ok(())
}
fn retain_selected_tree(
    bundle: &mut profile::ProfileBundle,
    page: usize,
    slot: usize,
) -> Result<()> {
    let mut selected = bundle.pages[page].clone();
    selected.slots = vec![selected.slots[slot].clone()];
    let roots: HashSet<_> = bundle
        .page_order
        .iter()
        .chain([&bundle.default_page, &bundle.current_page])
        .cloned()
        .collect();
    let mut pending = HashSet::new();
    folder_references(&selected.slots[0].raw, 0, &mut pending)?;
    let mut keep = HashSet::new();
    while let Some(id) = pending.iter().next().cloned() {
        pending.remove(&id);
        if !keep.insert(id.clone()) {
            continue;
        }
        ensure!(!roots.contains(&id), "Shared folder points to a root page");
        let page = bundle.page(&id).context("Shared folder page is missing")?;
        for slot in &page.slots {
            folder_references(&slot.raw, 0, &mut pending)?;
        }
    }
    let mut pages = vec![selected];
    pages.extend(
        bundle
            .pages
            .iter()
            .filter(|p| keep.contains(&p.page))
            .cloned(),
    );
    bundle.current_page = pages[0].page.clone();
    bundle.default_page.clear();
    bundle.page_order = if bundle.current_page.is_empty() {
        vec![]
    } else {
        vec![bundle.current_page.clone()]
    };
    bundle.pages = pages;
    Ok(())
}

impl Host {
    pub(super) fn export_shared_action(&mut self, request: &Value) -> Result<()> {
        let context = request["context"]
            .as_str()
            .context("Missing action context")?;
        let path = Path::new(request["path"].as_str().context("Missing export path")?);
        if let Ok(destination) = path.canonicalize() {
            ensure!(
                !self
                    .retained_asset_sources()
                    .iter()
                    .filter_map(|p| p.canonicalize().ok())
                    .any(|p| p == destination),
                "Choose an export destination outside the managed profile sources"
            );
        }
        let copy = capture(&self.library, context, false)?;
        let mut slot = copy.slot;
        let model = slot.model;
        let controller = slot.controller.clone();
        // Neo's managed infobar address is 1,0; the storage device uses 0,0.
        position(
            &mut slot,
            &controller,
            if controller == "Neo" { 1 } else { 0 },
        );
        let root_id = library::new_id();
        let pin_id = library::new_id();
        let mut pages = vec![
            library::Page {
                id: root_id.clone(),
                name: "Shared action".into(),
                slots: vec![slot],
                raw: json!({"Controllers":[]}),
                source_manifest: None,
                asset_source: None,
                background_source: None,
                manifest_path: library::page_path(&root_id),
            },
            library::Page {
                id: pin_id.clone(),
                name: String::new(),
                slots: vec![],
                raw: json!({"Controllers":[]}),
                source_manifest: None,
                asset_source: None,
                background_source: None,
                manifest_path: library::page_path(&pin_id),
            },
        ];
        pages.extend(copy.pages);
        let profile = library::Profile {
            model,
            id: library::new_id(),
            name: "Shared action".into(),
            device: "$storage".into(),
            pin_page: pin_id,
            page_order: vec![root_id.clone()],
            pages,
            raw: json!({"Device":{"UUID":"$storage","Model":"Fake/Storage"},"Version":"3.0"}),
            source: None,
            owner_plugin: None,
            readonly: false,
        };
        let roots: HashMap<_, _> = self
            .actions
            .iter()
            .filter_map(|a| {
                self.plugins
                    .iter()
                    .find(|p| p.id == a.plugin)
                    .map(|p| (a.uuid.clone(), p.root.clone()))
            })
            .collect();
        let mut files = export::build_profile_files(&profile, &root_id, &roots)?;
        let mut required = BTreeSet::new();
        for slot in profile
            .pages
            .iter()
            .flat_map(|p| &p.slots)
            .flat_map(Slot::flatten)
        {
            let manifest = roots
                .get(&slot.uuid)
                .and_then(|root| fs::read(root.join("manifest.json")).ok())
                .and_then(|v| serde_json::from_slice::<Value>(&v).ok());
            if let Some(id) = slot.raw["Plugin"]["UUID"]
                .as_str()
                .or_else(|| manifest.as_ref().and_then(|v| v["UUID"].as_str()))
            {
                required.insert(id.to_owned());
            }
        }
        let mut package: Value = serde_json::from_slice(&files["package.json"])?;
        if let Some(ids) = package["RequiredPlugins"].as_array() {
            required.extend(ids.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        package["RequiredPlugins"] = json!(required);
        package["AppVersion"] = json!(env!("CARGO_PKG_VERSION"));
        package["OSType"] = json!("Linux");
        package["OSVersion"] = json!(
            fs::read_to_string("/proc/sys/kernel/osrelease")
                .unwrap_or_default()
                .trim()
        );
        package["DeviceSettings"] = Value::Null;
        files.insert("package.json".into(), serde_json::to_vec_pretty(&package)?);
        if controller == "Neo" {
            let manifest = format!(
                "Profiles/{}.sdProfile/{}",
                profile.id,
                library::page_path(&root_id)
            );
            let mut page: Value = serde_json::from_slice(&files[&manifest])?;
            let actions = page["Controllers"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|c| c["Type"] == "Neo")
                .unwrap()["Actions"]
                .as_object_mut()
                .unwrap();
            let action = actions.remove("1,0").context("Shared infobar missing")?;
            actions.insert("0,0".into(), action);
            files.insert(manifest, serde_json::to_vec_pretty(&page)?);
        }
        let backup = export::write_archive(path, &files)?;
        self.log(format!(
            "Exported action to {}{}",
            path.display(),
            backup
                .map(|p| format!("; prior file backed up to {}", p.display()))
                .unwrap_or_default()
        ));
        Ok(())
    }

    pub(super) fn import_shared_action(&mut self, request: &Value) -> Result<()> {
        ensure!(
            !self.library.current()?.readonly,
            "Cannot edit a read-only profile"
        );
        ensure!(
            request["expected_profile"].as_str() == Some(&self.library.active_profile)
                && request["expected_page"].as_str() == Some(&self.library.active_page),
            "Destination page changed; select the destination again"
        );
        let path = Path::new(request["path"].as_str().context("Missing action file")?);
        let (copy, source) = self.prepare_shared_action(
            path,
            request["controller"].as_str(),
            selection(request)?,
            request["source_hash"].as_str(),
        )?;
        let result = self.paste_copy(request, copy);
        if !self.retained_asset_sources().contains(&source.as_path()) {
            let _ = fs::remove_file(source);
        }
        result
    }

    pub(in crate::host) fn load_action_clipboard(&mut self, path: &Path) -> Result<()> {
        self.load_selected_action_clipboard(path, None, None)
    }
    pub(in crate::host) fn load_selected_action_clipboard(
        &mut self,
        path: &Path,
        root: Option<usize>,
        source_hash: Option<&str>,
    ) -> Result<()> {
        let (mut copy, source) = self.prepare_shared_action(path, None, root, source_hash)?;
        copy.temporary_source = Some(source);
        self.replace_clipboard(Some(copy));
        Ok(())
    }

    fn prepare_shared_action(
        &mut self,
        path: &Path,
        target: Option<&str>,
        selected: Option<usize>,
        expected_hash: Option<&str>,
    ) -> Result<(Clipboard, PathBuf)> {
        let (mut bundle, roots) = crate::shared_action::load(path)?;
        ensure!(
            selected.is_some() || roots.len() == 1,
            "Choose one root action from this file (found {})",
            roots.len()
        );
        ensure!(
            roots.len() == 1 || expected_hash.is_some(),
            "Inspect the shared file before choosing an action"
        );
        if let Some(expected) = expected_hash {
            ensure!(
                crate::shared_action::digest(&bundle) == expected,
                "Shared action file changed; open it again"
            );
        }
        let chosen = roots
            .get(selected.unwrap_or(0))
            .context("Selected root action is missing")?;
        let (root_page, root_slot) = (chosen.page_index, chosen.slot_index);
        let root_manifest = bundle.pages[root_page].manifest_path();
        retain_selected_tree(&mut bundle, root_page, root_slot)?;
        let (root_page, root_slot) = (0, 0);
        let source_controller = bundle.pages[root_page].slots[root_slot].controller.clone();
        let item = &mut bundle.pages[root_page].slots[root_slot];
        let target_controller = target.unwrap_or(&source_controller);
        ensure!(
            self.actions.iter().find(|a| a.uuid == item.uuid).map_or(
                source_controller == target_controller,
                |a| a.controllers.iter().any(|c| c == target_controller)
            ),
            "Action does not support the target controller"
        );
        item.controller = target_controller.into();
        item.column = if target_controller == "Neo" { 1 } else { 0 };
        item.row = 0;
        // Fake/Storage has no physical geometry. Preserve every folder coordinate;
        // fail rather than silently drop cells the destination cannot represent.
        let mut model = self.library.current()?.model;
        if target.is_none()
            && library::position_index(model, target_controller, item.column, 0).is_none()
        {
            model = if target_controller == "Neo" {
                Model::Neo
            } else {
                Model::Plus
            };
        }
        for page in &bundle.pages {
            for item in &page.slots {
                ensure!(
                    library::position_index(model, &item.controller, item.column, item.row)
                        .is_some(),
                    "Shared action or folder contains a {} control at {},{} outside this device layout",
                    item.controller,
                    item.column,
                    item.row
                );
            }
        }
        bundle.raw["Device"]["Model"] = json!(model.profile_model());
        let (mut profile, _) = self.prepare_bundle(bundle, None, false)?;
        let source = profile
            .source
            .clone()
            .context("Imported action source missing")?;
        let result = (|| {
            for slot in profile.pages.iter_mut().flat_map(|p| &mut p.slots) {
                slot.hydrate_children(&self.actions)?;
            }
            let context = profile
                .pages
                .iter()
                .find(|p| p.source_manifest.as_deref() == Some(root_manifest.as_str()))
                .and_then(|p| p.slots.first())
                .context("Shared root action missing")?
                .context
                .clone();
            if target.is_none() {
                profile.device.clear();
            }
            let staged = Library {
                profiles: vec![profile],
                ..Default::default()
            };
            capture(&staged, &context, false)
        })();
        match result {
            Ok(copy) => Ok((copy, source)),
            Err(error) => {
                let _ = fs::remove_file(source);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Host {
        super::super::super::tests::fixture(root)
    }
    fn import(host: &mut Host, path: &Path, index: u8) -> Result<()> {
        host.transfer_command(&json!({"command":"slot_import","path":path,
            "controller":"Keypad","index":index,"expected_target":null,
            "expected_profile":host.library.active_profile,"expected_page":host.library.active_page}))
    }
    fn export(host: &mut Host, context: &str, path: &Path) {
        host.transfer_command(&json!({"command":"slot_export","context":context,"path":path}))
            .unwrap();
    }
    #[test]
    fn file_clipboard_accepts_encoder_on_readonly_mini_without_changing_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.actions[0].controllers.push("Encoder".into());
        host.slots[0].controller = "Encoder".into();
        let context = host.slots[0].context.clone();
        let path = dir.path().join("dial.streamDeckAction");
        export(&mut host, &context, &path);
        host.add_simulated_device("Mini", Model::Mini).unwrap();
        host.with_device("Mini", |host| {
            host.library.current_mut()?.readonly = true;
            let before = serde_json::to_value(&host.library)?;
            host.load_action_clipboard(&path)?;
            assert_eq!(serde_json::to_value(&host.library)?, before);
            assert_eq!(host.clipboard.as_ref().unwrap().slot.controller, "Encoder");
            assert_eq!(host.clipboard.as_ref().unwrap().slot.model, Model::Plus);
            let previous = host
                .clipboard
                .as_ref()
                .unwrap()
                .temporary_source
                .clone()
                .unwrap();
            assert!(previous.exists());
            host.load_action_clipboard(&path)?;
            assert!(
                !previous.exists(),
                "Replaced unreferenced clipboard archive must be removed"
            );
            let clipboard = host.clipboard_snapshot();
            let bad = dir.path().join("bad.streamDeckAction");
            fs::write(&bad, b"not a zip")?;
            assert!(host.load_action_clipboard(&bad).is_err());
            assert_eq!(host.clipboard_snapshot(), clipboard);
            Ok(())
        })
        .unwrap();
        fs::remove_file(path).unwrap();
        host.transfer_command(&json!({"command":"slot_paste","controller":"Encoder","index":1,"expected_target":null})).unwrap();
        assert!(
            host.slots
                .iter()
                .any(|s| s.controller == "Encoder" && s.index == 1)
        );
        let source = host
            .clipboard
            .as_ref()
            .unwrap()
            .temporary_source
            .clone()
            .unwrap();
        host.transfer_command(&json!({"command":"slot_clipboard_clear"}))
            .unwrap();
        assert!(source.exists(), "Pasted media still owns its archive");
    }
    #[test]
    fn shared_action_roundtrip_keeps_assets_unknown_fields_and_clipboard() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        fs::write(
            dir.path().join("manifest.json"),
            br#"{"UUID":"test.plugin"}"#,
        )
        .unwrap();
        let mut pixels = std::io::Cursor::new(Vec::new());
        DynamicImage::new_rgba8(200, 100)
            .write_to(&mut pixels, image::ImageFormat::Png)
            .unwrap();
        let image = base64::engine::general_purpose::STANDARD.encode(pixels.into_inner());
        let original = host.slots[0].context.clone();
        host.slots[0].controller = "Encoder".into();
        host.actions[0].controllers.push("Encoder".into());
        host.slots[0].settings = json!({"nested":{"value":42}});
        host.slots[0].raw = json!({"UUID":"test.action","VendorExtra":{"keep":true},
            "Encoder":{"background":"Images/owned.png"}});
        host.slots[0]
            .encoder_overrides
            .insert("background".into(), image);
        host.transfer_command(&json!({"command":"slot_copy","context":original}))
            .unwrap();
        let clipboard = host.clipboard_snapshot();
        let file = dir.path().join("one.streamDeckAction");
        export(&mut host, &original, &file);
        let bundle = profile::load_bundle(&file).unwrap();
        assert_eq!(
            bundle.raw["Device"],
            json!({"UUID":"$storage","Model":"Fake/Storage"})
        );
        let package: Value = serde_json::from_slice(
            bundle
                .files()
                .find(|(n, _)| *n == "package.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(package["RequiredPlugins"], json!(["test.plugin"]));
        assert!(bundle.manifest_path().starts_with("Profiles/"));
        let count = host.library.profiles.len();
        import(&mut host, &file, 1).unwrap();
        assert_eq!(host.library.profiles.len(), count);
        assert_eq!(host.clipboard_snapshot(), clipboard);
        fs::remove_file(&file).unwrap();
        let imported = host.slots.iter().find(|s| s.index == 1).unwrap();
        assert_ne!(imported.context, original);
        assert_eq!(imported.settings, json!({"nested":{"value":42}}));
        assert_eq!(imported.raw["VendorExtra"], json!({"keep":true}));
        let origin = imported.asset_source.as_ref().unwrap();
        assert!(origin.source.is_file());
        let source = profile::load_bundle(&origin.source).unwrap();
        let page = source
            .pages
            .iter()
            .find(|p| p.manifest_path() == origin.manifest)
            .unwrap();
        let bytes = page
            .asset(imported.raw["Encoder"]["background"].as_str().unwrap())
            .unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        let context = imported.context.clone();
        export(
            &mut host,
            &context,
            &dir.path().join("again.streamDeckAction"),
        );
    }
    #[test]
    fn shared_folder_remaps_nested_actions_without_importing_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.library_command(&json!({"command":"folder_create","name":"Folder","index":1}))
            .unwrap();
        let folder = host.slots.iter().find(|s| s.index == 1).unwrap();
        let context = folder.context.clone();
        let page_id = folder.settings["ProfileUUID"].as_str().unwrap().to_owned();
        let mut nested = host.slots[0].clone();
        nested.context = library::new_id();
        nested.index = 3;
        nested.raw = json!({"UUID":"test.action","FutureMetadata":7});
        host.library
            .current_mut()
            .unwrap()
            .pages
            .iter_mut()
            .find(|p| p.id == page_id)
            .unwrap()
            .slots
            .push(nested);
        let path = dir.path().join("folder.streamDeckAction");
        export(&mut host, &context, &path);
        let pages = host.library.current().unwrap().pages.len();
        import(&mut host, &path, 2).unwrap();
        assert_eq!(host.library.current().unwrap().pages.len(), pages + 1);
        let copied = host.slots.iter().find(|s| s.index == 2).unwrap();
        assert_ne!(copied.settings["ProfileUUID"], page_id);
        let page = host
            .library
            .current()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.id == copied.settings["ProfileUUID"].as_str().unwrap())
            .unwrap();
        assert!(
            page.slots
                .iter()
                .any(|s| s.raw["FutureMetadata"] == 7 && s.index == 3)
        );
        assert_eq!(host.library.profiles.len(), 1);
        host.library.validate().unwrap();
    }
    #[test]
    fn shared_multi_action_keeps_nested_resource_bytes_after_source_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.actions.extend(multiaction::definitions());
        host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
        let context = host.slots[0].context.clone();
        host.save_multi(
            &context,
            json!({"Actions":[{"Actions":[{
            "UUID":"test.action","ActionID":"child","Settings":{"keep":42},
            "State":0,"States":[{}],"Extra":"nested"}]}]}),
        )
        .unwrap();
        let resource = dir.path().join("sound.txt");
        fs::write(&resource, b"nested resource body").unwrap();
        let child = host.slots[0].children[0][0].clone();
        host.set_resources(&child, json!({"file":resource}))
            .unwrap();
        let path = dir.path().join("multi.streamDeckAction");
        export(&mut host, &context, &path);
        fs::remove_file(resource).unwrap();
        import(&mut host, &path, 1).unwrap();
        fs::remove_file(path).unwrap();
        let parent = host.slots.iter().find(|s| s.index == 1).unwrap();
        let copied = &parent.children[0][0];
        assert_ne!(copied.context, child.context);
        assert_eq!(copied.settings, json!({"keep":42}));
        assert_eq!(copied.raw["Extra"], "nested");
        let resource = &copied.resources.as_ref().unwrap()["file"];
        assert_eq!(fs::read(resource).unwrap(), b"nested resource body");
        let context = parent.context.clone();
        export(
            &mut host,
            &context,
            &dir.path().join("second.streamDeckAction"),
        );
    }
    #[test]
    fn shared_infobar_uses_storage_coordinate_then_restores_neo_address() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        host.actions[0].controllers.push("Neo".into());
        host.add_simulated_device("neo", Model::Neo).unwrap();
        host.with_device("neo", |host| {
            host.assign("Neo", 1, "test.action")?;
            let context = host.slots[0].context.clone();
            let path = dir.path().join("neo.streamDeckAction");
            export(host, &context, &path);
            let bundle = profile::load_bundle(&path)?;
            let root = bundle.page(&bundle.current_page).unwrap();
            assert_eq!((root.slots[0].column,root.slots[0].row),(0,0));
            host.transfer_command(&json!({"command":"slot_import","path":path,
                "controller":"Neo","index":1,"expected_target":context,
                "expected_profile":host.library.active_profile,"expected_page":host.library.active_page}))?;
            assert_eq!(host.slots[0].index,1);
            assert_eq!(host.slots[0].controller,"Neo");
            assert_ne!(host.slots[0].context,context);
            Ok(())
        }).unwrap();
    }
    #[test]
    fn shared_import_rejects_multiple_roots_instead_of_discarding_actions() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        let context = host.slots[0].context.clone();
        let path = dir.path().join("multiple.streamDeckAction");
        export(&mut host, &context, &path);
        let bundle = profile::load_bundle(&path).unwrap();
        let manifest = bundle.page(&bundle.current_page).unwrap().manifest_path();
        let mut files: std::collections::BTreeMap<_, _> = bundle
            .files()
            .map(|(n, b)| (n.to_owned(), b.to_vec()))
            .collect();
        let mut page: Value = serde_json::from_slice(&files[&manifest]).unwrap();
        page["Controllers"][0]["Actions"]["1,0"] = page["Controllers"][0]["Actions"]["0,0"].clone();
        files.insert(manifest, serde_json::to_vec(&page).unwrap());
        export::write_archive(&path, &files).unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        let error = import(&mut host, &path, 1).unwrap_err();
        assert!(error.to_string().contains("one root action"));
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert!(!dir.path().join("sources").exists());
    }
    #[test]
    fn shared_action_failed_paste_preserves_state_and_cleans_staged_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = fixture(dir.path());
        let context = host.slots[0].context.clone();
        let path = dir.path().join("one.streamDeckAction");
        export(&mut host, &context, &path);
        let before = serde_json::to_value(&host.library).unwrap();
        assert!(import(&mut host, &path, 0).is_err()); // occupied, expected empty
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path().join("sources")).unwrap().count(), 0);
        assert!(
            host.transfer_command(&json!({"command":"slot_import","path":path,
            "controller":"Keypad","index":1,"expected_target":null,
            "expected_profile":"stale","expected_page":host.library.active_page}))
                .is_err()
        );
        // Persist cannot write a session over a directory; no action is committed.
        host.state_path = dir.path().join("unwritable");
        fs::create_dir(&host.state_path).unwrap();
        assert!(import(&mut host, &path, 1).is_err());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path().join("sources")).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod choice_tests {
    use super::*;
    use crate::desktop_action_file::{Destination, Manager};
    fn fixture(root: &Path) -> Host {
        super::super::super::tests::fixture(root)
    }
    fn bundle(host: &mut Host, path: &Path, folder: bool) {
        let context = host.slots[0].context.clone();
        host.export_shared_action(&json!({"context":context,"path":path}))
            .unwrap();
        let loaded = profile::load_bundle(path).unwrap();
        let page = loaded.page(&loaded.current_page).unwrap().manifest_path();
        let mut files: std::collections::BTreeMap<String, Vec<u8>> = loaded
            .files()
            .map(|(p, b)| (p.into(), b.to_vec()))
            .collect();
        let mut raw: Value = serde_json::from_slice(&files[&page]).unwrap();
        raw["Controllers"][0]["Actions"]["20,0"] = json!({"UUID":"test.action","ActionID":"second","Name":"Second root","Settings":{"chosen":2},"States":[{}]});
        if folder {
            let id = uuid::Uuid::new_v4().to_string();
            raw["Controllers"][0]["Actions"]["0,0"] = json!({"UUID":library::OPEN_FOLDER,"ActionID":"folder","Name":"Wide folder","Settings":{"ProfileUUID":id},"States":[{}]});
            let base = loaded
                .manifest_path()
                .trim_end_matches("manifest.json")
                .to_owned();
            files.insert(format!("{base}Profiles/{id}/manifest.json"),serde_json::to_vec(&json!({"Controllers":[{"Type":"Keypad","Actions":{"7,3":{"UUID":"test.action","Settings":{"folder":true},"States":[{}]}}}]})).unwrap());
        }
        files.insert(page, serde_json::to_vec(&raw).unwrap());
        export::write_archive(path, &files).unwrap();
    }
    #[test]
    fn inspect_selects_only_requested_root_and_ignores_unselected_folder_geometry() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let path = temp.path().join("multiple.streamDeckAction");
        bundle(&mut host, &path, true);
        let before = serde_json::to_value(&host.library).unwrap();
        let info = crate::shared_action::inspect(&path).unwrap();
        assert_eq!(info.roots.len(), 2);
        assert_eq!(info.roots[1].name, "Second root");
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        let request = json!({"command":"slot_import","path":path,"root":1,"source_hash":info.source_hash,"controller":"Keypad","index":1,"expected_target":null,"expected_profile":host.library.active_profile,"expected_page":host.library.active_page});
        host.import_shared_action(&request).unwrap();
        assert_eq!(
            host.slots.iter().find(|s| s.index == 1).unwrap().settings["chosen"],
            2
        );
        assert_eq!(host.library.current().unwrap().pages.len(), 2);
        let mut bad = request;
        bad["root"] = json!(0);
        bad["index"] = json!(2);
        let graph = serde_json::to_value(&host.library).unwrap();
        assert!(
            host.import_shared_action(&bad)
                .unwrap_err()
                .to_string()
                .contains("outside this device layout")
        );
        assert_eq!(serde_json::to_value(&host.library).unwrap(), graph);
    }
    #[test]
    fn pending_choice_is_inert_exclusive_cancelable_and_checks_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let path = temp.path().join("multiple.streamDeckAction");
        bundle(&mut host, &path, false);
        let device = host.device.clone();
        let graph = serde_json::to_value(&host.library).unwrap();
        let controller = Controller {
            shared: Arc::new(Mutex::new(host)),
        };
        let mut manager = Manager::default();
        manager
            .prepare(&controller, &path, device.clone(), None)
            .unwrap();
        let pending = manager.snapshot();
        let id = pending["id"].as_str().unwrap();
        assert!(
            manager
                .prepare(&controller, &path, device.clone(), None)
                .is_err()
        );
        assert!(manager.cancel("stale").is_err());
        assert_eq!(manager.snapshot(), pending);
        assert_eq!(
            serde_json::to_value(&controller.shared.lock().unwrap().library).unwrap(),
            graph
        );
        let old = profile::load_bundle(&path).unwrap();
        let mut files: std::collections::BTreeMap<String, Vec<u8>> =
            old.files().map(|(p, b)| (p.into(), b.to_vec())).collect();
        files.insert("changed.txt".into(), b"different contents".to_vec());
        export::write_archive(&path, &files).unwrap();
        assert!(
            manager
                .choose(&controller, id, 1)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        assert_eq!(manager.snapshot(), pending);
        assert!(controller.shared.lock().unwrap().clipboard.is_none());
        manager.cancel(id).unwrap();
        manager.prepare(&controller, &path, device, None).unwrap();
        let id = manager.snapshot()["id"].as_str().unwrap().to_owned();
        manager.choose(&controller, &id, 1).unwrap();
        assert!(manager.snapshot().is_null());
        let host = controller.shared.lock().unwrap();
        assert_eq!(serde_json::to_value(&host.library).unwrap(), graph);
        assert_eq!(host.clipboard.as_ref().unwrap().slot.settings["chosen"], 2);
    }
    #[test]
    fn changed_destination_rejects_without_lifecycle_and_failed_source_is_cleaned() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let path = temp.path().join("multiple.streamDeckAction");
        bundle(&mut host, &path, false);
        let destination = Destination {
            controller: "Keypad".into(),
            index: 1,
            expected_target: None,
            expected_profile: host.library.active_profile.clone(),
            expected_page: host.library.active_page.clone(),
        };
        let device = host.device.clone();
        let controller = Controller {
            shared: Arc::new(Mutex::new(host)),
        };
        let mut manager = Manager::default();
        manager
            .prepare(&controller, &path, device, Some(destination))
            .unwrap();
        let id = manager.snapshot()["id"].as_str().unwrap().to_owned();
        let mut host = controller.shared.lock().unwrap();
        host.assign("Keypad", 1, "test.action").unwrap();
        host.input(InputEvent::Key {
            index: 1,
            pressed: true,
        })
        .unwrap();
        host.sync_all_devices();
        let before = serde_json::to_value(&host.library).unwrap();
        let held = host.held.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        drop(host);
        assert!(manager.choose(&controller, &id, 1).is_err());
        let host = controller.shared.lock().unwrap();
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert_eq!(host.held, held);
        assert!(rx.try_recv().is_err());
        assert_eq!(
            fs::read_dir(temp.path().join("sources")).unwrap().count(),
            0
        );
    }
}
