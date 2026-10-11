//! Profile ZIP export preserves source bytes and unknown fields. It does not
//! translate platform-specific actions or claim acceptance by the Windows app.
use super::*;
use anyhow::ensure;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;

impl Host {
    pub(super) fn export_profile(&mut self, path: &Path) -> Result<()> {
        if let Ok(destination) = path.canonicalize() {
            ensure!(
                !self
                    .library
                    .profiles
                    .iter()
                    .filter_map(|p| p.source.as_ref())
                    .filter_map(|source| source.canonicalize().ok())
                    .any(|source| source == destination),
                "Choose an export destination outside the managed profile sources"
            );
        }
        self.sync_visible();
        let profile = self
            .library
            .profiles
            .iter()
            .find(|p| p.id == self.library.active_profile)
            .context("No active profile to export")?;
        let roots: HashMap<String, PathBuf> = self
            .actions
            .iter()
            .filter_map(|a| {
                self.plugins
                    .iter()
                    .find(|p| p.id == a.plugin)
                    .map(|p| (a.uuid.clone(), p.root.clone()))
            })
            .collect();
        let files = build_profile_files(profile, &self.library.active_page, &roots)?;
        let backup = write_archive(path, &files)?;
        self.log(format!(
            "Exported profile to {}{}",
            path.display(),
            backup
                .map(|p| format!("; prior file backed up to {}", p.display()))
                .unwrap_or_default()
        ));
        Ok(())
    }
}

