//! Profile/page ownership and active-page projection. Contexts remain stable
//! while pages are hidden, so asynchronous plugin replies keep their owner.
use super::*;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;

pub(super) const OPEN_FOLDER: &str = "com.elgato.streamdeck.profile.openchild";
pub(super) const BACK_FOLDER: &str = "com.elgato.streamdeck.profile.backtoparent";
pub(super) fn navigation_actions() -> Vec<Action> {
    [
        ("com.elgato.streamdeck.page.next", "Next page"),
        ("com.elgato.streamdeck.page.previous", "Previous page"),
        ("com.elgato.streamdeck.page.goto", "Go to page"),
        ("com.elgato.streamdeck.page.indicator", "Page indicator"),
        (BACK_FOLDER, "Back from folder"),
    ]
    .into_iter()
    .map(|(uuid, name)| Action {
        uuid: uuid.into(),
        name: name.into(),
        controllers: vec!["Keypad".into()],
        plugin: "deckforge.navigation".into(),
        raw: json!({"States":[{"Title":name}]}),
    })
    .chain(std::iter::once(Action {
        uuid: OPEN_FOLDER.into(),
        name: "Open folder".into(),
        controllers: vec!["Keypad".into()],
        plugin: "deckforge.navigation".into(),
        raw: json!({"States":[{"Title":"Folder"}]}),
    }))
    .collect()
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Page {
    pub id: String,
    pub name: String,
    pub slots: Vec<Slot>,
    pub raw: Value,
    pub source_manifest: Option<String>,
    pub manifest_path: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Profile {
    pub id: String,
    pub name: String,
    pub device: String,
    pub pin_page: String,
    pub page_order: Vec<String>,
    pub pages: Vec<Page>,
    pub raw: Value,
    pub source: Option<PathBuf>,
    pub owner_plugin: Option<String>,
    #[serde(default)]
    pub readonly: bool,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub(super) struct Library {
    pub profiles: Vec<Profile>,
    pub active_profile: String,
    pub active_page: String,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub previous: Option<(String, String)>,
    #[serde(default)]
    pub cursors: HashMap<String, String>,
    #[serde(default)]
    pub folder_paths: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub bundles: HashMap<String, HashMap<String, Option<String>>>,
}
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn page_path(id: &str) -> String {
    format!(
        "Profiles/{}/manifest.json",
        profile::encoded_uuid(id).unwrap_or_else(|| id.into())
    )
}
fn page(name: &str) -> Page {
    let id = new_id();
    Page {
        id: id.clone(),
        name: name.into(),
        slots: vec![],
        raw: json!({"Controllers":[]}),
        source_manifest: None,
        manifest_path: page_path(&id),
    }
}
fn native_profile(name: &str, device: &str) -> Profile {
    let pins = page("Pinned actions");
    let first = page("Page 1");
    Profile {
        id: new_id(),
        name: name.into(),
        device: device.into(),
        pin_page: pins.id.clone(),
        page_order: vec![first.id.clone()],
        pages: vec![first, pins],
        raw: json!({"Name":name,"Version":"2.0"}),
        source: None,
        owner_plugin: None,
        readonly: false,
    }
}
fn valid_name(value: &Value) -> Result<String> {
    let name = value.as_str().context("Name must be text")?.trim();
    if name.is_empty() {
        bail!("Name cannot be empty");
    }
    Ok(name.into())
}

fn page_label(profile: &Profile, page: &Page) -> String {
    if !page.name.is_empty() {
        return page.name.clone();
    }
    if let Some(index) = profile.page_order.iter().position(|id| id == &page.id) {
        return format!("Page {}", index + 1);
    }
    for slot in profile.pages.iter().flat_map(|p| &p.slots) {
        if slot.uuid == OPEN_FOLDER
            && slot.settings["ProfileUUID"]
                .as_str()
                .is_some_and(|id| profile::canonical_page_id(id) == page.id)
            && !slot.title.is_empty()
        {
            return slot.title.clone();
        }
    }
    "Folder".into()
}
fn remap_page_references(value: &mut Value, mapping: &HashMap<String, String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if key == "ProfileUUID"
                    && let Some(target) = value
                        .as_str()
                        .and_then(|v| mapping.get(&profile::canonical_page_id(v)))
                {
                    *value = json!(target);
                } else {
                    remap_page_references(value, mapping);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                remap_page_references(value, mapping);
            }
        }
        _ => {}
    }
}
fn renew_action_ids(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if fields.contains_key("UUID") && fields.contains_key("ActionID") {
                fields.insert("ActionID".into(), json!(new_id()));
            }
            for value in fields.values_mut() {
                renew_action_ids(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                renew_action_ids(value);
            }
        }
        _ => {}
    }
}
impl Library {
    pub fn legacy(slots: Vec<Slot>, device: &str) -> Self {
        let mut profile = native_profile("Default profile", device);
        profile.pages[0].slots = slots;
        Self {
            active_profile: profile.id.clone(),
            active_page: profile.page_order[0].clone(),
            profiles: vec![profile],
            ..Default::default()
        }
    }
    pub fn validate(&self) -> Result<()> {
        let mut profiles = HashSet::new();
        let mut contexts = HashSet::new();
        for profile in &self.profiles {
            if !profiles.insert(&profile.id) {
                bail!("Duplicate profile identity");
            }
            let mut pages = HashSet::new();
            for page in &profile.pages {
                if !pages.insert(&page.id) {
                    bail!("Duplicate page identity");
                }
                let mut positions = HashSet::new();
                for slot in &page.slots {
                    for action in slot.flatten() {
                        if !contexts.insert(&action.context) {
                            bail!("Duplicate action context in profile library");
                        }
                    }
                    if !positions.insert((&slot.controller, slot.index)) {
                        bail!("Duplicate placement in page");
                    }
                }
            }
            if profile.page_order.is_empty()
                || profile.page_order.iter().any(|id| !pages.contains(id))
                || !pages.contains(&profile.pin_page)
            {
                bail!("Invalid profile page/default references");
            }
        }
        self.current()?;
        if !self
            .current()?
            .pages
            .iter()
            .any(|p| p.id == self.active_page)
        {
            bail!("Active page is missing");
        }
        Ok(())
    }
    pub fn current(&self) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile)
            .context("Active profile is missing")
    }
    pub fn current_mut(&mut self) -> Result<&mut Profile> {
        self.profiles
            .iter_mut()
            .find(|p| p.id == self.active_profile)
            .context("Active profile is missing")
    }
    pub fn slot(&self, context: &str) -> Option<&Slot> {
        self.profiles
            .iter()
            .flat_map(|p| &p.pages)
            .flat_map(|p| &p.slots)
            .find_map(|s| s.find(context))
    }
    pub fn update(&mut self, slot: &Slot) -> bool {
        for page in self.profiles.iter_mut().flat_map(|p| &mut p.pages) {
            if let Some(found) = page
                .slots
                .iter_mut()
                .find_map(|s| s.find_mut(&slot.context))
            {
                *found = slot.clone();
                found.pinned = false;
                return true;
            }
        }
        false
    }
    pub fn remove(&mut self, context: &str) {
        for page in self.profiles.iter_mut().flat_map(|p| &mut p.pages) {
            page.slots.retain(|s| s.context != context);
        }
    }
    pub fn place(&mut self, slot: Slot) -> Result<()> {
        let active = self.active_page.clone();
        let profile = self.current_mut()?;
        let page = profile
            .pages
            .iter_mut()
            .find(|p| p.id == active)
            .context("Page missing")?;
        page.slots
            .retain(|s| !(s.controller == slot.controller && s.index == slot.index));
        page.slots.push(slot);
        Ok(())
    }
    pub fn visible(&self) -> Result<Vec<Slot>> {
        let profile = self.current()?;
        let mut slots = BTreeMap::new();
        for page_id in [&profile.pin_page, &self.active_page] {
            let page = profile
                .pages
                .iter()
                .find(|p| &p.id == page_id)
                .context("Page missing")?;
            for slot in &page.slots {
                let mut slot = slot.clone();
                slot.pinned = page_id == &profile.pin_page;
                slots.insert((slot.controller.clone(), slot.index), slot);
            }
        }
        Ok(slots.into_values().collect())
    }
    pub fn snapshot(&self) -> Value {
        json!({"active_profile":self.active_profile,"active_page":self.active_page,"folder_depth":self.folders.len(),
            "profiles":self.profiles.iter().map(|p|json!({"id":p.id,"name":p.name,"device":p.device,"readonly":p.readonly,"pin_page":p.pin_page,"page_order":p.page_order,
                "pages":p.page_order.iter().filter_map(|id|p.pages.iter().find(|page| &page.id==id)).chain(p.pages.iter().filter(|page|page.id!=p.pin_page&&!p.page_order.contains(&page.id))).map(|page|json!({"id":page.id,"name":page_label(p,page),"folder":!p.page_order.contains(&page.id)})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }
}

impl Host {
    pub(super) fn resolve_state_image(&mut self, slot: &Slot, state: u64) -> Option<DynamicImage> {
        let action = self.actions.iter().find(|a| a.uuid == slot.uuid).cloned();
        let plugin = action
            .as_ref()
            .and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin))
            .map(|p| p.root.clone());
        if let Some(image) = slot
            .states
            .get(state as usize)
            .and_then(|s| s["Image"].as_str())
            .filter(|s| !s.is_empty())
        {
            let source = self.library.profiles.iter().find_map(|profile| {
                profile
                    .pages
                    .iter()
                    .find(|page| page.slots.iter().any(|s| s.find(&slot.context).is_some()))
                    .and_then(|page| Some((profile.source.clone()?, page.source_manifest.clone()?)))
            });
            if let Some((path, manifest)) = source {
                if !self.profile_assets.contains_key(&path)
                    && let Ok(bundle) = profile::load_bundle(&path)
                {
                    self.profile_assets.insert(path.clone(), bundle);
                }
                if let Some(bundle) = self.profile_assets.get(&path)
                    && let Some(page) = bundle.pages.iter().find(|p| p.manifest_path() == manifest)
                    && let Ok(bytes) = page.asset(image)
                    && let Ok(decoded) = display::decode_image(&bytes, image.ends_with(".svg"))
                {
                    return Some(decoded);
                }
            }
            return plugin
                .as_ref()
                .and_then(|root| load_image(root, image).ok());
        }
        action.and_then(|action| {
            plugin.and_then(|root| state_image(&root, &action.raw["States"], state))
        })
    }
    pub(super) fn sync_visible(&mut self) {
        for slot in &self.slots {
            self.library.update(slot);
        }
    }
    pub(super) fn editable(&self) -> Result<()> {
        if self.library.current()?.readonly {
            bail!("This bundled profile is read-only; duplicate it to edit");
        }
        Ok(())
    }
    pub(super) fn refresh_page(&mut self) -> Result<()> {
        self.hydrate_library_children();
        let mut next = self.library.visible()?;
        let profile = self.library.current()?;
        let page_index = profile
            .page_order
            .iter()
            .position(|id| id == &self.library.active_page)
            .map(|i| i + 1);
        for slot in &mut next {
            if slot.uuid == "com.elgato.streamdeck.page.indicator" {
                slot.title = page_index
                    .map(|i| format!("{i} / {}", profile.page_order.len()))
                    .unwrap_or_else(|| "Folder".into());
            }
        }
        let old: HashSet<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .map(|s| s.context.clone())
            .collect();
        let new: HashSet<_> = next
            .iter()
            .flat_map(Slot::visible_flatten)
            .map(|s| s.context.clone())
            .collect();
        let removed: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::flatten)
            .filter(|slot| {
                !new.contains(&slot.context) && self.library.slot(&slot.context).is_none()
            })
            .map(|slot| (slot.context.clone(), slot.uuid.clone()))
            .collect();
        for (context, action) in removed {
            self.remember_removed(&context, &action);
        }
        self.cancel_changed_triggers(&next);
        self.cancel_departing_triggers(&new);
        self.cancel_departing_stacks(&new);
        self.cancel_departing_multi(&new);
        self.cancel_departing_inputs(&new);
        for context in old.difference(&new) {
            self.close_inspectors(Some(context));
        }
        for slot in self.slots.iter().flat_map(Slot::visible_flatten) {
            if !new.contains(&slot.context) {
                self.emit(slot, "willDisappear", json!({}));
            }
        }
        self.slots = next;
        for index in 0..self.slots.len() {
            if self.slots[index].background.is_none() {
                let slot = self.slots[index].clone();
                self.slots[index].background = self.resolve_state_image(&slot, slot.state);
            }
        }
        for slot in self.slots.iter().flat_map(Slot::visible_flatten) {
            if !old.contains(&slot.context) {
                self.emit(slot, "willAppear", json!({}));
            }
        }
        self.library.cursors.insert(
            self.library.active_profile.clone(),
            self.library.active_page.clone(),
        );
        self.library.folder_paths.insert(
            self.library.active_profile.clone(),
            self.library.folders.clone(),
        );
        self.render()?;
        self.persist()
    }
    pub(super) fn select_profile(&mut self, id: &str, page: Option<&str>) -> Result<()> {
        self.sync_visible();
        let target = self
            .library
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("Profile not found")?;
        let selected = page
            .map(str::to_owned)
            .or_else(|| self.library.cursors.get(id).cloned())
            .unwrap_or_else(|| target.page_order[0].clone());
        if !target.pages.iter().any(|p| p.id == selected) {
            bail!("Page not found in destination profile");
        }
        if self.library.active_profile != id {
            self.library.previous = Some((
                self.library.active_profile.clone(),
                self.library.active_page.clone(),
            ));
        }
        let folders = if page.is_none() {
            self.library
                .folder_paths
                .get(id)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.library.active_profile = id.into();
        self.library.active_page = selected;
        self.library.folders = folders;
        self.refresh_page()
    }
    pub(super) fn select_page(&mut self, id: &str, folder: bool) -> Result<()> {
        self.sync_visible();
        let id = profile::canonical_page_id(id);
        if !self.library.current()?.pages.iter().any(|p| p.id == id) {
            bail!("Page not found");
        }
        if folder {
            if self.library.folders.len() >= 64 {
                bail!("Folder nesting limit reached");
            }
            self.library.folders.push(self.library.active_page.clone());
        } else {
            self.library.folders.clear();
        }
        self.library.active_page = id;
        self.refresh_page()
    }
    pub(super) fn step_page(&mut self, direction: i64) -> Result<()> {
        let profile = self.library.current()?;
        let base = self
            .library
            .folders
            .first()
            .unwrap_or(&self.library.active_page);
        let position = profile
            .page_order
            .iter()
            .position(|p| p == base)
            .unwrap_or(0);
        let next = (position as i64 + direction.signum())
            .clamp(0, profile.page_order.len() as i64 - 1) as usize;
        let id = profile.page_order[next].clone();
        self.select_page(&id, false)
    }
    pub(super) fn goto_page(&mut self, settings: &Value) -> Result<()> {
        // Elgato saves PageIndex as one-based; folder and pinned pages are
        // excluded from the numbered top-level page order.
        let number = match settings.get("PageIndex") {
            None => 1,
            Some(value) => value
                .as_u64()
                .context("PageIndex must be a positive integer")?,
        };
        let index = number
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .context("PageIndex must be a positive integer")?;
        let id = self
            .library
            .current()?
            .page_order
            .get(index)
            .cloned()
            .context("PageIndex is outside the current profile's pages")?;
        self.select_page(&id, false)
    }
    pub(super) fn folder_back(&mut self) -> Result<()> {
        self.sync_visible();
        if let Some(id) = self.library.folders.pop() {
            self.library.active_page = id;
            self.refresh_page()?;
        }
        Ok(())
    }
    pub(super) fn import_library(
        &mut self,
        path: &Path,
        owner: Option<&str>,
        readonly: bool,
    ) -> Result<String> {
        self.import_bundle(profile::load_bundle(path)?, owner, readonly)
    }
    fn import_bundle(
        &mut self,
        bundle: profile::ProfileBundle,
        owner: Option<&str>,
        readonly: bool,
    ) -> Result<String> {
        let id = new_id();
        let directory = self
            .state_path
            .parent()
            .context("State directory missing")?
            .join("sources");
        fs::create_dir_all(&directory)?;
        let source = directory.join(format!("{id}.streamDeckProfile"));
        let temporary = tempfile::NamedTempFile::new_in(&directory)?;
        {
            let mut writer = zip::ZipWriter::new(temporary.as_file());
            for (name, bytes) in bundle.files() {
                writer.start_file(
                    name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )?;
                writer.write_all(bytes)?;
            }
            writer.finish()?;
        }
        temporary.persist_noclobber(&source).map_err(|e| e.error)?;
        let root = bundle
            .manifest_path()
            .trim_end_matches("manifest.json")
            .to_owned();
        let mut pages = Vec::new();
        for imported in &bundle.pages {
            let page_id = if imported.page.is_empty() {
                new_id()
            } else {
                imported.page.clone()
            };
            let mut slots = Vec::new();
            for item in &imported.slots {
                if !self.actions.iter().any(|a| a.uuid == item.uuid) {
                    self.log(format!(
                        "Imported action {} is retained but unavailable",
                        item.uuid
                    ));
                }
                if !((item.controller == "Keypad" && item.column < 4 && item.row < 2)
                    || (item.controller == "Encoder" && item.column < 4 && item.row == 0))
                {
                    continue;
                }
                let state = item.raw["State"].as_u64().unwrap_or(0);
                let name = item.raw["Name"].as_str().unwrap_or(&item.uuid).to_owned();
                let background = item.states[state as usize]["Image"]
                    .as_str()
                    .and_then(|path| bundle.asset(&imported.page, path).ok())
                    .and_then(|bytes| {
                        display::decode_image(
                            &bytes,
                            bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml"),
                        )
                        .ok()
                    });
                let layout = self
                    .actions
                    .iter()
                    .find(|a| a.uuid == item.uuid)
                    .and_then(|a| a.raw["Encoder"]["layout"].as_str())
                    .unwrap_or("$B1")
                    .to_owned();
                slots.push(Slot {
                    children: vec![],
                    child_id: String::new(),
                    in_multi_action: false,
                    context: new_id(),
                    controller: item.controller.clone(),
                    index: if item.controller == "Encoder" {
                        item.column
                    } else {
                        item.row * 4 + item.column
                    },
                    uuid: item.uuid.clone(),
                    name: name.clone(),
                    settings: item.settings.clone(),
                    title: item.states[state as usize]["Title"]
                        .as_str()
                        .unwrap_or_else(|| {
                            self.actions
                                .iter()
                                .find(|a| a.uuid == item.uuid)
                                .and_then(|a| a.raw["States"][state as usize]["Title"].as_str())
                                .unwrap_or(if self.actions.iter().any(|a| a.uuid == item.uuid) {
                                    ""
                                } else {
                                    &name
                                })
                        })
                        .into(),
                    state,
                    feedback: json!({}),
                    layout,
                    states: item.states.clone(),
                    background,
                    raw: item.raw.clone(),
                    image_changed: false,
                    title_changed: false,
                    pinned: false,
                    layout_cache: None,
                    image_cache: HashMap::new(),
                    display_warnings: vec![],
                    preview: String::new(),
                });
            }
            let original = imported.manifest_path();
            let target = if original == bundle.manifest_path() {
                page_path(&page_id)
            } else {
                original.strip_prefix(&root).unwrap_or(&original).to_owned()
            };
            pages.push(Page {
                id: page_id.clone(),
                name: imported.page_raw["Name"].as_str().unwrap_or("").into(),
                slots,
                raw: imported.page_raw.clone(),
                source_manifest: Some(original),
                manifest_path: target,
            });
        }
        let mut order = bundle.page_order.clone();
        if order.is_empty() {
            order.push(
                pages
                    .iter()
                    .find(|p| p.id == bundle.current_page)
                    .unwrap_or(&pages[0])
                    .id
                    .clone(),
            );
        }
        let pin = if pages.iter().any(|p| p.id == bundle.default_page)
            && !order.contains(&bundle.default_page)
        {
            bundle.default_page.clone()
        } else {
            let pins = page("Pinned actions");
            let id = pins.id.clone();
            pages.push(pins);
            id
        };
        let current = if pages.iter().any(|p| p.id == bundle.current_page) {
            bundle.current_page.clone()
        } else {
            order[0].clone()
        };
        self.library.profiles.push(Profile {
            id: id.clone(),
            name: bundle.name.clone(),
            device: self.device.clone(),
            pin_page: pin,
            page_order: order,
            pages,
            raw: bundle.raw.clone(),
            source: Some(source),
            owner_plugin: owner.map(str::to_owned),
            readonly,
        });
        self.library.cursors.insert(id.clone(), current);
        for warning in bundle.warnings {
            self.log(warning);
        }
        Ok(id)
    }

    pub(super) fn library_command(&mut self, r: &Value) -> Result<()> {
        self.sync_visible();
        let command = r["command"].as_str().unwrap_or("");
        let id = r["id"].as_str().unwrap_or("");
        match command {
            "profile_create" => {
                let p = native_profile(&valid_name(&r["name"])?, &self.device);
                let id = p.id.clone();
                self.library.profiles.push(p);
                return self.select_profile(&id, None);
            }
            "profile_select" => return self.select_profile(id, None),
            "profile_rename" => {
                let name = valid_name(&r["name"])?;
                self.library
                    .profiles
                    .iter_mut()
                    .find(|p| p.id == id)
                    .context("Profile missing")?
                    .name = name;
            }
            "profile_duplicate" => {
                let mut p = self
                    .library
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .context("Profile missing")?
                    .clone();
                p.id = new_id();
                p.name = valid_name(&r["name"])?;
                p.owner_plugin = None;
                p.readonly = false;
                let map: HashMap<_, _> = p.pages.iter().map(|p| (p.id.clone(), new_id())).collect();
                for page in &mut p.pages {
                    page.id = map[&page.id].clone();
                    page.manifest_path = page_path(&page.id);
                    remap_page_references(&mut page.raw, &map);
                    renew_action_ids(&mut page.raw);
                    for slot in &mut page.slots {
                        slot.children.clear();
                        slot.context = new_id();
                        renew_action_ids(&mut slot.raw);
                        if slot.raw.is_object() {
                            slot.raw["ActionID"] = json!(slot.context);
                        }
                        remap_page_references(&mut slot.settings, &map);
                        remap_page_references(&mut slot.raw, &map);
                    }
                }
                p.pin_page = map[&p.pin_page].clone();
                p.page_order = p.page_order.iter().map(|id| map[id].clone()).collect();
                let id = p.id.clone();
                self.library.profiles.push(p);
                return self.select_profile(&id, None);
            }
            "profile_delete" => {
                if self.library.profiles.len() == 1 {
                    bail!("Create another profile before deleting the last one");
                }
                if !self.library.profiles.iter().any(|p| p.id == id) {
                    bail!("Profile missing");
                }
                self.library.profiles.retain(|p| p.id != id);
                self.library.cursors.remove(id);
                self.library.folder_paths.remove(id);
                if self
                    .library
                    .previous
                    .as_ref()
                    .is_some_and(|(profile, _)| profile == id)
                {
                    self.library.previous = None;
                }
                for bindings in self.library.bundles.values_mut() {
                    for target in bindings.values_mut() {
                        if target.as_deref() == Some(id) {
                            *target = None;
                        }
                    }
                }
                if self.library.active_profile == id {
                    let next = self.library.profiles[0].id.clone();
                    self.select_profile(&next, None)?;
                    self.library.previous = None;
                    return self.persist();
                }
            }
            "page_select" => {
                let folder = !self.library.current()?.page_order.iter().any(|p| p == id);
                return self.select_page(id, folder);
            }
            "page_step" => {
                return self.step_page(r["direction"].as_i64().context("Direction missing")?);
            }
            "folder_back" => return self.folder_back(),
            "page_create" => {
                self.editable()?;
                let p = page(&valid_name(&r["name"])?);
                let id = p.id.clone();
                let profile = self.library.current_mut()?;
                profile.page_order.push(id.clone());
                profile.pages.push(p);
                return self.select_page(&id, false);
            }
            "page_rename" => {
                self.editable()?;
                let name = valid_name(&r["name"])?;
                self.library
                    .current_mut()?
                    .pages
                    .iter_mut()
                    .find(|p| p.id == id)
                    .context("Page missing")?
                    .name = name;
            }
            "page_duplicate" => {
                self.editable()?;
                let mut p = self
                    .library
                    .current()?
                    .pages
                    .iter()
                    .find(|p| p.id == id)
                    .context("Page missing")?
                    .clone();
                p.id = new_id();
                p.name = valid_name(&r["name"])?;
                p.manifest_path = page_path(&p.id);
                renew_action_ids(&mut p.raw);
                for slot in &mut p.slots {
                    slot.children.clear();
                    slot.context = new_id();
                    renew_action_ids(&mut slot.raw);
                    if slot.raw.is_object() {
                        slot.raw["ActionID"] = json!(slot.context);
                    }
                }
                let id = p.id.clone();
                let profile = self.library.current_mut()?;
                profile.page_order.push(id.clone());
                profile.pages.push(p);
                return self.select_page(&id, false);
            }
            "page_delete" => {
                self.editable()?;
                let profile = self.library.current_mut()?;
                if id == profile.pin_page {
                    bail!("The pinned page cannot be deleted");
                }
                if !profile.pages.iter().any(|p| p.id == id) {
                    bail!("Page missing");
                }
                if profile.page_order.len() == 1 && profile.page_order[0] == id {
                    bail!("The last top-level page cannot be deleted");
                }
                profile.pages.retain(|p| p.id != id);
                profile.page_order.retain(|p| p != id);
                for page in &mut profile.pages {
                    page.slots.retain(|s| {
                        !(s.uuid == OPEN_FOLDER
                            && s.settings["ProfileUUID"]
                                .as_str()
                                .is_some_and(|v| profile::canonical_page_id(v) == id))
                    });
                }
                let fallback = profile.page_order[0].clone();
                self.library.folders.retain(|p| p != id);
                if self.library.active_page == id {
                    self.library.active_page = fallback;
                    self.library.folders.clear();
                }
                return self.refresh_page();
            }
            "page_move" => {
                self.editable()?;
                let order = &mut self.library.current_mut()?.page_order;
                let index = order
                    .iter()
                    .position(|p| p == id)
                    .context("Only top-level pages can be reordered")?;
                let next = (index as i64
                    + r["direction"]
                        .as_i64()
                        .context("Direction missing")?
                        .signum())
                .clamp(0, order.len() as i64 - 1) as usize;
                order.swap(index, next);
                return self.refresh_page();
            }
            "slot_pin" => {
                self.editable()?;
                let context = r["context"].as_str().context("Context missing")?;
                let mut slot = self
                    .slots
                    .iter()
                    .find(|s| s.context == context)
                    .context("Action missing")?
                    .clone();
                let pinned = r["pinned"].as_bool().context("Pinned must be boolean")?;
                if pinned == slot.pinned {
                    return Ok(());
                }
                let page_id =
                    if pinned {
                        self.library.current()?.pin_page.clone()
                    } else if self.library.active_page == self.library.current()?.pin_page {
                        self.library.folders.last().cloned().unwrap_or_else(|| {
                            self.library.current().unwrap().page_order[0].clone()
                        })
                    } else {
                        self.library.active_page.clone()
                    };
                if self
                    .library
                    .current()?
                    .pages
                    .iter()
                    .find(|p| p.id == page_id)
                    .context("Page missing")?
                    .slots
                    .iter()
                    .any(|s| s.controller == slot.controller && s.index == slot.index)
                {
                    bail!("A different pinned action already occupies this position");
                }
                self.library.remove(context);
                slot.pinned = false;
                let page = self
                    .library
                    .current_mut()?
                    .pages
                    .iter_mut()
                    .find(|p| p.id == page_id)
                    .context("Page missing")?;
                page.slots.push(slot);
                return self.refresh_page();
            }
            "folder_create" => {
                self.editable()?;
                let index = u8::try_from(r["index"].as_u64().context("Key index missing")?)?;
                if index >= 8 {
                    bail!("Key index out of range");
                }
                let mut folder = page(&valid_name(&r["name"])?);
                let id = folder.id.clone();
                folder
                    .slots
                    .push(navigation_slot(BACK_FOLDER, "Back", 0, json!({})));
                self.library.current_mut()?.pages.push(folder);
                self.library.place(navigation_slot(
                    OPEN_FOLDER,
                    &valid_name(&r["name"])?,
                    index,
                    json!({"ProfileUUID":id}),
                ))?;
                return self.refresh_page();
            }
            _ => bail!("Unknown profile command"),
        }
        self.persist()
    }

    pub(super) fn install_bundled_profiles(&mut self, activate: bool) -> Result<()> {
        let mut install = Vec::new();
        for plugin in &self.plugins {
            for entry in plugin.manifest["Profiles"].as_array().into_iter().flatten() {
                let Some(name) = entry["Name"].as_str() else {
                    continue;
                };
                if entry["DeviceType"] != 7 || entry["AutoInstall"] == false {
                    continue;
                }
                if self
                    .library
                    .bundles
                    .get(&plugin.id)
                    .is_some_and(|m| m.contains_key(name))
                {
                    continue;
                }
                let path = safe_file(&plugin.root, &format!("{name}.streamDeckProfile"))?;
                install.push((
                    plugin.id.clone(),
                    name.to_owned(),
                    path,
                    entry["Readonly"].as_bool().unwrap_or(false),
                    entry["DontAutoSwitchWhenInstalled"] != true,
                ));
            }
        }
        for (owner, name, path, readonly, switch) in install {
            let id = self.import_library(&path, Some(&owner), readonly)?;
            self.library
                .bundles
                .entry(owner)
                .or_default()
                .insert(name, Some(id.clone()));
            if activate && switch {
                self.select_profile(&id, None)?;
            }
        }
        self.persist()
    }

    pub(super) fn plugin_switch_profile(
        &mut self,
        plugin_index: usize,
        message: &Value,
    ) -> Result<()> {
        let plugin = &self.plugins[plugin_index];
        if message["context"] != plugin.registration {
            bail!("Profile switching requires the registered plugin context");
        }
        if message.get("device").is_some_and(|d| d != &self.device) {
            bail!("Requested device is not managed by this host");
        }
        let payload = &message["payload"];
        let page_index = payload
            .get("page")
            .map(|index| {
                usize::try_from(index.as_u64().context("Page must be a nonnegative index")?)
                    .map_err(anyhow::Error::from)
            })
            .transpose()?;
        if payload
            .get("profile")
            .is_some_and(|value| !value.is_string() && !value.is_null())
        {
            bail!("Profile name must be text");
        }
        let Some(name) = payload.get("profile").and_then(Value::as_str) else {
            let id = self
                .library
                .previous
                .as_ref()
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| self.library.active_profile.clone());
            let page = page_index
                .map(|index| {
                    self.library
                        .profiles
                        .iter()
                        .find(|p| p.id == id)
                        .context("Previous profile is missing")?
                        .page_order
                        .get(index)
                        .cloned()
                        .context("Page index out of range")
                })
                .transpose()?;
            self.select_profile(&id, page.as_deref())?;
            self.library.previous = None;
            return self.persist();
        };
        let entry = plugin.manifest["Profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["Name"] == name && entry["DeviceType"] == 7)
            .cloned()
            .context("Plugin may only switch to its declared profile for this device")?;
        let owner = plugin.id.clone();
        let existing = self
            .library
            .bundles
            .get(&owner)
            .and_then(|m| m.get(name))
            .and_then(Clone::clone)
            .filter(|id| self.library.profiles.iter().any(|p| p.id == *id));
        let id = if let Some(id) = existing {
            id
        } else {
            let path = safe_file(&plugin.root, &format!("{name}.streamDeckProfile"))?;
            let bundle = profile::load_bundle(&path)?;
            if page_index.is_some_and(|index| index >= bundle.page_order.len().max(1)) {
                bail!("Page index out of range");
            }
            let id = self.import_bundle(
                bundle,
                Some(&owner),
                entry["Readonly"].as_bool().unwrap_or(false),
            )?;
            self.library
                .bundles
                .entry(owner)
                .or_default()
                .insert(name.into(), Some(id.clone()));
            id
        };
        let page = page_index
            .map(|index| {
                self.library
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap()
                    .page_order
                    .get(index)
                    .cloned()
                    .context("Page index out of range")
            })
            .transpose()?;
        self.select_profile(&id, page.as_deref())
    }
}
fn navigation_slot(uuid: &str, name: &str, index: u8, settings: Value) -> Slot {
    Slot {
        children: vec![],
        child_id: String::new(),
        in_multi_action: false,
        context: new_id(),
        controller: "Keypad".into(),
        index,
        uuid: uuid.into(),
        name: name.into(),
        settings,
        title: name.into(),
        state: 0,
        feedback: json!({}),
        layout: "$B1".into(),
        states: json!([{"Title":name}]),
        raw: json!({}),
        image_changed: false,
        title_changed: false,
        pinned: false,
        background: None,
        layout_cache: None,
        image_cache: HashMap::new(),
        display_warnings: vec![],
        preview: String::new(),
    }
}
