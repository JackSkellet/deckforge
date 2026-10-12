//! Refresh plugin-derived display assets without rewriting authored profile data.
use super::*;
use std::os::unix::fs::MetadataExt;

impl Host {
    /// Repair a committed exchange whose new defaults were not saved before a
    /// process crash. Run before any restored device is rendered.
    pub(super) fn recover_updated_plugin_assets(&mut self) -> Result<()> {
        let candidates: Vec<_> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, plugin)| !plugin.runtime.load_failed)
            .filter_map(|(index, plugin)| {
                self.previous_plugins.get(&plugin.root).map(|previous| {
                    (
                        index,
                        plugin.id.clone(),
                        plugin.root.clone(),
                        previous.backup.clone(),
                    )
                })
            })
            .collect();
        for (index, id, root, backup) in candidates {
            let identity = directory_identity(&root)?;
            if self.plugin_asset_ids.get(&root) == Some(&identity) {
                continue;
            }
            let (previous, old_actions) =
                plugins::parse(backup.path(), self.reference_app.as_deref())?;
            anyhow::ensure!(
                previous.id == id,
                "Asset recovery backup belongs to another plugin"
            );
            let new_actions: Vec<_> = self
                .actions
                .iter()
                .filter(|action| action.plugin == id)
                .cloned()
                .collect();
            self.refresh_updated_plugin_assets(&old_actions, &new_actions);
            // Use the checked pre-refresh identity. A directory replacement
            // during recovery must never mark an unprocessed version as current.
            anyhow::ensure!(
                directory_identity(&self.plugins[index].root)? == identity,
                "Plugin directory changed during asset recovery"
            );
            self.plugin_asset_ids.insert(root, identity);
        }
        Ok(())
    }

    pub(super) fn record_plugin_asset_identity(&mut self, index: usize) -> Result<()> {
        let root = &self
            .plugins
            .get(index)
            .context("Plugin asset owner disappeared")?
            .root;
        self.plugin_asset_ids
            .insert(root.clone(), directory_identity(root)?);
        Ok(())
    }

    /// Call after replacing action/plugin metadata, before rendering or replaying.
    /// Refresh every copy: a later device projection must not restore old caches.
    pub(super) fn refresh_updated_plugin_assets(
        &mut self,
        old_actions: &[Action],
        new_actions: &[Action],
    ) {
        for slot in self
            .library
            .profiles
            .iter_mut()
            .flat_map(|p| &mut p.pages)
            .flat_map(|p| &mut p.slots)
            .chain(self.slots.iter_mut())
            .chain(self.other_devices.values_mut().flat_map(|d| &mut d.slots))
        {
            refresh_slot(slot, old_actions, new_actions);
        }
    }
}

fn directory_identity(root: &Path) -> Result<(u64, u64)> {
    let metadata = fs::symlink_metadata(root).context("Read plugin asset directory identity")?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Plugin asset root must remain a real directory"
    );
    Ok((metadata.dev(), metadata.ino()))
}

fn explicit_image(slot: &Slot, state: usize) -> bool {
    slot.raw["States"]
        .get(state)
        .is_some_and(|s| s.get("Image").is_some())
        || slot
            .appearances
            .get(&(state as u64))
            .is_some_and(|a| a.user_image.is_some() || a.user_pixels.is_some() || a.imported_image)
}

fn default_layout<'a>(action: &'a Action, controller: &str) -> &'a str {
    if controller == "Neo" {
        "$Neo"
    } else {
        action.raw["Encoder"]["layout"].as_str().unwrap_or("$B1")
    }
}