/// Returns complete safe ZIP entries. Source files are copied byte-for-byte;
/// only edited manifests and explicitly changed images are regenerated.
pub(super) fn build_profile_files(
    profile: &library::Profile,
    active_page: &str,
    plugin_roots: &HashMap<String, PathBuf>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    safe_export_path(&profile.id)?;
    ensure!(
        !profile.id.contains('/'),
        "Profile ID must be one path component"
    );
    ensure!(!profile.pages.is_empty(), "Cannot export an empty profile");
    let root = format!("{}.sdProfile/", profile.id);
    let source = profile
        .source
        .as_ref()
        .map(|path| profile::load_bundle(path))
        .transpose()?;
    let mut files = BTreeMap::new();
    if let Some(source) = &source {
        let manifest = source.manifest_path();
        let source_root = manifest.strip_suffix("manifest.json").unwrap();
        let old_manifests: HashSet<_> = source.pages.iter().map(|p| p.manifest_path()).collect();
        for (name, data) in source.files() {
            if name == manifest || old_manifests.contains(name) {
                continue;
            }
            let target = name
                .strip_prefix(source_root)
                .map(|p| format!("{root}{p}"))
                .unwrap_or_else(|| name.to_owned());
            insert_unique(&mut files, target, data.to_vec())?;
        }
    }
    let mut ids = HashSet::new();
    let mut manifests = HashSet::new();
    for page in &profile.pages {
        ensure!(
            ids.insert(page.id.as_str()),
            "Duplicate export page ID {}",
            page.id
        );
        safe_export_path(&page.manifest_path)?;
        ensure!(
            page.manifest_path.starts_with("Profiles/")
                && page.manifest_path.ends_with("/manifest.json")
                && page.manifest_path.matches('/').count() == 2,
            "Invalid page manifest path {}",
            page.manifest_path
        );
        ensure!(
            manifests.insert(page.manifest_path.as_str()),
            "Duplicate export page path {}",
            page.manifest_path
        );
        let page_root = format!(
            "{root}{}",
            page.manifest_path.trim_end_matches("manifest.json")
        );
        if let (Some(source), Some(old_manifest)) = (&source, &page.source_manifest) {
            let old_root = old_manifest
                .strip_suffix("manifest.json")
                .context("Invalid source page manifest")?;
            // Rebase duplicated page assets, retaining every original blob too.
            for (name, data) in source.files() {
                let Some(relative) = name.strip_prefix(old_root) else {
                    continue;
                };
                if relative == "manifest.json" {
                    continue;
                }
                let target = format!("{page_root}{relative}");
                if let Some(existing) = files.get(&target) {
                    ensure!(
                        existing.as_slice() == data,
                        "Conflicting duplicated profile asset {target}"
                    );
                } else {
                    insert_unique(&mut files, target, data.to_vec())?;
                }
            }
        }
        let mut raw = page.raw.clone();
        if !raw.is_object() {
            raw = json!({});
        }
        if raw.get("Name").is_some() || !page.name.is_empty() {
            raw["Name"] = json!(page.name);
        }
        if raw.get("Controllers").is_none() && raw["Actions"].is_object() {
            if page.slots.iter().any(|s| s.controller == "Encoder") {
                let actions = raw.as_object_mut().unwrap().remove("Actions").unwrap();
                raw["Controllers"] = json!([{"Type":"Keypad","Actions":actions}]);
                insert_controller_slots(&mut raw, page, &page_root, &mut files, plugin_roots)?;
            } else {
                // Retain the legacy page-level Actions shape when no encoder exists.
                clear_managed_positions(&mut raw["Actions"], "Keypad");
                let mut occupied = HashSet::new();
                for slot in &page.slots {
                    ensure!(
                        slot.controller == "Keypad",
                        "Invalid managed legacy controller"
                    );
                    let position = managed_position(slot)?;
                    ensure!(
                        occupied.insert(position.clone()),
                        "Duplicate managed legacy position {position}"
                    );
                    raw["Actions"][position] =
                        export_action(slot, &page_root, &mut files, plugin_roots)?;
                }
            }
        } else {
            insert_controller_slots(&mut raw, page, &page_root, &mut files, plugin_roots)?;
        }
        files.insert(
            format!("{root}{}", page.manifest_path),
            serde_json::to_vec_pretty(&raw)?,
        );
    }
    ensure!(ids.contains(active_page), "Active export page is missing");
    ensure!(
        ids.contains(profile.pin_page.as_str()),
        "Default export page is missing"
    );
    ensure!(
        profile.page_order.iter().all(|p| ids.contains(p.as_str())),
        "Page order references a missing page"
    );
    let mut raw = profile.raw.clone();
    if !raw.is_object() {
        raw = json!({});
    }
    // Page-only inputs become a standard profile tree; the full original page
    // payload is retained in the exported page's manifest above.
    raw.as_object_mut().unwrap().remove("Controllers");
    raw.as_object_mut().unwrap().remove("Actions");
    raw["Name"] = json!(profile.name);
    if !raw["Pages"].is_object() {
        raw["Pages"] = json!({});
    }
    raw["Pages"]["Current"] = json!(active_page);
    raw["Pages"]["Default"] = json!(profile.pin_page);
    raw["Pages"]["Pages"] = json!(profile.page_order);
    if raw.get("Version").is_none() {
        raw["Version"] = json!("2.0");
    }
    if raw.get("Device").is_none() {
        raw["Device"] = json!({"Model":"20GBD9901","UUID":profile.device});
    }
    files.insert(
        format!("{root}manifest.json"),
        serde_json::to_vec_pretty(&raw)?,
    );
    let total = files.values().try_fold(0usize, |sum, v| {
        sum.checked_add(v.len()).context("Export size overflow")
    })?;
    ensure!(
        files.len() <= 10_000 && total <= 128 * 1024 * 1024,
        "Export exceeds profile size limits"
    );
    Ok(files)
}

fn insert_controller_slots(
    raw: &mut Value,
    page: &library::Page,
    page_root: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    roots: &HashMap<String, PathBuf>,
) -> Result<()> {
    if raw.get("Controllers").is_none() {
        raw["Controllers"] = json!([]);
    }
    let controllers = raw["Controllers"]
        .as_array_mut()
        .context("Page Controllers must be an array")?;
    for controller in controllers.iter_mut() {
        if let Some(kind) = controller["Type"].as_str().map(str::to_owned)
            && matches!(kind.as_str(), "Keypad" | "Encoder")
        {
            clear_managed_positions(&mut controller["Actions"], &kind);
        }
    }
    let mut occupied = HashSet::new();
    for slot in &page.slots {
        let position = managed_position(slot)?;
        ensure!(
            occupied.insert((slot.controller.as_str(), position.clone())),
            "Duplicate managed slot {} {position}",
            slot.controller
        );
        let index = match controllers
            .iter()
            .position(|c| c["Type"] == slot.controller)
        {
            Some(index) => index,
            None => {
                controllers.push(json!({"Type":slot.controller,"Actions":{}}));
                controllers.len() - 1
            }
        };
        if controllers[index].get("Actions").is_none() {
            controllers[index]["Actions"] = json!({});
        }
        let actions = controllers[index]["Actions"]
            .as_object_mut()
            .context("Known controller Actions must be an object")?;
        actions.insert(position, export_action(slot, page_root, files, roots)?);
    }
    Ok(())
}

