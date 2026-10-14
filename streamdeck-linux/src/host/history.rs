//! Bounded session editor history. Only recorded configuration changes are
//! reversed; live device/input/process state never belongs to the history graph.
use super::*;
use anyhow::ensure;
use std::collections::{BTreeMap, BTreeSet};
mod patch;
use patch::Patch;

const MAX_ENTRIES: usize = 50;
const MAX_BYTES: usize = 32 * 1024 * 1024;
type Bundles = HashMap<String, HashMap<String, Option<String>>>;
#[derive(Clone, Serialize, Deserialize)]
struct Graph {
    profiles: Vec<library::Profile>,
    smart: smart_profiles::Configs,
    bundles: BTreeMap<String, Bundles>,
}
#[derive(Clone, Serialize)]
struct Navigation {
    devices: BTreeMap<String, device_pool::Navigation>,
    cursors: HashMap<String, String>,
    folders: HashMap<String, Vec<String>>,
}
struct Capture {
    graph: Value,
    navigation: Navigation,
    assets: Vec<PathBuf>,
}
#[derive(Clone, Serialize)]
struct Entry {
    label: String,
    group: Option<String>,
    changes: Vec<Patch>,
    before: Navigation,
    after: Navigation,
    assets: Vec<PathBuf>,
}
#[derive(Default)]
pub(super) struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    revision: u64,
    tracking: bool,
    pending: Vec<PathBuf>,
    note: Option<String>,
}
impl History {
    pub(super) fn snapshot(&self) -> Value {
        json!({"revision":self.revision,"undo":self.undo.last().map(|e|&e.label),
            "redo":self.redo.last().map(|e|&e.label),"note":self.note})
    }
    pub(super) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.pending
            .iter()
            .chain(self.undo.iter().chain(&self.redo).flat_map(|e| &e.assets))
            .map(PathBuf::as_path)
    }
    pub(super) fn clear(&mut self, reason: &str) {
        if self.undo.is_empty() && self.redo.is_empty() {
            return;
        }
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
        self.note = Some(reason.into());
    }
    fn record(&mut self, label: &str, group: Option<&str>, before: Capture, after: Capture) {
        let Some(change) = Patch::between(Some(&before.graph), Some(&after.graph)) else {
            return;
        };
        self.redo.clear();
        self.note = None;
        self.revision += 1;
        let assets: Vec<_> = before
            .assets
            .into_iter()
            .chain(after.assets)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if let Some(last) = self
            .undo
            .last_mut()
            .filter(|e| group.is_some() && e.group.as_deref() == group)
        {
            last.changes.push(change);
            last.after = after.navigation;
            last.assets = last
                .assets
                .iter()
                .cloned()
                .chain(assets)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        } else {
            self.undo.push(Entry {
                label: label.into(),
                group: group.map(str::to_owned),
                changes: vec![change],
                before: before.navigation,
                after: after.navigation,
                assets,
            });
        }
        self.enforce_limit();
    }
    fn enforce_limit(&mut self) {
        while self.undo.len() + self.redo.len() > MAX_ENTRIES
            || serde_json::to_vec(&(&self.undo, &self.redo)).map_or(true, |v| v.len() > MAX_BYTES)
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
            self.note = Some("Edit history reached its session memory limit".into());
        }
    }
}

