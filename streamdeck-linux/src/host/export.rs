//! Profile ZIP export preserves source bytes and unknown fields. It does not
//! translate platform-specific actions or claim acceptance by the Windows app.
use super::*;
use anyhow::ensure;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};

impl Host {
    pub(super) fn export_profile(&mut self, path: &Path) -> Result<()> {
        if let Ok(destination) = path.canonicalize() {
            ensure!(
                !self
                    .retained_asset_sources()
                    .into_iter()
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
    let source = profile
        .source
        .as_ref()
        .map(|path| profile::load_bundle(path))
        .transpose()?;
    let added_audio = profile
        .pages
        .iter()
        .flat_map(|page| &page.slots)
        .flat_map(Slot::flatten)
        .any(|slot| {
            slot.uuid == "com.elgato.streamdeck.soundboard.playaudio"
                && slot.settings["path"]
                    .as_str()
                    .is_some_and(|path| !path.is_empty())
                && (slot.raw.as_object().is_none_or(|raw| raw.is_empty())
                    || slot.settings["path"] != slot.raw["Settings"]["path"]
                    || (slot.raw.pointer("/Resources/file").is_none()
                        && slot.settings["path"]
                            .as_str()
                            .is_some_and(|p| Path::new(p).is_file())))
        });
    let added_resources = profile
        .pages
        .iter()
        .flat_map(|p| &p.slots)
        .flat_map(Slot::flatten)
        .any(|slot| {
            slot.resources
                .as_ref()
                .is_some_and(|paths| !paths.is_empty())
        });
    let transferred_resources = profile
        .pages
        .iter()
        .flat_map(|p| &p.slots)
        .flat_map(Slot::flatten)
        .any(|s| {
            s.asset_source.is_some()
                && s.raw["Resources"]
                    .as_object()
                    .is_some_and(|r| !r.is_empty())
        });
    let wrapped = transferred_resources
        || added_resources
        || added_audio
        || source
            .as_ref()
            .is_some_and(|bundle| bundle.manifest_path().starts_with("Profiles/"));
    let root = format!(
        "{}{}.sdProfile/",
        if wrapped { "Profiles/" } else { "" },
        profile.id
    );
    let mut files = BTreeMap::new();
    let mut resource_ids = HashMap::new();
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
        let page_bundle = page
            .asset_source
            .as_ref()
            .map(|o| profile::load_bundle(&o.source))
            .transpose()?;
        let page_source = page_bundle.as_ref().or(source.as_ref());
        let page_manifest = page
            .asset_source
            .as_ref()
            .map(|o| &o.manifest)
            .or(page.source_manifest.as_ref());
        if let (Some(source), Some(old_manifest)) = (page_source, page_manifest) {
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
                insert_controller_slots(
                    &mut raw,
                    page,
                    &page_root,
                    &mut files,
                    plugin_roots,
                    &mut resource_ids,
                    profile.model,
                )?;
            } else {
                // Retain the legacy page-level Actions shape when no encoder exists.
                clear_managed_positions(&mut raw["Actions"], "Keypad", profile.model);
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
                    raw["Actions"][position] = export_action(
                        slot,
                        &page_root,
                        &mut files,
                        plugin_roots,
                        &mut resource_ids,
                    )?;
                }
            }
        } else {
            insert_controller_slots(
                &mut raw,
                page,
                &page_root,
                &mut files,
                plugin_roots,
                &mut resource_ids,
                profile.model,
            )?;
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
        raw["Device"] = json!({"Model":profile.model.profile_model(),"UUID":profile.device});
    }
    if wrapped {
        let mut dependencies = std::collections::BTreeSet::new();
        if added_audio {
            dependencies.insert("com.elgato.streamdeck.soundboard".to_owned());
        }
        for slot in profile
            .pages
            .iter()
            .flat_map(|p| &p.slots)
            .flat_map(Slot::flatten)
            .filter(|s| s.resources.as_ref().is_some_and(|p| !p.is_empty()))
        {
            let manifest = plugin_roots
                .get(&slot.uuid)
                .and_then(|root| fs::read(root.join("manifest.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            if let Some(id) = slot.raw["Plugin"]["UUID"]
                .as_str()
                .or_else(|| manifest.as_ref().and_then(|v| v["UUID"].as_str()))
            {
                dependencies.insert(id.to_owned());
            }
        }
        raw["Version"] = json!("3.0");
        if !files.contains_key("package.json") {
            let package = json!({"FormatVersion":1,"DeviceModel":raw["Device"]["Model"],
                "RequiredPlugins":dependencies});
            files.insert("package.json".into(), serde_json::to_vec_pretty(&package)?);
        } else if !dependencies.is_empty() {
            let mut package: Value = serde_json::from_slice(&files["package.json"])?;
            let required = package
                .as_object_mut()
                .context("Package metadata must be an object")?
                .entry("RequiredPlugins")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .context("RequiredPlugins must be an array")?;
            let mut changed = false;
            for id in dependencies {
                if !required.iter().any(|old| old == &id) {
                    required.push(json!(id));
                    changed = true;
                }
            }
            if changed {
                files.insert("package.json".into(), serde_json::to_vec_pretty(&package)?);
            }
        }
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
    resource_ids: &mut HashMap<String, String>,
    model: Model,
) -> Result<()> {
    if raw.get("Controllers").is_none() {
        raw["Controllers"] = json!([]);
    }
    let controllers = raw["Controllers"]
        .as_array_mut()
        .context("Page Controllers must be an array")?;
    for controller in controllers.iter_mut() {
        if let Some(kind) = controller["Type"].as_str().map(str::to_owned)
            && matches!(kind.as_str(), "Keypad" | "Encoder" | "Neo")
        {
            clear_managed_positions(&mut controller["Actions"], &kind, model);
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
        if controllers[index].get("Actions").is_none_or(Value::is_null) {
            controllers[index]["Actions"] = json!({});
        }
        let actions = controllers[index]["Actions"]
            .as_object_mut()
            .context("Known controller Actions must be an object")?;
        actions.insert(
            position,
            export_action(slot, page_root, files, roots, resource_ids)?,
        );
    }
    Ok(())
}

fn clear_managed_positions(actions: &mut Value, controller: &str, model: Model) {
    if let Some(actions) = actions.as_object_mut() {
        actions.retain(|position, _| {
            let Some((column, row)) = position.split_once(',') else {
                return true;
            };
            let (Ok(column), Ok(row)) = (column.parse::<u32>(), row.parse::<u32>()) else {
                return true;
            };
            !match controller {
                "Keypad" => column < u32::from(model.columns()) && row < u32::from(model.rows()),
                "Encoder" => column < u32::from(model.dial_count()) && row == 0,
                "Neo" => model == Model::Neo && column == 1 && row == 0,
                _ => false,
            }
        });
    }
}

fn managed_position(slot: &Slot) -> Result<String> {
    match slot.controller.as_str() {
        "Keypad" if slot.index < slot.model.key_count() => Ok(format!(
            "{},{}",
            slot.index % slot.model.columns(),
            slot.index / slot.model.columns()
        )),
        "Encoder" if slot.index < slot.model.dial_count() => Ok(format!("{},0", slot.index)),
        "Neo" if slot.model == Model::Neo && slot.index == 1 => Ok("1,0".into()),
        _ => bail!("Invalid managed slot {} {}", slot.controller, slot.index),
    }
}

fn export_action(
    slot: &Slot,
    page_root: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    roots: &HashMap<String, PathBuf>,
    resource_ids: &mut HashMap<String, String>,
) -> Result<Value> {
    export_action_at(slot, page_root, files, roots, resource_ids, 0)
}

fn export_action_at(
    slot: &Slot,
    page_root: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    roots: &HashMap<String, PathBuf>,
    resource_ids: &mut HashMap<String, String>,
    depth: usize,
) -> Result<Value> {
    ensure!(depth <= 8, "Container nesting exceeds eight export levels");
    let mut raw = slot.raw.clone();
    let native = raw.as_object().is_none_or(|map| map.is_empty());
    if native {
        raw = json!({});
    }
    let origin = slot
        .asset_source
        .as_ref()
        .map(|o| profile::load_bundle(&o.source))
        .transpose()?;
    if let Some(bundle) = &origin {
        preserve_transferred_resources(slot, bundle, &mut raw, files, resource_ids)?;
    }
    embed_audio_resource(slot, native, &mut raw, files)?;
    embed_plugin_resources(slot, &mut raw, files, resource_ids)?;
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
    if let Some((origin, bundle)) = slot.asset_source.as_ref().zip(origin.as_ref()) {
        let prefix = origin
            .manifest
            .strip_suffix("manifest.json")
            .context("Invalid asset source page")?;
        for (name, bytes) in bundle.files() {
            if let Some(relative) = name.strip_prefix(prefix).filter(|r| *r != "manifest.json") {
                let direct = format!("{page_root}{relative}");
                if files
                    .get(&direct)
                    .is_some_and(|old| old.as_slice() != bytes)
                {
                    insert_unique(
                        files,
                        format!("{page_root}Transferred/{}/{relative}", slot.context),
                        bytes.to_vec(),
                    )?;
                } else if !files.contains_key(&direct) {
                    insert_unique(files, direct, bytes.to_vec())?;
                }
            }
        }
    }
    let mut states = slot.states.as_array().cloned().unwrap_or_default();
    let state = usize::try_from(slot.state).context("Action state index overflow")?;
    ensure!(state < 1024, "Action state index exceeds export limit");
    if native || slot.image_changed || slot.title_changed {
        states.resize_with(states.len().max(state + 1), || json!({}));
        if !states[state].is_object() {
            states[state] = json!({});
        }
        if slot.title_changed
            || (native
                && !slot
                    .appearances
                    .get(&(state as u64))
                    .is_some_and(|a| a.plugin_title.is_some() && a.user_title.is_none()))
        {
            states[state]["Title"] = json!(slot.title);
        }
    }
    if let Some(last) = slot.appearances.keys().next_back() {
        ensure!(*last < 1024, "Appearance state exceeds export limit");
        states.resize_with(states.len().max(*last as usize + 1), || json!({}));
    }
    for (index, metadata) in states.iter_mut().enumerate() {
        if !metadata.is_object() {
            continue;
        }
        let appearance = slot.appearances.get(&(index as u64));
        if let Some(title) = appearance.and_then(|a| a.user_title.as_ref()) {
            metadata["Title"] = json!(title);
        }
        if appearance.is_some_and(|a| a.title_reset && a.user_title.is_none()) {
            metadata.as_object_mut().unwrap().remove("Title");
        }
        if appearance.is_some_and(|a| a.image_reset && a.user_image.is_none()) {
            metadata.as_object_mut().unwrap().remove("Image");
            continue;
        }
        if let Some(path) = appearance.and_then(|a| a.user_image.as_ref()) {
            let (name, bytes) = crate::resources::read_regular(path)?;
            let extension = Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bin");
            let relative = format!("Images/custom-{}-{index}.{extension}", slot.context);
            insert_unique(files, format!("{page_root}{relative}"), bytes)?;
            metadata["Image"] = json!(relative);
        } else if slot.image_changed && index == state {
            if let Some(background) = &slot.background {
                embed_png(background, metadata, page_root, files)?;
            } else {
                metadata["Image"] = json!("");
            }
        } else if let Some((origin, bundle)) = slot.asset_source.as_ref().zip(origin.as_ref())
            && let Some(image) = metadata["Image"].as_str().filter(|s| !s.is_empty())
            && let Some(page) = bundle
                .pages
                .iter()
                .find(|p| p.manifest_path() == origin.manifest)
            && let Ok(bytes) = page.asset(image)
        {
            let extension = Path::new(image)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bin");
            let relative = format!("Images/transfer-{}-{index}.{extension}", slot.context);
            insert_unique(files, format!("{page_root}{relative}"), bytes)?;
            metadata["Image"] = json!(relative);
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
                && appearance.is_none_or(|a| a.plugin_pixels.is_none())
                && let Some(background) = &slot.background
            {
                embed_png(background, metadata, page_root, files)?;
            }
        } else if native
            && index == state
            && metadata.get("Image").is_none()
            && appearance.is_none_or(|a| a.plugin_pixels.is_none())
            && let Some(background) = &slot.background
        {
            embed_png(background, metadata, page_root, files)?;
        }
    }
    if native || raw.get("States").is_some() || !states.is_empty() {
        raw["States"] = json!(states);
    }
    if slot.uuid == crate::action_trigger::UUID && !slot.children.is_empty() {
        ensure!(
            slot.children.len() == 3,
            "Action Trigger runtime must retain three positions"
        );
        let mut records = raw
            .get("Actions")
            .map(|value| {
                value
                    .as_array()
                    .cloned()
                    .context("Action Trigger Actions must be an array")
            })
            .transpose()?
            .unwrap_or_default();
        ensure!(
            records.len() <= 3,
            "Action Trigger export would discard extra children"
        );
        for (index, children) in slot.children.iter().enumerate() {
            ensure!(
                children.len() <= 1,
                "Action Trigger position contains multiple runtime children"
            );
            if let Some(child) = children.first() {
                records.resize_with(records.len().max(index + 1), || json!({"Actions":[]}));
                records[index] =
                    export_action_at(child, page_root, files, roots, resource_ids, depth + 1)?;
            } else if let Some(record) = records.get_mut(index)
                && record["UUID"].as_str().is_some_and(|uuid| !uuid.is_empty())
            {
                // A removed assignment must not reappear. Already-blank records,
                // including their unknown metadata, are otherwise retained verbatim.
                *record = json!({"Actions":[]});
            }
        }
        raw["Actions"] = json!(records);
    } else if slot.uuid == crate::dial_stack::UUID && !slot.children.is_empty() {
        ensure!(
            slot.children.len() == 1,
            "Dial Stack runtime must have one child group"
        );
        raw["Actions"] = json!(
            slot.children[0]
                .iter()
                .map(|child| export_action_at(
                    child,
                    page_root,
                    files,
                    roots,
                    resource_ids,
                    depth + 1
                ))
                .collect::<Result<Vec<_>>>()?
        );
    } else if multiaction::container(&slot.uuid) && !slot.children.is_empty() {
        if raw.get("Actions").is_none() {
            raw["Actions"] = json!([]);
        }
        let groups = raw["Actions"]
            .as_array_mut()
            .context("Multi Action groups must be an array")?;
        groups.resize_with(
            groups.len().max(slot.children.len()),
            || json!({"Actions":[]}),
        );
        for (branch, children) in slot.children.iter().enumerate() {
            let exported = children
                .iter()
                .map(|child| {
                    export_action_at(child, page_root, files, roots, resource_ids, depth + 1)
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                groups[branch].is_object(),
                "Multi Action group must be an object"
            );
            groups[branch]["Actions"] = json!(exported);
        }
    }
    Ok(raw)
}

fn preserve_transferred_resources(
    slot: &Slot,
    bundle: &profile::ProfileBundle,
    raw: &mut Value,
    files: &mut BTreeMap<String, Vec<u8>>,
    ids: &mut HashMap<String, String>,
) -> Result<()> {
    if slot.resources.is_some() && slot.uuid != crate::audio_output::PLAY {
        return Ok(());
    }
    let Some(references) = raw["Resources"].as_object().cloned() else {
        return Ok(());
    };
    if references.is_empty() {
        return Ok(());
    }
    let mut registry: Value = files
        .get("Resources/manifest.json")
        .map(|b| serde_json::from_slice(b))
        .transpose()?
        .unwrap_or_else(|| json!({"resources":{}}));
    let resources = registry["resources"]
        .as_object_mut()
        .context("Invalid resource registry")?;
    for (key, id) in references {
        let original = id.as_str().context("Resource ID must be text")?;
        let origin = slot.asset_source.as_ref().unwrap();
        let alias = format!("archive:{}#{original}", origin.source.display());
        let id = if let Some(id) = ids.get(&alias) {
            id.clone()
        } else {
            let (name, bytes) = bundle.resource(original)?;
            let name = Path::new(&name)
                .file_name()
                .and_then(|s| s.to_str())
                .context("Invalid resource name")?;
            safe_export_path(name)?;
            let existing = resources
                .get(original)
                .and_then(|r| r["file"].as_str())
                .is_some_and(|relative| {
                    let member = if relative.starts_with("Resources/") {
                        relative.to_owned()
                    } else {
                        format!("Resources/{relative}")
                    };
                    Path::new(relative).file_name().and_then(|n| n.to_str()) == Some(name)
                        && files.get(&member) == Some(&bytes)
                });
            let id = if existing {
                original.to_owned()
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                let relative = format!("{id}/{name}");
                insert_unique(files, format!("Resources/{relative}"), bytes)?;
                resources.insert(id.clone(), json!({"file":relative}));
                id
            };
            ids.insert(alias, id.clone());
            id
        };
        raw["Resources"][key] = json!(id);
    }
    files.insert(
        "Resources/manifest.json".into(),
        serde_json::to_vec_pretty(&registry)?,
    );
    Ok(())
}

fn embed_plugin_resources(
    slot: &Slot,
    raw: &mut Value,
    files: &mut BTreeMap<String, Vec<u8>>,
    resource_ids: &mut HashMap<String, String>,
) -> Result<()> {
    let Some(paths) = &slot.resources else {
        return Ok(());
    };
    // The built-in soundboard's path override has its own compatibility behavior.
    if slot.uuid == crate::audio_output::PLAY {
        return Ok(());
    }
    if paths.is_empty() {
        if raw
            .get("Resources")
            .is_some_and(|v| !v.is_null() && *v != json!({}))
        {
            raw["Resources"] = json!({});
        }
        return Ok(());
    }
    let mut registry: Value = match files.get("Resources/manifest.json") {
        Some(bytes) => serde_json::from_slice(bytes).context("Invalid resource registry")?,
        None => json!({"resources":{}}),
    };
    let resources = registry
        .as_object_mut()
        .context("Resource registry must be an object")?
        .entry("resources")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("Resource map must be an object")?;
    let mut references = serde_json::Map::new();
    let mut registry_changed = false;
    for (key, path) in paths {
        if let Some(id) = resource_ids.get(path) {
            references.insert(key.clone(), json!(id));
            continue;
        }
        let (name, bytes) = crate::resources::read_regular(Path::new(path))
            .with_context(|| format!("Read resource {key:?} for export"))?;
        safe_export_path(&name)?;
        let existing = raw["Resources"][key].as_str().and_then(|id| {
            let relative = resources.get(id)?.get("file")?.as_str()?;
            let member = if relative.starts_with("Resources/") {
                relative.to_owned()
            } else {
                format!("Resources/{relative}")
            };
            (Path::new(relative).file_name().and_then(|p| p.to_str()) == Some(name.as_str())
                && files.get(&member) == Some(&bytes))
            .then(|| id.to_owned())
        });
        let id = if let Some(id) = existing {
            id
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            ensure!(!resources.contains_key(&id), "Resource ID collision");
            let relative = format!("{id}/{name}");
            insert_unique(files, format!("Resources/{relative}"), bytes)?;
            resources.insert(id.clone(), json!({"file":relative}));
            registry_changed = true;
            id
        };
        resource_ids.insert(path.clone(), id.clone());
        references.insert(key.clone(), json!(id));
    }
    raw["Resources"] = Value::Object(references);
    if registry_changed {
        files.insert(
            "Resources/manifest.json".into(),
            serde_json::to_vec_pretty(&registry)?,
        );
    }
    Ok(())
}

fn embed_audio_resource(
    slot: &Slot,
    native: bool,
    raw: &mut Value,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    if slot.uuid != "com.elgato.streamdeck.soundboard.playaudio" {
        return Ok(());
    }
    let path = slot.settings["path"].as_str();
    // An imported resource is authoritative even if its source machine's path
    // no longer exists. Keep its registry and compressed payload bytes intact.
    if !native
        && path == slot.raw["Settings"]["path"].as_str()
        && (raw.pointer("/Resources/file").is_some()
            || !path.is_some_and(|p| Path::new(p).is_file()))
    {
        return Ok(());
    }
    if let Some(resources) = raw.get_mut("Resources").and_then(Value::as_object_mut) {
        resources.remove("file");
    }
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let path = Path::new(path);
    let file = fs::File::open(path)
        .with_context(|| format!("Read soundboard media {} for export", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "Soundboard media must be a regular file"
    );
    const LIMIT: u64 = 32 * 1024 * 1024;
    ensure!(
        metadata.len() <= LIMIT,
        "Soundboard media exceeds 32 MiB export limit"
    );
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "Soundboard media exceeds 32 MiB export limit"
    );
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("Soundboard filename must be UTF-8")?;
    safe_export_path(name)?;
    let id = uuid::Uuid::new_v4().to_string();
    let relative = format!("{id}/{name}");
    let mut registry: Value = match files.get("Resources/manifest.json") {
        Some(bytes) => serde_json::from_slice(bytes).context("Invalid resource registry")?,
        None => json!({"resources":{}}),
    };
    let resources = registry
        .as_object_mut()
        .context("Resource registry must be an object")?
        .entry("resources")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("Resource map must be an object")?;
    ensure!(!resources.contains_key(&id), "Resource ID collision");
    resources.insert(id.clone(), json!({"file":relative}));
    insert_unique(files, format!("Resources/{relative}"), bytes)?;
    files.insert(
        "Resources/manifest.json".into(),
        serde_json::to_vec_pretty(&registry)?,
    );
    if !raw["Resources"].is_object() {
        raw["Resources"] = json!({});
    }
    raw["Resources"]["file"] = json!(id);
    Ok(())
}

fn original_image(root: &Path, source: &str) -> Result<Option<(Vec<u8>, String)>> {
    for suffix in display::IMAGE_SUFFIXES {
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
    ensure!(!files.contains_key(&name), "Duplicate export asset {name}");
    let total = files.values().try_fold(data.len(), |sum, bytes| {
        sum.checked_add(bytes.len()).context("Export size overflow")
    })?;
    ensure!(
        files.len() < 10_000 && total <= 128 * 1024 * 1024,
        "Export exceeds profile size limits"
    );
    files.insert(name, data);
    Ok(())
}

#[cfg(test)]
mod resource_budget_tests {
    use super::*;
    #[test]
    fn aggregate_asset_limit_rejects_before_growing_the_export_map() {
        let mut files = BTreeMap::new();
        for index in 0..4 {
            insert_unique(
                &mut files,
                format!("Resources/{index}"),
                vec![0; 32 * 1024 * 1024],
            )
            .unwrap();
        }
        assert!(insert_unique(&mut files, "Resources/overflow".into(), vec![1]).is_err());
        assert_eq!(files.len(), 4);
        assert!(insert_unique(&mut files, "Resources/0".into(), vec![1]).is_err());
        assert_eq!(files["Resources/0"].len(), 32 * 1024 * 1024);
    }
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
        let model = bundle.raw["Device"]["Model"]
            .as_str()
            .and_then(Model::from_profile_model)
            .unwrap_or_default();
        let source_root = bundle
            .manifest_path()
            .trim_end_matches("manifest.json")
            .to_owned();
        library::Profile {
            model,
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
                    name: page.page_raw["Name"].as_str().unwrap_or("").to_owned(),
                    raw: page.page_raw.clone(),
                    asset_source: None,
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
                            library::position_index(model, &s.controller, s.column, s.row).is_some()
                        })
                        .map(|s| {
                            let mut slot = runtime_slot(s);
                            slot.model = model;
                            slot.index =
                                library::position_index(model, &s.controller, s.column, s.row)
                                    .unwrap();
                            slot
                        })
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
    #[ignore = "requires Elgato 7.6 resource profile artifacts"]
    fn original_wrapped_media_exports_byte_exact_even_with_foreign_paths() {
        let root = std::env::var_os("DECKFORGE_ELGATO_RAW")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../evidence/elgato-7.6/raw")
            });
        let temporary = tempfile::tempdir().unwrap();
        let mut resources = 0;
        for file in [
            "flsKaSPXYxUUvmKtC3TdqgYiqQcKKY",
            "flsLUxNAo3HxRBo8yay7EbNtFyJC9c",
        ] {
            let path = root.join(file);
            let before = profile::load_bundle(&path).unwrap();
            let mut runtime = imported(&path);
            // Exercise the host's regenerated outer profile identity too.
            runtime.id = uuid::Uuid::new_v4().to_string();
            let files =
                build_profile_files(&runtime, &before.current_page, &HashMap::new()).unwrap();
            let output = temporary.path().join(format!("{file}.streamDeckProfile"));
            write_archive(&output, &files).unwrap();
            let after = profile::load_bundle(&output).unwrap();
            assert_eq!(
                after.manifest_path(),
                format!("Profiles/{}.sdProfile/manifest.json", runtime.id)
            );
            assert_eq!(after.raw["Version"], "3.0");
            assert_eq!(after.current_page, before.page_order[0]);
            for (path, bytes) in before
                .files()
                .filter(|(path, _)| path.starts_with("Resources/") || *path == "package.json")
            {
                assert_eq!(files.get(path).map(Vec::as_slice), Some(bytes), "{path}");
            }
            for action in after
                .pages
                .iter()
                .flat_map(|page| &page.slots)
                .filter(|slot| slot.uuid == "com.elgato.streamdeck.soundboard.playaudio")
            {
                let id = action.raw["Resources"]["file"].as_str().unwrap();
                assert_eq!(after.resource(id).unwrap(), before.resource(id).unwrap());
                assert!(
                    action.settings["path"]
                        .as_str()
                        .unwrap()
                        .starts_with("/Users/")
                );
                resources += 1;
            }
        }
        assert_eq!(resources, 15);
    }

    fn local_wav(directory: &Path, name: &str, sample: i16) -> (PathBuf, Vec<u8>) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&38u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
        let path = directory.join(name);
        fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    #[test]
    fn new_and_edited_local_audio_embed_reimport_without_stale_references() {
        let temporary = tempfile::tempdir().unwrap();
        let (source, current, _, _) = source_fixture(temporary.path());
        let (wav, bytes) = local_wav(temporary.path(), "sound æ.wav", 73);
        let mut runtime = imported(&source);
        let slot = &mut runtime
            .pages
            .iter_mut()
            .find(|page| page.id == current)
            .unwrap()
            .slots[0];
        slot.uuid = "com.elgato.streamdeck.soundboard.playaudio".into();
        slot.raw = json!({});
        slot.settings = json!({"path":wav,"actionType":1,"volume":50});
        slot.states = json!([{}]);
        let output = temporary.path().join("audio.streamDeckProfile");
        let files = build_profile_files(&runtime, &current, &HashMap::new()).unwrap();
        write_archive(&output, &files).unwrap();
        let loaded = profile::load_bundle(&output).unwrap();
        let action = &loaded.page(&current).unwrap().slots[0];
        let first_id = action.raw["Resources"]["file"].as_str().unwrap().to_owned();
        let (member, restored) = loaded.resource(&first_id).unwrap();
        assert!(member.ends_with("/sound æ.wav"));
        assert_eq!(restored, bytes);
        assert!(loaded.manifest_path().starts_with("Profiles/"));
        assert_eq!(
            serde_json::from_slice::<Value>(&files["package.json"]).unwrap()["FormatVersion"],
            1
        );
        // Changing the foreign/source path must replace the action reference,
        // while retaining old source blobs for lossless/shared-resource export.
        let mut edited = imported(&output);
        let (replacement, replacement_bytes) = local_wav(temporary.path(), "replacement.wav", -91);
        let slot = &mut edited
            .pages
            .iter_mut()
            .find(|p| p.id == current)
            .unwrap()
            .slots[0];
        slot.settings["path"] = json!(replacement);
        let output2 = temporary.path().join("edited.streamDeckProfile");
        let changed = build_profile_files(&edited, &current, &HashMap::new()).unwrap();
        write_archive(&output2, &changed).unwrap();
        let loaded2 = profile::load_bundle(&output2).unwrap();
        let second_id = loaded2.page(&current).unwrap().slots[0].raw["Resources"]["file"]
            .as_str()
            .unwrap();
        assert_ne!(second_id, first_id);
        assert_eq!(loaded2.resource(second_id).unwrap().1, replacement_bytes);
        assert_eq!(loaded2.resource(&first_id).unwrap().1, bytes);
        // Clearing a previously embedded path must not replay its stale resource.
        edited
            .pages
            .iter_mut()
            .find(|p| p.id == current)
            .unwrap()
            .slots[0]
            .settings["path"] = json!("");
        let cleared = build_profile_files(&edited, &current, &HashMap::new()).unwrap();
        let page = &edited
            .pages
            .iter()
            .find(|p| p.id == current)
            .unwrap()
            .manifest_path;
        let raw: Value =
            serde_json::from_slice(&cleared[&format!("Profiles/{}.sdProfile/{page}", edited.id)])
                .unwrap();
        assert!(
            raw["Controllers"][0]["Actions"]["0,0"]["Resources"]
                .get("file")
                .is_none()
        );
    }

    #[test]
    fn oversized_media_is_rejected_before_export() {
        let temporary = tempfile::tempdir().unwrap();
        let (source, current, _, _) = source_fixture(temporary.path());
        let huge = temporary.path().join("too-big.wav");
        fs::File::create(&huge)
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        let mut runtime = imported(&source);
        let slot = &mut runtime
            .pages
            .iter_mut()
            .find(|p| p.id == current)
            .unwrap()
            .slots[0];
        slot.uuid = "com.elgato.streamdeck.soundboard.playaudio".into();
        slot.raw = json!({});
        slot.settings = json!({"path":huge});
        assert!(
            build_profile_files(&runtime, &current, &HashMap::new())
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
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
                asset_source: None,
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
            &mut HashMap::new(),
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
            &mut HashMap::new(),
        )
        .unwrap();
        assert_eq!(action["States"][0]["Title"], "Explicit edited title");
        assert_eq!(action["States"][0]["ShowTitle"], false);
        assert_eq!(action["States"][0]["FutureState"], 7);
    }

    #[test]
    fn trigger_export_recurses_through_multi_and_preserves_positional_blank_records() {
        let raw = json!({"UUID":crate::action_trigger::UUID,"ActionID":"trigger","Name":"Trigger","Settings":{},"State":0,"States":[{}],"FutureParent":1,"Actions":[
            {"Actions":[],"FutureBlank":{"keep":true}},
            {"UUID":crate::multi::ROUTINE,"ActionID":"multi","Settings":null,"State":0,"States":[{}],"FutureMulti":2,"Actions":[
                {"FutureGroup":3,"Actions":[
                    {"UUID":"test.action","ActionID":"first","Settings":null,"State":0,"States":[{"Title":"Before","FutureState":4}],"FutureLeaf":5},
                    {"UUID":"test.action","ActionID":"second","Settings":{"value":1},"State":0,"States":[{}]}
                ]},{"Actions":[]}
            ]},{"Actions":[],"OtherBlank":9}
        ]});
        let mut slot = runtime_slot(&profile::Slot {
            controller: "Encoder".into(),
            column: 2,
            row: 0,
            uuid: crate::action_trigger::UUID.into(),
            settings: json!({}),
            states: json!([{}]),
            raw: raw.clone(),
        });
        slot.hydrate_children(&[]).unwrap();
        slot.children[1][0].children[0][0].title = "Edited nested title".into();
        slot.children[1][0].children[0][0].title_changed = true;
        slot.children[1][0].children[0][1].settings = json!({"value":42});
        let library = Library::legacy(vec![slot], "test-device");
        let files =
            build_profile_files(&library.profiles[0], &library.active_page, &HashMap::new())
                .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("trigger.streamDeckProfile");
        write_archive(&output, &files).unwrap();
        let imported = profile::load_bundle(&output).unwrap();
        let action = &imported.page(&library.active_page).unwrap().slots[0].raw;
        assert_eq!(action["Actions"].as_array().unwrap().len(), 3);
        assert_eq!(action["Actions"][0], raw["Actions"][0]);
        assert_eq!(action["Actions"][2], raw["Actions"][2]);
        assert!(action["Actions"][0].get("ActionID").is_none());
        assert_eq!(action["Actions"][1]["Settings"], Value::Null);
        let first = &action["Actions"][1]["Actions"][0]["Actions"][0];
        assert_eq!(first["Settings"], Value::Null);
        assert_eq!(first["States"][0]["Title"], "Edited nested title");
        assert_eq!(first["States"][0]["FutureState"], 4);
        assert_eq!(first["FutureLeaf"], 5);
        assert_eq!(
            action["Actions"][1]["Actions"][0]["Actions"][1]["Settings"]["value"],
            42
        );
        assert_eq!(action["Actions"][1]["Actions"][0]["FutureGroup"], 3);
        assert_eq!(action["FutureParent"], 1);
    }
    #[test]
    fn trigger_export_keeps_short_untouched_arrays_and_does_not_resurrect_removed_children() {
        let raw = json!({"UUID":crate::action_trigger::UUID,"ActionID":"trigger","Settings":null,"Actions":[{"Actions":[],"UnknownBlank":7}]});
        let mut slot = runtime_slot(&profile::Slot {
            controller: "Encoder".into(),
            column: 0,
            row: 0,
            uuid: crate::action_trigger::UUID.into(),
            settings: json!({}),
            states: json!([]),
            raw: raw.clone(),
        });
        slot.hydrate_children(&[]).unwrap();
        let mut files = BTreeMap::new();
        assert_eq!(
            export_action(
                &slot,
                "p/",
                &mut files,
                &HashMap::new(),
                &mut HashMap::new()
            )
            .unwrap(),
            raw
        );
        slot.raw["Actions"] = json!([{"UUID":"test.action","ActionID":"remove","Settings":{}}]);
        slot.hydrate_children(&[]).unwrap();
        slot.children[0].clear();
        let exported = export_action(
            &slot,
            "p/",
            &mut files,
            &HashMap::new(),
            &mut HashMap::new(),
        )
        .unwrap();
        assert_eq!(exported["Actions"][0], json!({"Actions":[]}));
    }
    #[test]
    fn recursive_export_rejects_excessive_runtime_depth() {
        let mut slot = runtime_slot(&profile::Slot {
            controller: "Keypad".into(),
            column: 0,
            row: 0,
            uuid: "test.action".into(),
            settings: json!({}),
            states: json!([]),
            raw: json!({"UUID":"test.action","ActionID":"leaf"}),
        });
        for _ in 0..9 {
            let mut parent = slot.clone();
            parent.uuid = crate::multi::ROUTINE.into();
            parent.raw = json!({"UUID":crate::multi::ROUTINE,"Actions":[{"Actions":[]}]});
            parent.children = vec![vec![slot]];
            slot = parent;
        }
        assert!(
            export_action(
                &slot,
                "p/",
                &mut BTreeMap::new(),
                &HashMap::new(),
                &mut HashMap::new()
            )
            .is_err()
        );
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
    #[test]
    fn transferred_archive_resources_and_animation_survive_without_profile_source() {
        let dir = tempfile::tempdir().unwrap();
        let (source, current, _, animation) = source_fixture(dir.path());
        let (wav, bytes) = local_wav(dir.path(), "portable.wav", 41);
        let mut original = imported(&source);
        let page = original.pages.iter_mut().find(|p| p.id == current).unwrap();
        page.slots[0].resources = Some(BTreeMap::from([(
            "media".into(),
            wav.to_str().unwrap().to_owned(),
        )]));
        let with_resource = dir.path().join("original-resources.streamDeckProfile");
        write_archive(
            &with_resource,
            &build_profile_files(&original, &current, &HashMap::new()).unwrap(),
        )
        .unwrap();
        let mut copied = imported(&with_resource);
        let page = copied.pages.iter_mut().find(|p| p.id == current).unwrap();
        page.slots[0].asset_source = Some(transfer::AssetSource {
            source: with_resource.clone(),
            manifest: page.source_manifest.clone().unwrap(),
            profile: copied.id.clone(),
        });
        let mut second = page.slots[0].clone();
        second.context = library::new_id();
        second.index = 1;
        page.slots.push(second);
        copied.pages.retain(|p| p.id == current);
        copied.pin_page = current.clone();
        copied.source = None;
        for page in &mut copied.pages {
            page.source_manifest = None;
        }
        let output = dir.path().join("transferred.streamDeckProfile");
        write_archive(
            &output,
            &build_profile_files(&copied, &current, &HashMap::new()).unwrap(),
        )
        .unwrap();
        let loaded = profile::load_bundle(&output).unwrap();
        let page = loaded.page(&current).unwrap();
        let a = &page.slots[0];
        let b = &page.slots[1];
        assert_eq!(a.raw["Resources"]["media"], b.raw["Resources"]["media"]);
        assert_eq!(
            loaded
                .resource(a.raw["Resources"]["media"].as_str().unwrap())
                .unwrap()
                .1,
            bytes
        );
        assert_eq!(
            page.asset(a.states[0]["Image"].as_str().unwrap()).unwrap(),
            animation
        );
        assert_eq!(a.settings, json!({"number":20}));
        assert_eq!(a.raw["FutureAction"], 6);
    }
}