fn clear_managed_positions(actions: &mut Value, controller: &str) {
    if let Some(actions) = actions.as_object_mut() {
        actions.retain(|position, _| {
            let Some((column, row)) = position.split_once(',') else {
                return true;
            };
            let (Ok(column), Ok(row)) = (column.parse::<u32>(), row.parse::<u32>()) else {
                return true;
            };
            !match controller {
                "Keypad" => column < 4 && row < 2,
                "Encoder" => column < 4 && row == 0,
                _ => false,
            }
        });
    }
}

fn managed_position(slot: &Slot) -> Result<String> {
    match slot.controller.as_str() {
        "Keypad" if slot.index < 8 => Ok(format!("{},{}", slot.index % 4, slot.index / 4)),
        "Encoder" if slot.index < 4 => Ok(format!("{},0", slot.index)),
        _ => bail!("Invalid managed slot {} {}", slot.controller, slot.index),
    }
}

fn export_action(
    slot: &Slot,
    page_root: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    roots: &HashMap<String, PathBuf>,
) -> Result<Value> {
    let mut raw = slot.raw.clone();
    let native = raw.as_object().is_none_or(|map| map.is_empty());
    if native {
        raw = json!({});
    }
    raw["UUID"] = json!(slot.uuid);
    if native {
        raw["Name"] = json!(slot.name);
    }
    if (native || raw.get("Settings").is_some() || slot.settings != json!({}))
        && !(raw.get("Settings") == Some(&Value::Null) && slot.settings == json!({}))
    {
        raw["Settings"] = slot.settings.clone();
    }
    if native || raw.get("State").is_some() || slot.state != 0 {
        raw["State"] = json!(slot.state);
    }
    if raw.get("ActionID").is_none() {
        raw["ActionID"] = json!(
            uuid::Uuid::parse_str(&slot.context)
                .unwrap_or_else(|_| uuid::Uuid::new_v4())
                .to_string()
        );
    }
    let mut states = slot.states.as_array().cloned().unwrap_or_default();
    let state = usize::try_from(slot.state).context("Action state index overflow")?;
    ensure!(state < 1024, "Action state index exceeds export limit");
    if native || slot.image_changed || slot.title_changed {
        states.resize_with(states.len().max(state + 1), || json!({}));
        if !states[state].is_object() {
            states[state] = json!({});
        }
        if native || slot.title_changed {
            states[state]["Title"] = json!(slot.title);
        }
    }
    for (index, metadata) in states.iter_mut().enumerate() {
        if !metadata.is_object() {
            continue;
        }
        if slot.image_changed && index == state {
            if let Some(background) = &slot.background {
                embed_png(background, metadata, page_root, files)?;
            } else {
                metadata["Image"] = json!("");
            }
        } else if native
            && let Some(source) = metadata["Image"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        {
            if let Some(root) = roots.get(&slot.uuid)
                && let Some((bytes, extension)) = original_image(root, &source)?
            {
                let relative = format!("Images/deckforge-{}.{}", uuid::Uuid::new_v4(), extension);
                insert_unique(files, format!("{page_root}{relative}"), bytes)?;
                metadata["Image"] = json!(relative);
                continue;
            }
            // Only a new native assignment may fall back to rendered pixels;
            // existing imported animated/vector assets are never flattened.
            if index == state
                && let Some(background) = &slot.background
            {
                embed_png(background, metadata, page_root, files)?;
            }
        } else if native
            && index == state
            && metadata.get("Image").is_none()
            && let Some(background) = &slot.background
        {
            embed_png(background, metadata, page_root, files)?;
        }
    }
    if native || raw.get("States").is_some() || !states.is_empty() {
        raw["States"] = json!(states);
    }
    Ok(raw)
}

fn original_image(root: &Path, source: &str) -> Result<Option<(Vec<u8>, String)>> {
    for suffix in ["", ".png", ".jpg", ".svg", ".gif"] {
        let Ok(path) = safe_file(root, &format!("{source}{suffix}")) else {
            continue;
        };
        ensure!(
            fs::metadata(&path)?.len() <= 32 * 1024 * 1024,
            "Plugin image exceeds profile asset limit"
        );
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();
        ensure!(
            extension.chars().all(|c| c.is_ascii_alphanumeric()),
            "Invalid image extension"
        );
        return Ok(Some((fs::read(path)?, extension)));
    }
    Ok(None)
}

fn embed_png(
    image: &DynamicImage,
    metadata: &mut Value,
    page_root: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    let relative = format!("Images/deckforge-{}.png", uuid::Uuid::new_v4());
    insert_unique(files, format!("{page_root}{relative}"), bytes.into_inner())?;
    metadata["Image"] = json!(relative);
    Ok(())
}

fn safe_export_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.contains(['\\', ':', '\0'])
            && path
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."),
        "Unsafe export path {path:?}"
    );
    Ok(())
}