fn refresh_slot(slot: &mut Slot, old_actions: &[Action], new_actions: &[Action]) {
    let old = old_actions.iter().find(|a| a.uuid == slot.uuid);
    let new = new_actions.iter().find(|a| a.uuid == slot.uuid);
    if old.is_some() || new.is_some() {
        // Older saved states do not always retain provenance. A non-default
        // image must be kept conservatively rather than relabelled as derived.
        let unmatched_image = slot
            .states
            .get(slot.state as usize)
            .and_then(|state| state["Image"].as_str())
            .filter(|image| !image.is_empty())
            .is_some_and(|image| {
                old.and_then(|a| a.raw["States"].get(slot.state as usize))
                    .and_then(|state| state["Image"].as_str())
                    != Some(image)
            });
        if let (Some(old), Some(new)) = (old, new) {
            // Assignment copies manifest states. Only a matching old default
            // without authored-image evidence is eligible to follow its update.
            let count = slot.states.as_array().map_or(0, Vec::len);
            for state in 0..count {
                if explicit_image(slot, state) {
                    continue;
                }
                let before = old.raw["States"].get(state).and_then(|s| s.get("Image"));
                let after = new.raw["States"].get(state).and_then(|s| s.get("Image"));
                if before.is_some()
                    && slot.states[state].get("Image") == before
                    && let Some(fields) = slot.states[state].as_object_mut()
                {
                    if let Some(after) = after {
                        fields.insert("Image".into(), after.clone());
                    } else {
                        fields.remove("Image");
                    }
                }
            }
            if matches!(slot.controller.as_str(), "Encoder" | "Neo")
                && slot.layout == default_layout(old, &slot.controller)
            {
                slot.layout = default_layout(new, &slot.controller).into();
            }
        }
        let current = slot.appearances.get(&slot.state);
        let user_pixels = current.and_then(|a| a.user_pixels.clone());
        let reset = current.is_some_and(|a| a.image_reset);
        let authored = current
            .is_some_and(|a| a.user_image.is_some() || (a.imported_image && !reset))
            || (!reset && unmatched_image)
            || (!reset
                && slot.raw["States"]
                    .get(slot.state as usize)
                    .and_then(|s| s["Image"].as_str())
                    .is_some_and(|s| !s.is_empty()));
        // An old SDK frame is not an imported/custom image merely because an
        // older saved record also carries import flags. Prefer known user pixels.
        let old_sdk_frame = slot.image_changed
            || current
                .and_then(|a| a.plugin_pixels.as_ref())
                .zip(slot.background.as_ref())
                .is_some_and(|(sdk, cached)| {
                    sdk.width() == cached.width()
                        && sdk.height() == cached.height()
                        && sdk.as_bytes() == cached.as_bytes()
                });
        slot.background = user_pixels.or_else(|| {
            (authored && !old_sdk_frame)
                .then(|| slot.background.clone())
                .flatten()
        });
        for appearance in slot.appearances.values_mut() {
            appearance.plugin_pixels = None;
            appearance.encoder_pixels = None;
            if let Some(hardware) = &mut appearance.hardware {
                hardware.pixels = None;
            }
        }
        slot.image_changed = false;
        slot.layout_cache = None;
        slot.image_cache.clear();
        slot.display_warnings.clear();
        slot.preview.clear();
        slot.software_icon.clear();
    }
    for child in slot.children.iter_mut().flatten() {
        refresh_slot(child, old_actions, new_actions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn image(color: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb(color)))
    }
    fn actions(host: &Host) -> (Vec<Action>, Vec<Action>) {
        let mut old = host.actions.clone();
        old[0].raw =
            json!({"States":[{"Image":"old","Title":"Keep"}],"Encoder":{"layout":"old.json"}});
        let mut new = old.clone();
        new[0].raw =
            json!({"States":[{"Image":"new","Title":"Changed"}],"Encoder":{"layout":"new.json"}});
        (old, new)
    }
    #[test]
    fn changed_default_and_same_path_bytes_reload_while_authored_fields_stay() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(temp.path());
        let (old, new) = actions(&host);
        image([1, 2, 3]).save(temp.path().join("old.png")).unwrap();
        image([80, 90, 100])
            .save(temp.path().join("new.png"))
            .unwrap();
        host.actions = new.clone();
        host.slots[0].states = old[0].raw["States"].clone();
        host.slots[0].states[0]["FontSize"] = json!(19);
        host.slots[0].background = Some(image([1, 2, 3]));
        host.slots[0].settings = json!({"unknown":{"keep":17}});
        host.slots[0].raw = json!({"future":[1,2]});
        let before = host.slots[0].clone();
        host.sync_visible();
        host.refresh_updated_plugin_assets(&old, &new);
        let slot = host.slots[0].clone();
        assert_eq!(slot.states[0]["Image"], "new");
        assert_eq!(slot.states[0]["FontSize"], 19);
        assert_eq!(slot.raw, before.raw);
        assert_eq!(slot.settings, before.settings);
        assert_eq!(slot.context, before.context);
        assert_eq!(slot.title, before.title);
        assert_eq!(
            host.resolve_state_image(&slot, 0)
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [80, 90, 100]
        );
        host.slots[0].background = Some(image([80, 90, 100]));
        image([8, 9, 10]).save(temp.path().join("new.png")).unwrap();
        host.refresh_updated_plugin_assets(&new, &new);
        let slot = host.slots[0].clone();
        assert_eq!(
            host.resolve_state_image(&slot, 0)
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [8, 9, 10]
        );
    }
    #[test]
    fn authored_images_and_raw_records_survive_while_sdk_frames_expire() {
        let temp = tempfile::tempdir().unwrap();
        let host = crate::host::tests::fixture(temp.path());
        let (old, new) = actions(&host);
        for user in [false, true] {
            let mut slot = host.slots[0].clone();
            slot.states = old[0].raw["States"].clone();
            slot.raw = json!({"States":[{"Image":"old","future":7}],"unknown":true});
            let authored = image([30, 40, 50]);
            slot.background = Some(authored.clone());
            slot.appearances.insert(
                0,
                appearance::StateAppearance {
                    imported_image: !user,
                    user_image: user.then(|| temp.path().join("custom.svg")),
                    user_pixels: user.then(|| authored.clone()),
                    plugin_pixels: Some(image([1, 2, 3])),
                    plugin_title: Some("SDK title retained".into()),
                    hardware: Some(appearance::PluginAppearance {
                        title: Some("Hardware title retained".into()),
                        pixels: Some(image([4, 5, 6])),
                    }),
                    encoder_pixels: Some(image([7, 8, 9])),
                    ..Default::default()
                },
            );
            let before = slot.clone();
            refresh_slot(&mut slot, &old, &new);
            assert_eq!(slot.states, before.states);
            assert_eq!(slot.raw, before.raw);
            assert_eq!(
                slot.background.as_ref().unwrap().as_bytes(),
                authored.as_bytes()
            );
            assert!(slot.appearances[&0].plugin_pixels.is_none());
            assert!(slot.appearances[&0].encoder_pixels.is_none());
            assert!(
                slot.appearances[&0]
                    .hardware
                    .as_ref()
                    .unwrap()
                    .pixels
                    .is_none()
            );
            assert_eq!(
                slot.appearances[&0]
                    .hardware
                    .as_ref()
                    .unwrap()
                    .title
                    .as_deref(),
                Some("Hardware title retained")
            );
            assert_eq!(
                slot.appearances[&0].plugin_title.as_deref(),
                Some("SDK title retained")
            );
            if user {
                assert_eq!(
                    slot.appearances[&0]
                        .user_pixels
                        .as_ref()
                        .unwrap()
                        .as_bytes(),
                    authored.as_bytes()
                );
            }
        }
        let mut legacy = host.slots[0].clone();
        legacy.background = Some(image([1, 2, 3]));
        legacy.appearances.insert(
            0,
            appearance::StateAppearance {
                imported_image: true,
                plugin_pixels: legacy.background.clone(),
                ..Default::default()
            },
        );
        refresh_slot(&mut legacy, &old, &new);
        assert!(legacy.background.is_none());
        let mut unknown_origin = host.slots[0].clone();
        unknown_origin.states = json!([{"Image":"custom-imported-image.png"}]);
        unknown_origin.background = Some(image([31, 41, 51]));
        refresh_slot(&mut unknown_origin, &old, &new);
        assert_eq!(
            unknown_origin.states[0]["Image"],
            "custom-imported-image.png"
        );
        assert_eq!(
            unknown_origin
                .background
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [31, 41, 51]
        );
    }
    #[test]
    fn hidden_nested_and_parked_copies_refresh_without_stale_sync_back() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(temp.path());
        let (old, new) = actions(&host);
        let mut child = host.slots[0].clone();
        child.context = "nested".into();
        child.controller = "Encoder".into();
        child.states = old[0].raw["States"].clone();
        child.layout = "old.json".into();
        child.layout_cache = Some(json!({"old":true}));
        child
            .image_cache
            .insert("icon".into(), ("same.png".into(), image([1, 2, 3])));
        host.slots[0].uuid = "unrelated.container".into();
        host.slots[0].children = vec![vec![child.clone()]];
        host.sync_visible();
        let mut hidden = host.library.profiles[0].clone();
        hidden.id = "hidden-profile".into();
        hidden.pages[0].slots[0].context = "hidden-parent".into();
        host.library.profiles.push(hidden);
        let record = device_pool::Record {
            scope: "parked".into(),
            device: "parked-device".into(),
            name: None,
            model: Model::Plus,
            firmware: "fixture".into(),
            config: None,
            navigation: device_pool::Navigation::default(),
        };
        let mut parked = device_pool::DeviceRuntime::new(record);
        parked.slots = vec![child];
        host.other_devices.insert("parked".into(), parked);
        host.refresh_updated_plugin_assets(&old, &new);
        host.sync_all_devices();
        for slot in host
            .library
            .profiles
            .iter()
            .flat_map(|p| &p.pages)
            .flat_map(|p| &p.slots)
            .chain(&host.slots)
            .chain(host.other_devices.values().flat_map(|d| &d.slots))
        {
            for child in slot
                .flatten()
                .into_iter()
                .filter(|s| s.uuid == "test.action")
            {
                assert_eq!(child.states[0]["Image"], "new");
                assert_eq!(child.layout, "new.json");
                assert!(child.layout_cache.is_none() && child.image_cache.is_empty());
            }
        }
        let mut customized = host.other_devices["parked"].slots[0].clone();
        customized.layout = "sdk-choice.json".into();
        refresh_slot(&mut customized, &new, &old);
        assert_eq!(customized.layout, "sdk-choice.json");
        assert_eq!(customized.states[0]["Image"], "old");
    }

    #[test]
    fn committed_exchange_recovers_before_render_and_marker_preserves_later_sdk_pixels() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let root = state.join("plugins/test.plugin.sdPlugin");
        fs::create_dir_all(&root).unwrap();
        let manifest = |version: &str, name: &str| {
            json!({
                "UUID":"test.plugin", "Name":"Fixture", "Version":version, "CodePath":"plugin.html",
                "Actions":[{"UUID":"test.action","Name":"Fixture", "Controllers":["Keypad","Encoder"],
                    "States":[{"Image":name}], "Encoder":{"layout":format!("{name}.json")}}]
            })
        };
        fs::write(
            root.join("manifest.json"),
            manifest("1.0.0", "old").to_string(),
        )
        .unwrap();
        fs::write(root.join("plugin.html"), "<!doctype html>").unwrap();
        let layout = serde_json::to_vec(&crate::layout::builtin("$B1").unwrap()).unwrap();
        fs::write(root.join("old.json"), &layout).unwrap();
        image([255, 0, 0]).save(root.join("old.png")).unwrap();
        let mut host = crate::host::tests::fixture(&state);
        let (plugin, actions) = plugins::parse(&root, None).unwrap();
        host.plugins[0] = plugin;
        host.actions = actions;
        host.assign("Keypad", 0, "test.action").unwrap();
        host.assign("Encoder", 0, "test.action").unwrap();
        host.assign("Keypad", 1, "test.action").unwrap();
        let custom_context = host
            .slots
            .iter()
            .find(|s| s.controller == "Keypad" && s.index == 1)
            .unwrap()
            .context
            .clone();
        let custom = host
            .slots
            .iter_mut()
            .find(|s| s.context == custom_context)
            .unwrap();
        custom.appearances.insert(
            0,
            appearance::StateAppearance {
                user_image: Some(temporary.path().join("custom.svg")),
                user_pixels: Some(image([0, 255, 0])),
                user_title: Some("User title".into()),
                ..Default::default()
            },
        );
        custom.background = Some(image([0, 255, 0]));
        custom.settings = json!({"future":{"preserved":true}});
        let custom_raw = custom.raw.clone();
        host.sync_visible();
        host.record_plugin_asset_identity(0).unwrap();
        let old_identity = host.plugin_asset_ids[&root];
        host.persist().unwrap();
        let saved_before: Saved =
            serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();

        let package = temporary.path().join("new.streamDeckPlugin");
        let mut archive = zip::ZipWriter::new(File::create(&package).unwrap());
        let mut png = Cursor::new(Vec::new());
        image([0, 0, 255])
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        for (name, bytes) in [
            (
                "manifest.json",
                manifest("2.0.0", "new").to_string().into_bytes(),
            ),
            ("plugin.html", b"<!doctype html>".to_vec()),
            ("new.json", layout),
            ("new.png", png.into_inner()),
        ] {
            archive
                .start_file(
                    format!("test.plugin.sdPlugin/{name}"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive.write_all(&bytes).unwrap();
        }
        archive.finish().unwrap();
        // Commit only the filesystem operation: no host asset refresh/persist.
        crate::plugin_update::Prepared::new(&package, &root, &state, None)
            .unwrap()
            .commit()
            .unwrap();
        let (plugin, actions) = plugins::parse(&root, None).unwrap();
        host.plugins[0] = plugin;
        host.actions = actions;
        host.slots = saved_before.slots;
        host.library = saved_before.library.unwrap();
        host.plugin_asset_ids = saved_before.plugin_asset_ids;
        assert!(
            host.slots
                .iter()
                .any(|s| s.controller == "Encoder" && s.layout == "old.json")
        );
        assert!(display::load_layout(Some(&root), "old.json").is_err());
        host.refresh_previous_plugins();
        host.recover_updated_plugin_assets().unwrap();
        assert_ne!(host.plugin_asset_ids[&root], old_identity);
        assert!(
            host.slots
                .iter()
                .any(|s| s.controller == "Encoder" && s.layout == "new.json")
        );
        host.render().unwrap();
        let key = host
            .slots
            .iter()
            .find(|s| s.controller == "Keypad" && s.index == 0)
            .unwrap();
        assert_eq!(
            key.background.as_ref().unwrap().to_rgb8().get_pixel(0, 0).0,
            [0, 0, 255]
        );
        let custom = host
            .slots
            .iter()
            .find(|s| s.context == custom_context)
            .unwrap();
        assert_eq!(custom.raw, custom_raw);
        assert_eq!(custom.title, "User title");
        assert_eq!(custom.settings, json!({"future":{"preserved":true}}));
        assert_eq!(
            custom
                .background
                .as_ref()
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [0, 255, 0]
        );
        host.persist().unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.plugin_asset_ids[&root], host.plugin_asset_ids[&root]);

        let current = host
            .slots
            .iter_mut()
            .find(|s| s.controller == "Keypad" && s.index == 0)
            .unwrap();
        current.background = Some(image([91, 92, 93]));
        current.appearances.entry(0).or_default().plugin_pixels = current.background.clone();
        host.sync_visible();
        host.recover_updated_plugin_assets().unwrap();
        let current = host
            .slots
            .iter()
            .find(|s| s.controller == "Keypad" && s.index == 0)
            .unwrap();
        assert_eq!(
            current
                .background
                .as_ref()
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [91, 92, 93]
        );
        assert!(current.appearances[&0].plugin_pixels.is_some());
    }
}