pub(super) fn label(command: &str) -> Option<&'static str> {
    Some(match command {
        "assign" => "Assign action",
        "remove" => "Remove action",
        "settings" | "settings_patch" | "appearance" => "Edit action",
        "slot_paste" | "child_paste" => "Paste action",
        "slot_transfer" => "Move or copy action",
        "slot_import" => "Import action",
        "slot_pin" => "Pin action",
        "folder_create" => "Create folder",
        "folder_exit" | "folder_exit_reset" => "Edit folder exit",
        "multi_save" | "stack_save" | "trigger_save" | "logic_save" | "wheel_save" => {
            "Edit action sequence"
        }
        "encoder_appearance" | "page_background" => "Edit artwork",
        "profile_create" => "Create profile",
        "profile_rename" => "Rename profile",
        "profile_duplicate" => "Duplicate profile",
        "profile_delete" => "Delete profile",
        "profile_copy_to_device" => "Copy profile to device",
        "import_profile" => "Import profile",
        "page_create" => "Create page",
        "page_rename" => "Rename page",
        "page_duplicate" => "Duplicate page",
        "page_delete" => "Delete page",
        "page_move" => "Reorder page",
        "smart_config" => "Edit Smart Profiles",
        _ => return None,
    })
}
impl Host {
    fn history_capture(&mut self) -> Result<Capture> {
        self.sync_all_devices();
        let devices: BTreeMap<_, _> = self
            .device_records()
            .into_iter()
            .map(|r| (r.scope, r.navigation))
            .collect();
        let mut graph = serde_json::to_value(Graph {
            profiles: self.library.profiles.clone(),
            smart: self.smart_profiles.clone(),
            bundles: devices
                .iter()
                .map(|(s, n)| (s.clone(), n.bundles.clone()))
                .collect(),
        })?;
        normalize_graph(&mut graph, true)?;
        Ok(Capture {
            graph,
            navigation: Navigation {
                devices,
                cursors: self.library.cursors.clone(),
                folders: self.library.folder_paths.clone(),
            },
            assets: self
                .live_asset_sources()
                .into_iter()
                .map(Path::to_path_buf)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        })
    }
    pub(super) fn history_edit<T>(
        &mut self,
        label: &str,
        group: Option<&str>,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if self.history.tracking {
            return operation(self);
        }
        ensure!(
            group.is_none_or(|v| !v.is_empty() && v.len() <= 128),
            "Invalid edit group"
        );
        let before = self.history_capture()?;
        self.history.tracking = true;
        self.history.pending = before.assets.clone();
        let result = operation(self);
        let after = self.history_capture();
        self.history.tracking = false;
        self.history.pending.clear();
        match after {
            Ok(after) => self.history.record(label, group, before, after),
            Err(error) => self
                .history
                .clear(&format!("History could not capture this edit: {error}")),
        }
        result
    }
    pub(super) fn history_command(&mut self, request: &Value) -> Result<()> {
        ensure!(
            request["expected_revision"].as_u64() == Some(self.history.revision),
            "Edit history changed; try again"
        );
        let undo = request["command"] == "history_undo";
        let mut entry = if undo {
            self.history.undo.last()
        } else {
            self.history.redo.last()
        }
        .cloned()
        .context("No edit to reverse")?;
        let before = self.history_capture()?;
        let mut value = before.graph.clone();
        let indices: Vec<_> = if undo {
            (0..entry.changes.len()).rev().collect()
        } else {
            (0..entry.changes.len()).collect()
        };
        for i in indices {
            let next = entry.changes[i]
                .apply_history(Some(&value), undo)?
                .context("History graph is missing")?;
            // Capture current payload for redo when reversing structural creation:
            // normal plugin updates must not make assignment undo unusable.
            entry.changes[i] = if undo {
                Patch::between(Some(&next), Some(&value))
            } else {
                Patch::between(Some(&value), Some(&next))
            }
            .unwrap_or_else(Patch::empty);
            value = next;
        }
        let mut decoded = value.clone();
        normalize_graph(&mut decoded, false)?;
        let graph: Graph = serde_json::from_value(decoded)?;
        let (expected, desired) = if undo {
            (&entry.after, &entry.before)
        } else {
            (&entry.before, &entry.after)
        };
        let records = self.device_records();
        ensure!(
            expected
                .devices
                .keys()
                .all(|scope| records.iter().any(|r| &r.scope == scope)),
            "A device used by this edit is no longer available"
        );
        let mut next = self.library.clone();
        next.profiles = graph.profiles;
        restore_map(&mut next.cursors, &expected.cursors, &desired.cursors);
        restore_map(&mut next.folder_paths, &expected.folders, &desired.folders);
        next.cursors.retain(|id, page| {
            next.profiles
                .iter()
                .any(|p| &p.id == id && p.pages.iter().any(|g| &g.id == page))
        });
        next.folder_paths.retain(|id, path| {
            next.profiles
                .iter()
                .any(|p| &p.id == id && path.iter().all(|g| p.pages.iter().any(|p| &p.id == g)))
        });
        let mut navigation = BTreeMap::new();
        let mut refresh = Vec::new();
        for record in &records {
            let mut nav = record.navigation.clone();
            if let (Some(expected), Some(desired)) = (
                expected.devices.get(&record.scope),
                desired.devices.get(&record.scope),
            ) {
                if nav.active_profile == expected.active_profile
                    && nav.active_page == expected.active_page
                    && nav.folders == expected.folders
                {
                    nav.active_profile = desired.active_profile.clone();
                    nav.active_page = desired.active_page.clone();
                    nav.folders = desired.folders.clone();
                }
                if nav.previous == expected.previous {
                    nav.previous = desired.previous.clone();
                }
            }
            nav.bundles = graph
                .bundles
                .get(&record.scope)
                .cloned()
                .unwrap_or_default();
            repair_navigation(&next, record, &mut nav)?;
            let old: Vec<_> = self
                .library
                .profiles
                .iter()
                .filter(|p| p.device == record.device)
                .collect();
            let new: Vec<_> = next
                .profiles
                .iter()
                .filter(|p| p.device == record.device)
                .collect();
            if serde_json::to_value(old)? != serde_json::to_value(new)?
                || nav.active_profile != record.navigation.active_profile
                || nav.active_page != record.navigation.active_page
            {
                refresh.push(record.scope.clone());
            }
            navigation.insert(record.scope.clone(), nav);
        }
        let active = navigation
            .get(&self.scope)
            .context("Current device missing")?;
        set_navigation(&mut next, active);
        next.validate()?;
        // Validate every parked projection before any mutation or lifecycle event.
        for (scope, nav) in &navigation {
            if scope != &self.scope {
                let mut check = next.clone();
                set_navigation(&mut check, nav);
                check.validate()?;
                check.visible()?;
            }
        }
        let slots = next.visible()?;
        let old_library = std::mem::replace(&mut self.library, next);
        let old_smart = std::mem::replace(&mut self.smart_profiles, graph.smart);
        let old_slots = std::mem::replace(&mut self.slots, slots);
        for (scope, state) in &mut self.other_devices {
            state.navigation = navigation[scope].clone();
        }
        let mode = std::mem::replace(&mut self.canonical_graph_update, true);
        if let Err(error) = self.persist() {
            self.library = old_library;
            self.smart_profiles = old_smart;
            self.slots = old_slots;
            for record in records {
                if let Some(state) = self.other_devices.get_mut(&record.scope) {
                    state.navigation = record.navigation;
                }
            }
            self.canonical_graph_update = mode;
            return Err(error);
        }
        self.slots = old_slots;
        let changed_rules: Vec<_> = old_smart
            .keys()
            .chain(self.smart_profiles.keys())
            .filter(|scope| old_smart.get(*scope) != self.smart_profiles.get(*scope))
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for scope in changed_rules {
            self.invalidate_smart_scope(&scope);
        }
        // History advances at the durable commit, even if rendering later fails.
        let after_nav = Navigation {
            devices: navigation,
            cursors: self.library.cursors.clone(),
            folders: self.library.folder_paths.clone(),
        };
        if undo {
            entry.before = after_nav;
            entry.after = before.navigation;
            self.history.undo.pop();
        } else {
            entry.before = before.navigation;
            entry.after = after_nav;
            self.history.redo.pop();
        }
        entry.assets = entry
            .assets
            .into_iter()
            .chain(before.assets)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if undo {
            self.history.redo.push(entry);
        } else {
            self.history.undo.push(entry);
        }
        self.history.revision += 1;
        self.history.note = None;
        self.history.enforce_limit();
        let mut failure = None;
        for scope in refresh {
            let result=self.with_scope(&scope,|host| {
                host.refresh_page()?;
                let current:Vec<_>=host.slots.iter().flat_map(Slot::visible_flatten).cloned().collect();
                for slot in current {
                    if let Some(old)=old_library.slot(&slot.context) {
                        if slot.settings!=old.settings {
                            host.open_app_holds.remove(&slot.context);
                            if let Some(worker)=&host.system_actions {
                                worker.submit(&slot.context,crate::system_actions::Plan::CancelDials(HashSet::from([slot.context.clone()])))?;
                            }
                            host.emit(&slot,"didReceiveSettings",json!({}));
                            host.notify_inspectors(&slot.context,host.settings_event(&slot,None),None);
                        }
                        if slot.resources!=old.resources {host.emit(&slot,"didReceiveResources",json!({}));host.notify_inspectors(&slot.context,host.resources_event(&slot,None),None);}
                        let state_count=slot.states.as_array().map_or(0,Vec::len).max(old.states.as_array().map_or(0,Vec::len));
                        let states: BTreeSet<_>=(0..state_count as u64).chain(slot.appearances.keys().copied()).chain(old.appearances.keys().copied()).collect();
                        for state in states {
                            if slot.states.get(state as usize)!=old.states.get(state as usize)
                                || serde_json::to_value(slot.appearances.get(&state))?!=serde_json::to_value(old.appearances.get(&state))? {
                                let mut edited=slot.clone();edited.state=state;edited.title=appearance::title(&edited,&host.actions);
                                host.emit(&edited,"titleParametersDidChange",json!({"title":edited.title,"titleParameters":appearance::event_style(&edited,&host.actions),"state":state}));
                            }
                        }
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        self.canonical_graph_update = mode;
        // Hydration can canonicalize lazy resources and child records during
        // refresh. Rebase the opposite operation onto that actual graph while
        // the mutex still excludes asynchronous plugin updates.
        match self.history_capture() {
            Ok(after) => {
                let change = if undo {
                    Patch::between(Some(&after.graph), Some(&before.graph))
                } else {
                    Patch::between(Some(&before.graph), Some(&after.graph))
                }
                .unwrap_or_else(Patch::empty);
                let destination = if undo {
                    &mut self.history.redo
                } else {
                    &mut self.history.undo
                };
                if let Some(entry) = destination.last_mut() {
                    entry.changes = vec![change];
                    entry.assets = entry
                        .assets
                        .iter()
                        .cloned()
                        .chain(after.assets)
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                }
                self.history.enforce_limit();
            }
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error).context("History change was saved, but refresh failed");
        }
        Ok(())
    }
}
// Container branch numbers are semantic addresses (switch state / trigger),
// while entries inside a branch use action identity. Normalize only host-owned
// Slot fields, never arbitrary plugin settings. Native archive data is restored
// exactly before deserializing the replacement graph.
fn normalize_graph(graph: &mut Value, encode: bool) -> Result<()> {
    fn branches(value: &mut Value, encode: bool) -> Result<()> {
        if encode {
            let array = value.as_array().context("History branch list is invalid")?;
            *value = Value::Object(
                array
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), v.clone()))
                    .collect(),
            );
        } else {
            let object = value.as_object().context("History branch map is invalid")?;
            let array = (0..object.len())
                .map(|i| {
                    object
                        .get(&i.to_string())
                        .cloned()
                        .context("History branch address is missing")
                })
                .collect::<Result<Vec<_>>>()?;
            *value = Value::Array(array);
        }
        Ok(())
    }
    fn slot(value: &mut Value, encode: bool) -> Result<()> {
        let uuid = value["uuid"].as_str().unwrap_or("").to_owned();
        if !encode {
            branches(&mut value["children"], false)?;
        }
        for branch in value["children"]
            .as_array_mut()
            .context("History children missing")?
        {
            for child in branch
                .as_array_mut()
                .context("History child branch invalid")?
            {
                slot(child, encode)?;
            }
        }
        if encode {
            branches(&mut value["children"], true)?;
        }
        if encode {
            if (multiaction::container(&uuid)
                || matches!(
                    uuid.as_str(),
                    crate::action_trigger::UUID | crate::key_logic::UUID
                ))
                && value["raw"]["Actions"].is_array()
            {
                let mut raw = value["raw"]
                    .as_object_mut()
                    .unwrap()
                    .remove("Actions")
                    .unwrap();
                branches(&mut raw, true)?;
                value["history_raw_branches"] = raw;
            }
        } else if let Some(mut raw) = value
            .as_object_mut()
            .unwrap()
            .remove("history_raw_branches")
        {
            branches(&mut raw, false)?;
            value["raw"]["Actions"] = raw;
        }
        Ok(())
    }
    for profile in graph["profiles"]
        .as_array_mut()
        .context("History profiles missing")?
    {
        for page in profile["pages"]
            .as_array_mut()
            .context("History pages missing")?
        {
            for action in page["slots"]
                .as_array_mut()
                .context("History slots missing")?
            {
                slot(action, encode)?;
            }
        }
    }
    Ok(())
}

fn restore_map<T: Clone + PartialEq>(
    current: &mut HashMap<String, T>,
    expected: &HashMap<String, T>,
    desired: &HashMap<String, T>,
) {
    for key in expected
        .keys()
        .chain(desired.keys())
        .collect::<HashSet<_>>()
    {
        if current.get(key) == expected.get(key) {
            if let Some(value) = desired.get(key) {
                current.insert(key.clone(), value.clone());
            } else {
                current.remove(key);
            }
        }
    }
}
fn set_navigation(library: &mut Library, nav: &device_pool::Navigation) {
    library.active_profile = nav.active_profile.clone();
    library.active_page = nav.active_page.clone();
    library.folders = nav.folders.clone();
    library.previous = nav.previous.clone();
    library.bundles = nav.bundles.clone();
}
fn repair_navigation(
    library: &Library,
    record: &device_pool::Record,
    nav: &mut device_pool::Navigation,
) -> Result<()> {
    let profile = library
        .profiles
        .iter()
        .find(|p| {
            p.id == nav.active_profile
                && p.device == record.device
                && p.model.profile_compatible(record.model)
        })
        .or_else(|| {
            library
                .profiles
                .iter()
                .find(|p| p.device == record.device && p.model.profile_compatible(record.model))
        })
        .context("History would leave this device without a compatible profile")?;
    if nav.active_profile != profile.id {
        nav.active_profile = profile.id.clone();
        nav.folders.clear();
    }
    if !profile.pages.iter().any(|p| p.id == nav.active_page) {
        nav.active_page = library
            .cursors
            .get(&profile.id)
            .filter(|id| profile.pages.iter().any(|p| &p.id == *id))
            .cloned()
            .unwrap_or_else(|| profile.page_order[0].clone());
        nav.folders.clear();
    }
    nav.folders
        .retain(|id| profile.pages.iter().any(|p| &p.id == id));
    nav.previous = nav.previous.take().filter(|(id, page)| {
        library.profiles.iter().any(|p| {
            p.id == *id && p.device == record.device && p.pages.iter().any(|g| g.id == *page)
        })
    });
    Ok(())
}

#[cfg(test)]
mod tests;