fn insert_unique(files: &mut BTreeMap<String, Vec<u8>>, name: String, data: Vec<u8>) -> Result<()> {
    safe_export_path(&name)?;
    ensure!(
        data.len() <= 32 * 1024 * 1024,
        "Export asset exceeds 32 MiB"
    );
    ensure!(
        files.insert(name.clone(), data).is_none(),
        "Duplicate export asset {name}"
    );
    Ok(())
}

fn write_archive(path: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<Option<PathBuf>> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut pending = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut archive = zip::ZipWriter::new(pending.as_file_mut());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            safe_export_path(name)?;
            archive.start_file(name, options)?;
            archive.write_all(data)?;
        }
        archive.finish()?;
    }
    pending.as_file().sync_all()?;
    // Validate the complete generated archive before touching an existing output.
    profile::load_bundle(pending.path()).context("Exported profile failed roundtrip validation")?;
    let backup = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Export destination must be a regular file"
            );
            let file_name = path
                .file_name()
                .context("Export destination has no filename")?
                .to_string_lossy();
            let backup = parent.join(format!("{file_name}.backup-{}", uuid::Uuid::new_v4()));
            let mut target = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)?;
            std::io::copy(&mut fs::File::open(path)?, &mut target)?;
            target.sync_all()?;
            Some(backup)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    pending.persist(path).map_err(|error| error.error)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Frame, Rgba, RgbaImage};

    fn gif() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            encoder
                .encode_frame(Frame::new(RgbaImage::from_pixel(
                    2,
                    2,
                    Rgba([255, 0, 0, 255]),
                )))
                .unwrap();
            encoder
                .encode_frame(Frame::new(RgbaImage::from_pixel(
                    2,
                    2,
                    Rgba([0, 0, 255, 255]),
                )))
                .unwrap();
        }
        bytes
    }

    fn runtime_slot(slot: &profile::Slot) -> Slot {
        let index = if slot.controller == "Keypad" {
            slot.row * 4 + slot.column
        } else {
            slot.column
        };
        serde_json::from_value(json!({
            "context":uuid::Uuid::new_v4().to_string(),"controller":slot.controller,"index":index,
            "uuid":slot.uuid,"name":slot.raw["Name"].as_str().unwrap_or("Imported"),
            "settings":slot.settings,"title":slot.states[0]["Title"].as_str().unwrap_or(""),
            "state":0,"feedback":{},"layout":"$B1","states":slot.states,
            "background":null,"raw":slot.raw,"image_changed":false,"pinned":false
        }))
        .unwrap()
    }

    fn imported(path: &Path) -> library::Profile {
        let bundle = profile::load_bundle(path).unwrap();
        let source_root = bundle
            .manifest_path()
            .trim_end_matches("manifest.json")
            .to_owned();
        library::Profile {
            id: bundle.uuid.clone(),
            name: bundle.name.clone(),
            device: "test-serial".into(),
            pin_page: bundle.default_page.clone(),
            page_order: bundle.page_order.clone(),
            raw: bundle.raw.clone(),
            source: Some(path.to_owned()),
            owner_plugin: None,
            readonly: false,
            pages: bundle
                .pages
                .iter()
                .map(|page| library::Page {
                    id: page.page.clone(),
                    name: page.page.clone(),
                    raw: page.page_raw.clone(),
                    source_manifest: Some(page.manifest_path()),
                    manifest_path: page
                        .manifest_path()
                        .strip_prefix(&source_root)
                        .unwrap()
                        .to_owned(),
                    slots: page
                        .slots
                        .iter()
                        .filter(|s| {
                            (s.controller == "Keypad" && s.column < 4 && s.row < 2)
                                || (s.controller == "Encoder" && s.column < 4 && s.row == 0)
                        })
                        .map(runtime_slot)
                        .collect(),
                })
                .collect(),
        }
    }

    fn source_fixture(directory: &Path) -> (PathBuf, String, String, Vec<u8>) {
        let p1 = "37aea345-7227-420a-bff0-988d1b05aeaa".to_owned();
        let p2 = "75560e74-1759-4cf4-8cfb-91d738b906c4".to_owned();
        let mut files = BTreeMap::new();
        files.insert(
            "abc.sdProfile/manifest.json".into(),
            serde_json::to_vec(&json!({
                "Name":"Before","Version":"2.0","FutureRoot":{"unknown":true},
                "Device":{"Model":"20GBD9901","UUID":"original-serial","Extra":7},
                "Pages":{"Current":p1,"Default":p1,"Pages":[p1],"FuturePageMetadata":9}
            }))
            .unwrap(),
        );
        let action = json!({"UUID":"unknown.action","ActionID":"preserve-id","Name":"Original",
            "Settings":{"number":20},"States":[{"Title":"Old","Image":"Images/motion.gif","FutureState":5}],"FutureAction":6});
        let page = json!({"FuturePage":8,"Controllers":[
            {"Type":"Keypad","FutureController":3,"Actions":{"0,0":action,"7,4":{"UUID":"unknown.outside","Settings":{},"States":[],"Keep":4}}},
            {"Type":"FutureTouch","Actions":[{"Anything":"goes"}],"Keep":11}
        ]});
        let animation = gif();
        for id in [&p1, &p2] {
            let prefix = format!(
                "abc.sdProfile/Profiles/{}/",
                profile::encoded_uuid(id).unwrap()
            );
            files.insert(
                format!("{prefix}manifest.json"),
                serde_json::to_vec(&page).unwrap(),
            );
            files.insert(format!("{prefix}Images/motion.gif"), animation.clone());
            files.insert(format!("{prefix}Unrecognized/child.bin"), vec![7, 8, 9]);
        }
        files.insert("abc.sdProfile/Unknown/blob.dat".into(), vec![4, 5, 6]);
        files.insert("outside-profile.bin".into(), vec![10, 11]);
        let path = directory.join("original.streamDeckProfile");
        write_archive(&path, &files).unwrap();
        (path, p1, p2, animation)
    }

    #[test]
    fn roundtrip_preserves_unknown_fields_animation_and_hidden_page_assets() {
        let dir = tempfile::tempdir().unwrap();
        let (source, current, folder, animation) = source_fixture(dir.path());
        let mut profile = imported(&source);
        profile.name = "After".into();
        let active = profile.pages.iter_mut().find(|p| p.id == current).unwrap();
        active.name = "Renamed page".into();
        active.slots[0].settings = json!({"number":42,"extra":true});
        active.slots[0].title = "New title".into();
        active.slots[0].title_changed = true;
        // A decoded preview must not replace the original animated asset.
        active.slots[0].background = Some(DynamicImage::ImageRgb8(RgbImage::from_pixel(
            2,
            2,
            Rgb([0, 255, 0]),
        )));
        let files = build_profile_files(&profile, &current, &HashMap::new()).unwrap();
        let exported = dir.path().join("export.streamDeckProfile");
        write_archive(&exported, &files).unwrap();
        let restored = profile::load_bundle(&exported).unwrap();
        assert_eq!(restored.name, "After");
        assert_eq!(restored.pages.len(), 2);
        assert_eq!(restored.raw["FutureRoot"]["unknown"], true);
        assert_eq!(restored.raw["Pages"]["FuturePageMetadata"], 9);
        assert_eq!(restored.raw["Device"]["UUID"], "original-serial");
        let page = restored.page(&current).unwrap();
        assert_eq!(page.page_raw["Name"], "Renamed page");
        let updated = page.slots.iter().find(|s| s.column == 0).unwrap();
        assert_eq!(updated.settings, json!({"number":42,"extra":true}));
        assert_eq!(updated.raw["ActionID"], "preserve-id");
        assert_eq!(updated.raw["FutureAction"], 6);
        assert_eq!(updated.states[0]["Title"], "New title");
        assert_eq!(updated.states[0]["FutureState"], 5);
        assert_eq!(page.page_raw["Controllers"][0]["FutureController"], 3);
        assert_eq!(page.page_raw["Controllers"][1]["Keep"], 11);
        assert_eq!(page.page_raw["Controllers"][0]["Actions"]["7,4"]["Keep"], 4);
        assert_eq!(
            restored.asset(&current, "Images/motion.gif").unwrap(),
            animation
        );
        assert_eq!(
            restored.asset(&folder, "Images/motion.gif").unwrap(),
            animation
        );
        assert!(
            restored
                .files()
                .any(|(name, bytes)| name == "outside-profile.bin" && bytes == [10, 11])
        );
    }

    #[test]
    fn deleting_page_does_not_resurrect_it_and_duplicate_page_rebases_assets() {
        let dir = tempfile::tempdir().unwrap();
        let (source, current, folder, animation) = source_fixture(dir.path());
        let mut profile = imported(&source);
        let mut duplicate = profile
            .pages
            .iter()
            .find(|p| p.id == folder)
            .unwrap()
            .clone();
        duplicate.id = uuid::Uuid::new_v4().to_string();
        duplicate.manifest_path = format!(
            "Profiles/{}/manifest.json",
            profile::encoded_uuid(&duplicate.id).unwrap()
        );
        let duplicate_id = duplicate.id.clone();
        profile.pages.retain(|p| p.id != folder);
        profile.pages.push(duplicate);
        let files = build_profile_files(&profile, &current, &HashMap::new()).unwrap();
        let exported = dir.path().join("duplicate.streamDeckProfile");
        write_archive(&exported, &files).unwrap();
        let restored = profile::load_bundle(&exported).unwrap();
        assert_eq!(restored.pages.len(), 2);
        assert!(restored.page(&folder).is_none());
        assert_eq!(
            restored.asset(&duplicate_id, "Images/motion.gif").unwrap(),
            animation
        );
        assert_eq!(
            restored
                .asset(&duplicate_id, "Unrecognized/child.bin")
                .unwrap(),
            [7, 8, 9]
        );
    }

    #[test]
    fn native_plugin_assets_keep_original_bytes_explicit_changes_use_png() {
        let dir = tempfile::tempdir().unwrap();
        let (source, current, _, _) = source_fixture(dir.path());
        let mut profile = imported(&source);
        let slot = &mut profile
            .pages
            .iter_mut()
            .find(|p| p.id == current)
            .unwrap()
            .slots[0];
        slot.raw = Value::Null;
        let animation = gif();
        fs::write(dir.path().join("original.gif"), &animation).unwrap();
        slot.states = json!([{"Image":"original.gif","Title":"Icon"}]);
        slot.background = Some(DynamicImage::ImageRgb8(RgbImage::from_pixel(
            2,
            2,
            Rgb([0, 255, 0]),
        )));
        let roots = HashMap::from([(slot.uuid.clone(), dir.path().to_owned())]);
        let files = build_profile_files(&profile, &current, &roots).unwrap();
        let exported = dir.path().join("native.streamDeckProfile");
        write_archive(&exported, &files).unwrap();
        let restored = profile::load_bundle(&exported).unwrap();
        let page = restored.page(&current).unwrap();
        let action = page.slots.iter().find(|s| s.column == 0).unwrap();
        let image = action.states[0]["Image"].as_str().unwrap();
        assert!(image.ends_with(".gif"));
        assert_eq!(page.asset(image).unwrap(), animation);
        profile
            .pages
            .iter_mut()
            .find(|p| p.id == current)
            .unwrap()
            .slots[0]
            .image_changed = true;
        let files = build_profile_files(&profile, &current, &roots).unwrap();
        write_archive(&exported, &files).unwrap();
        let restored = profile::load_bundle(&exported).unwrap();
        let page = restored.page(&current).unwrap();
        let action = page.slots.iter().find(|s| s.column == 0).unwrap();
        let data = page
            .asset(action.states[0]["Image"].as_str().unwrap())
            .unwrap();
        assert!(data.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            image::load_from_memory(&data)
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0,
            [0, 255, 0]
        );
    }

    #[test]
    #[ignore = "requires Elgato 7.6 installer artifacts; DECKFORGE_ELGATO_RAW overrides their directory"]
    fn installer_profiles_roundtrip_all_pages_and_every_asset_byte() {
        let root = std::env::var_os("DECKFORGE_ELGATO_RAW")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../evidence/elgato-7.6/raw")
            });
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "flsysF8Fq6CoTq162E1zc6fyeIlKcw",
            "fls1.KgkvZ_osjTGmFQpIDeeaNhGPU",
        ] {
            let source = root.join(file);
            let before = profile::load_bundle(&source).unwrap();
            let profile = imported(&source);
            let files =
                build_profile_files(&profile, &before.current_page, &HashMap::new()).unwrap();
            let output = dir.path().join(format!("{file}.streamDeckProfile"));
            write_archive(&output, &files).unwrap();
            let after = profile::load_bundle(&output).unwrap();
            assert_eq!(after.pages.len(), 8);
            assert_eq!(before.raw, after.raw);
            for page in &before.pages {
                let restored = after.page(&page.page).unwrap();
                assert_eq!(restored.slots.len(), page.slots.len());
                for action in &page.slots {
                    let restored_action = restored
                        .slots
                        .iter()
                        .find(|s| {
                            s.controller == action.controller
                                && s.column == action.column
                                && s.row == action.row
                        })
                        .unwrap();
                    assert_eq!(restored_action.raw, action.raw);
                }
            }
            let exported_files: BTreeMap<_, _> = after.files().collect();
            for (path, bytes) in before
                .files()
                .filter(|(p, _)| !p.ends_with("manifest.json"))
            {
                assert_eq!(
                    exported_files.get(path).copied(),
                    Some(bytes),
                    "Changed source asset {path}"
                );
            }
        }
    }

    #[test]
    fn root_page_only_import_exports_a_complete_tree_with_later_page_edits() {
        let dir = tempfile::tempdir().unwrap();
        for legacy in [false, true] {
            let action = json!({"UUID":"unknown.action","Settings":{"value":20},
                "States":[{"Image":"Images/motion.gif","FutureState":true}],"FutureAction":7});
            let mut raw =
                json!({"Name":"Single page","Version":"2.0","FutureRoot":{"retained":true}});
            if legacy {
                raw["Actions"] = json!({"0,0":action});
            } else {
                raw["Controllers"] =
                    json!([{"Type":"Keypad","Actions":{"0,0":action},"FutureController":8}]);
            }
            let animation = gif();
            let files = BTreeMap::from([
                (
                    "single.sdProfile/manifest.json".into(),
                    serde_json::to_vec(&raw).unwrap(),
                ),
                (
                    "single.sdProfile/Images/motion.gif".into(),
                    animation.clone(),
                ),
                (
                    "single.sdProfile/Future/unrecognized.dat".into(),
                    vec![9, 8, 7],
                ),
            ]);
            let source = dir
                .path()
                .join(format!("page-only-{legacy}.streamDeckProfile"));
            write_archive(&source, &files).unwrap();
            let mut profile = imported(&source);
            assert_eq!(profile.pages[0].id, "");
            // The library normalizes the single root page to a generated page
            // UUID and keeps its original manifest path for asset relocation.
            let first = uuid::Uuid::new_v4().to_string();
            profile.pages[0].id = first.clone();
            profile.pages[0].name = "Imported page".into();
            profile.pages[0].manifest_path = format!(
                "Profiles/{}/manifest.json",
                profile::encoded_uuid(&first).unwrap()
            );
            let pin = uuid::Uuid::new_v4().to_string();
            profile.pin_page = pin.clone();
            profile.pages.push(library::Page {
                id: pin.clone(),
                name: "Pinned".into(),
                slots: vec![],
                raw: json!({"Controllers":[]}),
                source_manifest: None,
                manifest_path: format!(
                    "Profiles/{}/manifest.json",
                    profile::encoded_uuid(&pin).unwrap()
                ),
            });
            // Model adding a page after import and editing its cloned action.
            let mut added = profile.pages[0].clone();
            let added_id = uuid::Uuid::new_v4().to_string();
            added.id = added_id.clone();
            added.name = "Added later".into();
            added.manifest_path = format!(
                "Profiles/{}/manifest.json",
                profile::encoded_uuid(&added_id).unwrap()
            );
            added.slots[0].settings = json!({"value":99});
            added.slots[0].title = "Explicit new title".into();
            added.slots[0].title_changed = true;
            profile.pages.push(added);
            profile.page_order = vec![first.clone(), added_id.clone()];
            let files = build_profile_files(&profile, &added_id, &HashMap::new()).unwrap();
            let output = dir
                .path()
                .join(format!("page-only-export-{legacy}.streamDeckProfile"));
            write_archive(&output, &files).unwrap();
            let restored = profile::load_bundle(&output).unwrap();
            assert_eq!(restored.pages.len(), 3);
            assert_eq!(restored.page_order, vec![first.clone(), added_id.clone()]);
            assert_eq!(restored.current_page, added_id);
            assert_eq!(restored.default_page, pin);
            assert!(restored.raw.get("Controllers").is_none());
            assert!(restored.raw.get("Actions").is_none());
            assert_eq!(restored.raw["FutureRoot"]["retained"], true);
            let original = restored.page(&first).unwrap();
            assert_eq!(original.slots[0].settings["value"], 20);
            assert!(original.slots[0].states[0].get("Title").is_none());
            assert_eq!(original.page_raw["FutureRoot"]["retained"], true);
            let added = restored.page(&added_id).unwrap();
            assert_eq!(added.page_raw["Name"], "Added later");
            assert_eq!(added.slots[0].settings["value"], 99);
            assert_eq!(added.slots[0].states[0]["Title"], "Explicit new title");
            assert_eq!(added.slots[0].raw["FutureAction"], 7);
            for page in [original, added] {
                assert_eq!(page.asset("Images/motion.gif").unwrap(), animation);
                assert_eq!(page.asset("Future/unrecognized.dat").unwrap(), [9, 8, 7]);
            }
        }
    }

    #[test]
    fn untouched_imported_title_omission_survives_runtime_placeholder() {
        let parsed = profile::Slot {
            controller: "Keypad".into(),
            column: 0,
            row: 0,
            uuid: "unknown.action".into(),
            settings: json!({}),
            states: json!([{"ShowTitle":false,"FutureState":7}]),
            raw: json!({"UUID":"unknown.action","Settings":{},"States":[{"ShowTitle":false,"FutureState":7}]}),
        };
        let mut slot = runtime_slot(&parsed);
        slot.title = "UI action-name placeholder".into();
        slot.title_changed = false;
        let mut files = BTreeMap::new();
        let action = export_action(
            &slot,
            "profile.sdProfile/Profiles/page/",
            &mut files,
            &HashMap::new(),
        )
        .unwrap();
        assert!(action["States"][0].get("Title").is_none());
        assert_eq!(action["States"][0]["ShowTitle"], false);
        assert_eq!(action["States"][0]["FutureState"], 7);
        slot.title_changed = true;
        slot.title = "Explicit edited title".into();
        let action = export_action(
            &slot,
            "profile.sdProfile/Profiles/page/",
            &mut files,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(action["States"][0]["Title"], "Explicit edited title");
        assert_eq!(action["States"][0]["ShowTitle"], false);
        assert_eq!(action["States"][0]["FutureState"], 7);
    }

    #[test]
    fn overwrite_keeps_exact_backup_and_invalid_export_keeps_destination() {
        let dir = tempfile::tempdir().unwrap();
        let (source, current, _, _) = source_fixture(dir.path());
        let original = fs::read(&source).unwrap();
        let profile = imported(&source);
        let files = build_profile_files(&profile, &current, &HashMap::new()).unwrap();
        let backup = write_archive(&source, &files).unwrap().unwrap();
        assert_eq!(fs::read(backup).unwrap(), original);
        let before = fs::read(&source).unwrap();
        assert!(write_archive(&source, &BTreeMap::from([("invalid".into(), vec![1])])).is_err());
        assert_eq!(fs::read(source).unwrap(), before);
    }
}
