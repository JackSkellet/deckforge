//! In-memory action clipboard. Archive provenance is persisted with copied data,
//! never inserted into a vendor action's settings or manifest.
use super::*;
use anyhow::ensure;
use std::collections::HashSet;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct AssetSource {
    pub source: PathBuf,
    pub manifest: String,
    pub profile: String,
}
#[derive(Clone)]
struct Anchor {
    device: String,
    profile: String,
    page: String,
    context: String,
    controller: String,
    index: u8,
    uuid: String,
}
#[derive(Clone)]
pub(super) struct Clipboard {
    anchor: Anchor,
    slot: Slot,
    pages: Vec<library::Page>,
    cut: bool,
}
fn locate(library: &Library, context: &str) -> Result<(usize, usize, usize)> {
    library
        .profiles
        .iter()
        .enumerate()
        .find_map(|(p, profile)| {
            profile.pages.iter().enumerate().find_map(|(g, page)| {
                page.slots
                    .iter()
                    .position(|s| s.context == context)
                    .map(|s| (p, g, s))
            })
        })
        .context("Action source is missing or is a nested action; select its device control")
}
fn stamp(slot: &mut Slot, origin: Option<&AssetSource>) {
    if slot.asset_source.is_none() {
        slot.asset_source = origin.cloned();
    }
    let origin = slot.asset_source.clone();
    for child in slot.children.iter_mut().flatten() {
        stamp(child, origin.as_ref());
    }
}
fn page_origin(profile: &library::Profile, page: &library::Page) -> Option<AssetSource> {
    page.asset_source.clone().or_else(|| {
        Some(AssetSource {
            source: profile.source.clone()?,
            manifest: page.source_manifest.clone()?,
            profile: profile.id.clone(),
        })
    })
}
fn folder_ids(slot: &Slot) -> Vec<String> {
    slot.flatten()
        .into_iter()
        .filter(|s| s.uuid == library::OPEN_FOLDER)
        .filter_map(|s| s.settings["ProfileUUID"].as_str())
        .map(profile::canonical_page_id)
        .collect()
}
fn capture(library: &Library, context: &str, cut: bool) -> Result<Clipboard> {
    let (p, g, s) = locate(library, context)?;
    let profile = &library.profiles[p];
    let page = &profile.pages[g];
    ensure!(
        !cut || !profile.readonly,
        "Cannot move an action from a read-only profile"
    );
    let mut slot = page.slots[s].clone();
    slot.sync_children_raw();
    stamp(&mut slot, page_origin(profile, page).as_ref());
    let anchor = Anchor {
        device: profile.device.clone(),
        profile: profile.id.clone(),
        page: page.id.clone(),
        context: slot.context.clone(),
        controller: slot.controller.clone(),
        index: slot.index,
        uuid: slot.uuid.clone(),
    };
    let mut pending = folder_ids(&slot);
    let mut seen = HashSet::new();
    let mut pages = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        ensure!(
            id != profile.pin_page && !profile.page_order.contains(&id),
            "Folder points to a top-level page"
        );
        let original = profile
            .pages
            .iter()
            .find(|p| p.id == id)
            .context("Folder page is missing")?;
        let mut page = original.clone();
        page.asset_source = page_origin(profile, original);
        for slot in &mut page.slots {
            slot.sync_children_raw();
            stamp(slot, page.asset_source.as_ref());
            pending.extend(folder_ids(slot));
        }
        pages.push(page);
    }
    Ok(Clipboard {
        anchor,
        slot,
        pages,
        cut,
    })
}
fn compatible(slot: &Slot, controller: &str, actions: &[Action]) -> Result<()> {
    ensure!(
        actions
            .iter()
            .find(|a| a.uuid == slot.uuid)
            .map_or(slot.controller == controller, |a| a
                .controllers
                .iter()
                .any(|c| c == controller)),
        "Action does not support the target controller"
    );
    Ok(())
}
fn position(slot: &mut Slot, controller: &str, index: u8) {
    slot.controller = controller.into();
    slot.index = index;
    slot.pinned = false;
    fn children(slot: &mut Slot, index: u8) {
        for child in slot.children.iter_mut().flatten() {
            child.index = index;
            children(child, index);
        }
    }
    children(slot, index);
}
fn remap_slot(slot: &mut Slot, mapping: &HashMap<String, String>) {
    library::remap_page_references(&mut slot.settings, mapping);
    library::remap_page_references(&mut slot.raw, mapping);
    for child in slot.children.iter_mut().flatten() {
        remap_slot(child, mapping);
    }
    slot.sync_children_raw();
}
fn materialize(
    copy: &Clipboard,
    clone_pages: bool,
    actions: &[Action],
) -> Result<(Slot, Vec<library::Page>)> {
    let mut slot = copy.slot.clone();
    let mut pages = if clone_pages {
        copy.pages.clone()
    } else {
        vec![]
    };
    let mapping: HashMap<_, _> = pages
        .iter()
        .map(|p| (p.id.clone(), library::new_id()))
        .collect();
    remap_slot(&mut slot, &mapping);
    library::duplicate_slot_identity(&mut slot, actions)?;
    for page in &mut pages {
        page.id = mapping[&page.id].clone();
        page.manifest_path = library::page_path(&page.id);
        library::remap_page_references(&mut page.raw, &mapping);
        for child in &mut page.slots {
            remap_slot(child, &mapping);
            library::duplicate_slot_identity(child, actions)?;
        }
    }
    Ok((slot, pages))
}
fn stale(original: &Anchor, current: &Anchor) -> bool {
    original.profile != current.profile
        || original.page != current.page
        || original.context != current.context
        || original.controller != current.controller
        || original.index != current.index
        || original.uuid != current.uuid
}
/// Build a complete replacement graph before any lifecycle or persistent mutation.
fn transferred(
    library: &Library,
    clipboard: &Clipboard,
    controller: &str,
    index: u8,
    expected: &Value,
    actions: &[Action],
) -> Result<Library> {
    ensure!(
        device::valid_slot(library.current()?.model, controller, index),
        "Invalid target control"
    );
    ensure!(
        expected.is_null() || expected.is_string(),
        "Expected target must be a context or null"
    );
    let current = library.current()?;
    ensure!(!current.readonly, "Cannot edit a read-only profile");
    let visible = library.visible()?;
    let target = visible
        .iter()
        .find(|s| s.controller == controller && s.index == index);
    ensure!(
        target.map(|s| s.context.as_str()) == expected.as_str(),
        "Target action changed; select the destination again"
    );
    let source = if clipboard.cut {
        let latest = capture(library, &clipboard.anchor.context, true)?;
        ensure!(
            !stale(&clipboard.anchor, &latest.anchor),
            "Cut source changed; cut the action again"
        );
        latest
    } else {
        clipboard.clone()
    };
    compatible(&source.slot, controller, actions)?;
    ensure!(
        source.pages.is_empty() || source.slot.model.profile_compatible(current.model),
        "Folder geometry must match the destination device model"
    );
    if source.cut && target.is_some_and(|s| s.context == source.anchor.context) {
        return Ok(library.clone());
    }
    let target_anchor = target.map(|s| locate(library, &s.context)).transpose()?;
    let dp = library
        .profiles
        .iter()
        .position(|p| p.id == library.active_profile)
        .unwrap();
    let dg = target_anchor.map(|(_, g, _)| g).unwrap_or_else(|| {
        library.profiles[dp]
            .pages
            .iter()
            .position(|p| p.id == library.active_page)
            .unwrap()
    });
    let dest_page = &library.profiles[dp].pages[dg].id;
    if source.cut && source.anchor.profile == library.active_profile {
        ensure!(
            !source.pages.iter().any(|p| p.id == *dest_page),
            "Cannot move a folder inside itself"
        );
    }
    let swapped = if source.cut {
        target
            .map(|s| capture(library, &s.context, true))
            .transpose()?
    } else {
        None
    };
    if let Some(other) = &swapped {
        compatible(&other.slot, &source.anchor.controller, actions)?;
        ensure!(
            other.pages.is_empty() || other.slot.model.profile_compatible(source.slot.model),
            "Folder geometry must match the destination device model"
        );
        if other.anchor.profile == source.anchor.profile {
            ensure!(
                !other.pages.iter().any(|p| p.id == source.anchor.page),
                "Cannot swap a folder inside itself"
            );
        }
    }
    let mut next = library.clone();
    let (mut incoming, pages) = materialize(
        &source,
        !source.cut || source.anchor.profile != library.active_profile,
        actions,
    )?;
    position(&mut incoming, controller, index);
    library::set_slot_model(&mut incoming, current.model);
    if source.cut {
        next.remove(&source.anchor.context);
    }
    if let Some(target) = target {
        next.remove(&target.context);
    }
    next.profiles[dp].pages[dg].slots.push(incoming);
    next.profiles[dp].pages.extend(pages);
    if let Some(other) = swapped {
        let sp = next
            .profiles
            .iter()
            .position(|p| p.id == source.anchor.profile)
            .context("Source profile missing")?;
        let sg = next.profiles[sp]
            .pages
            .iter()
            .position(|p| p.id == source.anchor.page)
            .context("Source page missing")?;
        let (mut outgoing, pages) = materialize(
            &other,
            other.anchor.profile != source.anchor.profile,
            actions,
        )?;
        position(
            &mut outgoing,
            &source.anchor.controller,
            source.anchor.index,
        );
        library::set_slot_model(&mut outgoing, next.profiles[sp].model);
        next.profiles[sp].pages[sg].slots.push(outgoing);
        next.profiles[sp].pages.extend(pages);
    }
    next.validate()?;
    Ok(next)
}
impl Host {
    pub(super) fn clear_device_clipboard(&mut self, device: &str) {
        if self
            .clipboard
            .as_ref()
            .is_some_and(|c| c.anchor.device == device)
        {
            self.clipboard = None;
        }
    }
    pub(super) fn asset_origin(&self, slot: &Slot) -> Option<AssetSource> {
        slot.asset_source.clone().or_else(|| {
            self.library.profiles.iter().find_map(|p| {
                p.pages
                    .iter()
                    .find(|g| g.slots.iter().any(|s| s.find(&slot.context).is_some()))
                    .and_then(|g| page_origin(p, g))
            })
        })
    }
    pub(super) fn retained_asset_sources(&self) -> Vec<&Path> {
        let mut paths = Vec::new();
        for profile in &self.library.profiles {
            if let Some(source) = &profile.source {
                paths.push(source.as_path());
            }
            for page in &profile.pages {
                if let Some(origin) = &page.asset_source {
                    paths.push(origin.source.as_path());
                }
                for slot in page.slots.iter().flat_map(Slot::flatten) {
                    paths.extend(
                        slot.appearances
                            .values()
                            .flat_map(|a| a.user_image.iter().chain(a.plugin_animation_source.iter()).chain(a.encoder_animation_source.iter())
                                .chain(a.hardware.iter().filter_map(|h| h.animation_source.as_ref())))
                            .map(PathBuf::as_path),
                    );
                    if let Some(origin) = &slot.asset_source {
                        paths.push(origin.source.as_path());
                    }
                }
            }
        }
        if let Some(copy) = &self.clipboard {
            for slot in copy.slot.flatten().into_iter().chain(
                copy.pages
                    .iter()
                    .flat_map(|p| p.slots.iter().flat_map(Slot::flatten)),
            ) {
                paths.extend(
                    slot.appearances
                        .values()
                        .flat_map(|a| a.user_image.iter().chain(a.plugin_animation_source.iter()).chain(a.encoder_animation_source.iter())
                            .chain(a.hardware.iter().filter_map(|h| h.animation_source.as_ref())))
                        .map(PathBuf::as_path),
                );
                if let Some(origin) = &slot.asset_source {
                    paths.push(origin.source.as_path());
                }
            }
            for page in &copy.pages {
                if let Some(origin) = &page.asset_source {
                    paths.push(origin.source.as_path());
                }
            }
        }
        paths
    }
    pub(super) fn clipboard_snapshot(&self) -> Value {
        self.clipboard.as_ref().map_or(Value::Null,|c|json!({"name":c.slot.name,"uuid":c.slot.uuid,"controller":c.slot.controller,"mode":if c.cut {"cut"}else{"copy"},"context":c.anchor.context,"profile":c.anchor.profile,"page":c.anchor.page,"device":self.library.profiles.iter().find(|p|p.id==c.anchor.profile).map(|p|p.device.as_str()).unwrap_or(&c.anchor.device)}))
    }
    pub(super) fn transfer_command(&mut self, request: &Value) -> Result<()> {
        let command = request["command"]
            .as_str()
            .context("Missing transfer command")?;
        if command == "slot_clipboard_clear" {
            self.clipboard = None;
            return Ok(());
        }
        self.sync_all_devices();
        if matches!(command, "slot_copy" | "slot_cut") {
            self.clipboard = Some(capture(
                &self.library,
                request["context"]
                    .as_str()
                    .context("Missing source context")?,
                command == "slot_cut",
            )?);
            return Ok(());
        }
        let source = if command == "slot_transfer" {
            capture(
                &self.library,
                request["context"]
                    .as_str()
                    .context("Missing source context")?,
                !request["copy"]
                    .as_bool()
                    .context("Copy must be true or false")?,
            )?
        } else {
            self.clipboard
                .clone()
                .context("Action clipboard is empty")?
        };
        let expected = request
            .get("expected_target")
            .context("Expected target is required")?;
        let controller = request["controller"]
            .as_str()
            .context("Target controller missing")?;
        let index = u8::try_from(request["index"].as_u64().context("Target index missing")?)?;
        let next = transferred(
            &self.library,
            &source,
            controller,
            index,
            expected,
            &self.actions,
        )?;
        // During graph replacement the canonical library is authoritative.
        // Old visible projections remain only long enough to emit their scoped
        // releases/disappearances; they must never overwrite the new graph.
        let source_device = self
            .library
            .profiles
            .iter()
            .find(|p| p.id == source.anchor.profile)
            .map(|p| p.device.clone());
        let target_device = self.device.clone();
        let old_library = std::mem::replace(&mut self.library, next);
        let old_slots = std::mem::replace(&mut self.slots, self.library.visible()?);
        let previous_mode = std::mem::replace(&mut self.canonical_graph_update, true);
        if let Err(error) = self.persist() {
            self.library = old_library;
            self.slots = old_slots;
            self.canonical_graph_update = previous_mode;
            return Err(error);
        }
        self.slots = old_slots;
        if command == "slot_paste" && source.cut {
            self.clipboard = None;
        }
        let mut devices = vec![target_device];
        if source.cut
            && let Some(device) = source_device
            && !devices.contains(&device)
        {
            devices.push(device);
        }
        let mut failure = None;
        for device in devices {
            if let Err(error) = self.with_device(&device, Host::refresh_page) {
                failure.get_or_insert(error);
            }
        }
        self.canonical_graph_update = previous_mode;
        self.persist()?;
        if let Some(error) = failure {
            Err(error)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Host) {
        let dir = tempfile::tempdir().unwrap();
        let host = super::super::tests::fixture(dir.path());
        (dir, host)
    }
    #[test]
    fn occupied_cross_model_swap_updates_both_sdk_coordinate_systems() {
        let (_dir, mut host) = setup();
        let a = host.device.clone();
        host.assign("Keypad", 5, "test.action").unwrap();
        let source = host
            .slots
            .iter()
            .find(|s| s.index == 5)
            .unwrap()
            .context
            .clone();
        host.add_simulated_device("Mini-B", Model::Mini).unwrap();
        let target = host
            .with_device("Mini-B", |host| {
                host.assign("Keypad", 5, "test.action")?;
                Ok(host.slots[0].context.clone())
            })
            .unwrap();
        super::super::apply_control(&mut host,json!({"command":"slot_transfer","device":"Mini-B","context":source,"controller":"Keypad","index":5,"copy":false,"expected_target":target})).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        for device in [&a, "Mini-B"] {
            host.with_device(device, |host| {
                host.input(InputEvent::Key {
                    index: 5,
                    pressed: true,
                })
            })
            .unwrap();
        }
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|event| event["event"] == "keyDown")
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["device"], a);
        assert_eq!(
            events[0]["payload"]["coordinates"],
            json!({"column":1,"row":1})
        );
        assert_eq!(events[1]["device"], "Mini-B");
        assert_eq!(
            events[1]["payload"]["coordinates"],
            json!({"column":2,"row":1})
        );
    }

    #[test]
    fn cross_device_cut_releases_only_source_and_persists_both_projections() {
        let (_dir, mut host) = setup();
        let a = host.device.clone();
        let source = host.slots[0].context.clone();
        host.add_simulated_device("B", Model::Plus).unwrap();
        let b = host
            .with_device("B", |host| {
                host.assign("Keypad", 0, "test.action")?;
                Ok(host.slots[0].context.clone())
            })
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.with_device("B", |host| {
            host.input(InputEvent::Key {
                index: 0,
                pressed: true,
            })
        })
        .unwrap();
        while rx.try_recv().is_ok() {}
        super::super::apply_control(
            &mut host,
            json!({"command":"slot_cut","device":a,"context":source}),
        )
        .unwrap();
        assert_eq!(host.clipboard_snapshot()["device"], a);
        super::super::apply_control(&mut host,json!({"command":"slot_paste","device":"B","controller":"Keypad","index":1,"expected_target":null})).unwrap();
        assert_eq!(host.device, a);
        assert!(host.slots.is_empty());
        let pasted = host
            .with_device("B", |host| {
                assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&b));
                assert_eq!(
                    host.slots
                        .iter()
                        .find(|slot| slot.index == 0)
                        .unwrap()
                        .context,
                    b
                );
                Ok(host
                    .slots
                    .iter()
                    .find(|slot| slot.index == 1)
                    .unwrap()
                    .context
                    .clone())
            })
            .unwrap();
        assert_ne!(pasted, source);
        assert!(host.library.slot(&source).is_none());
        let mut events = vec![];
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        assert!(events.iter().any(|event| event["event"] == "keyUp"
            && event["device"] == a
            && event["context"] == source));
        assert!(
            !events
                .iter()
                .any(|event| event["event"] == "keyUp" && event["device"] == "B")
        );
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.library.as_ref().unwrap().slot(&source).is_none());
        assert!(saved.library.as_ref().unwrap().slot(&pasted).is_some());
    }
    #[test]
    fn cross_device_failed_save_preserves_source_destination_and_cut_for_retry() {
        let (dir, mut host) = setup();
        let a = host.device.clone();
        let source = host.slots[0].context.clone();
        host.add_simulated_device("B", Model::Mini).unwrap();
        host.with_device("B", |host| host.assign("Keypad", 0, "test.action"))
            .unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        super::super::apply_control(
            &mut host,
            json!({"command":"slot_cut","device":a,"context":source}),
        )
        .unwrap();
        let original_path = host.state_path.clone();
        host.state_path = dir.path().join("missing/session.json");
        let paste = json!({"command":"slot_paste","device":"B","controller":"Keypad","index":1,"expected_target":null});
        assert!(super::super::apply_control(&mut host, paste.clone()).is_err());
        assert_eq!(host.device, a);
        assert!(!host.canonical_graph_update);
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert_eq!(host.slots[0].context, source);
        assert!(host.clipboard.as_ref().unwrap().cut);
        host.state_path = original_path;
        super::super::apply_control(&mut host, paste).unwrap();
        assert!(host.slots.is_empty());
        assert!(host.clipboard.is_none());
        assert_eq!(
            host.with_device("B", |host| Ok(host.slots.len())).unwrap(),
            2
        );
    }

    #[test]
    fn cut_is_deferred_and_uses_latest_settings_with_stale_target_guard() {
        let (_dir, mut host) = setup();
        let source = host.slots[0].context.clone();
        host.transfer_command(&json!({"command":"slot_cut","context":source}))
            .unwrap();
        host.slots[0].settings = json!({"latest":42});
        host.sync_visible();
        let copy = host.clipboard.as_ref().unwrap();
        assert!(
            transferred(
                &host.library,
                copy,
                "Keypad",
                1,
                &json!("stale"),
                &host.actions
            )
            .is_err()
        );
        assert!(host.library.slot(&source).is_some());
        let next = transferred(
            &host.library,
            copy,
            "Keypad",
            1,
            &Value::Null,
            &host.actions,
        )
        .unwrap();
        assert!(next.slot(&source).is_none());
        let moved = next
            .visible()
            .unwrap()
            .into_iter()
            .find(|s| s.index == 1)
            .unwrap();
        assert_eq!(moved.settings, json!({"latest":42}));
        assert_ne!(moved.context, source);
    }
    #[test]
    fn pinned_move_unpins_and_swap_uses_actual_ownership_without_aliases() {
        let (_dir, mut host) = setup();
        host.sync_visible();
        let source = host.slots[0].clone();
        host.library.remove(&source.context);
        let profile = host.library.current_mut().unwrap();
        profile
            .pages
            .iter_mut()
            .find(|p| p.id == profile.pin_page)
            .unwrap()
            .slots
            .push(source.clone());
        let copy = capture(&host.library, &source.context, true).unwrap();
        let next = transferred(
            &host.library,
            &copy,
            "Keypad",
            1,
            &Value::Null,
            &host.actions,
        )
        .unwrap();
        assert!(
            next.current()
                .unwrap()
                .pages
                .iter()
                .find(|p| p.id == next.current().unwrap().pin_page)
                .unwrap()
                .slots
                .is_empty()
        );
        assert!(!next.visible().unwrap()[0].pinned);
        let mut target = source.clone();
        target.context = "target".into();
        target.index = 2;
        target.settings = json!({"target":true});
        let active = host.library.active_page.clone();
        host.library
            .current_mut()
            .unwrap()
            .pages
            .iter_mut()
            .find(|p| p.id == active)
            .unwrap()
            .slots
            .push(target.clone());
        let next = transferred(
            &host.library,
            &copy,
            "Keypad",
            2,
            &json!(target.context),
            &host.actions,
        )
        .unwrap();
        assert!(next.slot(&source.context).is_none());
        assert!(next.slot(&target.context).is_none());
        let visible = next.visible().unwrap();
        assert!(visible.iter().find(|s| s.index == 0).unwrap().pinned);
        assert_eq!(
            visible.iter().find(|s| s.index == 0).unwrap().settings,
            json!({"target":true})
        );
        assert!(!visible.iter().find(|s| s.index == 2).unwrap().pinned);
    }
    #[test]
    fn copied_folder_is_independent_and_cannot_move_into_itself() {
        let (_dir, mut host) = setup();
        host.library_command(&json!({"command":"folder_create","name":"Folder","index":1}))
            .unwrap();
        let folder = host.slots.iter().find(|s| s.index == 1).unwrap();
        let context = folder.context.clone();
        host.sync_visible();
        let copy = capture(&host.library, &context, false).unwrap();
        let next = transferred(
            &host.library,
            &copy,
            "Keypad",
            2,
            &Value::Null,
            &host.actions,
        )
        .unwrap();
        let other = next
            .visible()
            .unwrap()
            .into_iter()
            .find(|s| s.index == 2)
            .unwrap();
        assert_ne!(
            other.settings["ProfileUUID"],
            copy.slot.settings["ProfileUUID"]
        );
        assert_eq!(
            next.current().unwrap().pages.len(),
            host.library.current().unwrap().pages.len() + 1
        );
        let mut inside = host.library.clone();
        inside.active_page = copy.pages[0].id.clone();
        let cut = capture(&inside, &context, true).unwrap();
        assert!(transferred(&inside, &cut, "Keypad", 3, &Value::Null, &host.actions).is_err());
    }
    #[test]
    fn unknown_actions_stay_on_their_controller_and_explicit_resource_clear_survives() {
        let (_dir, mut host) = setup();
        host.slots[0].uuid = "unknown.action".into();
        host.slots[0].resources = Some(Default::default());
        host.sync_visible();
        let copy = capture(&host.library, &host.slots[0].context, false).unwrap();
        assert!(
            transferred(
                &host.library,
                &copy,
                "Encoder",
                0,
                &Value::Null,
                &host.actions
            )
            .is_err()
        );
        let next = transferred(
            &host.library,
            &copy,
            "Keypad",
            1,
            &Value::Null,
            &host.actions,
        )
        .unwrap();
        assert_eq!(
            next.visible()
                .unwrap()
                .iter()
                .find(|s| s.index == 1)
                .unwrap()
                .resources,
            Some(Default::default())
        );
    }
    #[test]
    fn failed_persistence_keeps_cut_source_and_clipboard() {
        let (dir, mut host) = setup();
        let context = host.slots[0].context.clone();
        host.transfer_command(&json!({"command":"slot_cut","context":context}))
            .unwrap();
        host.state_path = dir.path().join("missing-parent/state.json");
        assert!(host.transfer_command(&json!({"command":"slot_paste","controller":"Keypad","index":1,"expected_target":null})).is_err());
        assert_eq!(host.slots[0].context, context);
        assert_eq!(host.slots[0].index, 0);
        assert!(host.library.slot(&context).is_some());
        assert!(host.clipboard.is_some());
    }
    #[test]
    fn nested_copy_keeps_runtime_state_and_independent_identities() {
        let (_dir, mut host) = setup();
        host.actions.extend(multiaction::definitions());
        host.assign("Keypad", 1, multiaction::ROUTINE).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.index == 1)
            .unwrap()
            .context
            .clone();
        host.save_multi(&parent,json!({"Actions":[{"Actions":[{"UUID":"test.action","ActionID":"original","Settings":{"opaque":[1,2]},"States":[{},{}]}]},{"Actions":[]}]})).unwrap();
        let source = host.slots.iter_mut().find(|s| s.context == parent).unwrap();
        let child = &mut source.children[0][0];
        child.state = 1;
        child.image_changed = true;
        child.title_changed = true;
        child.title = "Runtime".into();
        child.feedback = json!({"value":73});
        child.background = Some(DynamicImage::new_rgba8(2, 2));
        child.resources = Some(Default::default());
        source.sync_children_raw();
        host.sync_visible();
        let copy = capture(&host.library, &parent, false).unwrap();
        let next = transferred(
            &host.library,
            &copy,
            "Keypad",
            2,
            &Value::Null,
            &host.actions,
        )
        .unwrap();
        let mut copied = next
            .visible()
            .unwrap()
            .into_iter()
            .find(|s| s.index == 2)
            .unwrap();
        copied.hydrate_children(&host.actions).unwrap();
        let old = &copy.slot.children[0][0];
        let new = &copied.children[0][0];
        assert_ne!(old.context, new.context);
        assert_ne!(old.raw["ActionID"], new.raw["ActionID"]);
        assert_eq!(new.state, 1);
        assert_eq!(new.index, 2);
        assert!(new.image_changed && new.title_changed);
        assert_eq!(new.title, "Runtime");
        assert_eq!(new.feedback, json!({"value":73}));
        assert!(new.background.is_some());
        assert_eq!(new.resources, Some(Default::default()));
    }
}
