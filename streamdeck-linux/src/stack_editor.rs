//! Native Dial Stack editor. Children remain complete raw action records.
//! This edits drafts only; activation and plugin windows are explicit host requests.
use eframe::egui;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const STACK: &str = "com.elgato.streamdeck.dial.stack";
const MEDIA: &str = "com.elgato.streamdeck.system.multimedia";

#[derive(Default)]
pub struct Editor {
    drafts: HashMap<String, Draft>,
}

struct Draft {
    raw: Value,
    dirty: bool,
    selected: Option<usize>,
    add_uuid: String,
    detail_key: String,
    detail: SettingsDraft,
    saving: bool,
    error: Option<String>,
}

#[derive(Default)]
struct SettingsDraft {
    json: String,
    json_dirty: bool,
    numbers: HashMap<String, String>,
    invalid_numbers: HashSet<String>,
    error: Option<String>,
}
impl SettingsDraft {
    fn new(settings: &Value) -> Self {
        Self {
            json: serde_json::to_string_pretty(settings).unwrap_or_default(),
            numbers: settings
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, v)| v.is_number())
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect(),
            ..Self::default()
        }
    }
    fn pending(&self) -> bool {
        self.json_dirty || !self.invalid_numbers.is_empty()
    }
}
impl Draft {
    fn new(raw: Value) -> Self {
        Self {
            raw,
            dirty: false,
            selected: None,
            add_uuid: String::new(),
            detail_key: String::new(),
            detail: SettingsDraft::default(),
            saving: false,
            error: None,
        }
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.saving = false;
        self.error = None;
    }
    fn select(&mut self, index: Option<usize>) {
        if self.selected != index {
            self.selected = index;
            self.detail_key.clear();
        }
    }
}
impl Editor {
    /// Acknowledge stack_save after the synchronous host command. Failed saves
    /// preserve the draft; successful saves permit the normalized snapshot refresh.
    pub fn saved(&mut self, context: &str, success: bool) {
        if let Some(draft) = self.drafts.get_mut(context) {
            draft.saving = false;
            if success {
                draft.dirty = false;
                draft.error = None;
                draft.detail_key.clear();
            } else {
                draft.error = Some("Changes were not saved. Your draft is kept.".into());
            }
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        parent: &Value,
        actions: &[Value],
        outputs: &[(String, String)],
    ) -> Vec<Value> {
        let Some(context) = parent["context"].as_str() else {
            return vec![];
        };
        if parent["uuid"] != STACK {
            return vec![];
        }
        let draft = self
            .drafts
            .entry(context.into())
            .or_insert_with(|| Draft::new(parent["raw"].clone()));
        if !draft.dirty && draft.raw != parent["raw"] {
            draft.raw = parent["raw"].clone();
            draft.detail_key.clear();
        }
        ui.push_id(context, |ui| {
            show_draft(ui, draft, parent, actions, outputs)
        })
        .inner
    }
}

fn show_draft(
    ui: &mut egui::Ui,
    draft: &mut Draft,
    parent: &Value,
    actions: &[Value],
    outputs: &[(String, String)],
) -> Vec<Value> {
    let mut requests = Vec::new();
    let context = parent["context"].as_str().unwrap_or_default();
    ui.heading("Dial Stack");
    if !draft.raw.is_object() || draft.raw.get("Settings").is_some_and(|v| !v.is_object()) {
        ui.weak("The imported stack options are preserved and need the advanced editor.");
    } else {
        ui.horizontal_wrapped(|ui| {
            for (field, label, default) in [
                ("ShowStackIcon", "Show stack icon", true),
                ("FastNavigation", "Fast navigation", false),
            ] {
                let mut value = draft.raw["Settings"][field].as_bool().unwrap_or(default);
                let response = ui.checkbox(&mut value, label);
                if field == "FastNavigation" {
                    response.clone().on_hover_text(
                        "Hold and turn the dial to choose an action, then release to activate it.",
                    );
                }
                if response.changed() {
                    set_option(&mut draft.raw, field, json!(value));
                    draft.changed();
                }
            }
        });
    }
    let entries: Vec<_> = children(&draft.raw)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, child)| {
            let name = child["Name"]
                .as_str()
                .or_else(|| {
                    actions
                        .iter()
                        .find(|a| a["uuid"] == child["UUID"])
                        .and_then(|a| a["name"].as_str())
                })
                .unwrap_or("Unavailable action")
                .to_owned();
            (
                i,
                name,
                child["UUID"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let active = if draft.dirty {
        current_index(&draft.raw)
    } else {
        live_index(parent, entries.len())
    };
    if draft.selected.is_some_and(|i| i >= entries.len()) {
        draft.select(None);
    }
    if entries.is_empty() {
        ui.weak("Add dial actions, then press the dial to switch between them.");
    }
    ui.add_enabled_ui(!draft.detail.pending(), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("stack_actions")
            .max_height(200.)
            .show(ui, |ui| {
                for (index, name, uuid) in &entries {
                    let label = format!(
                        "{}. {name}{}",
                        index + 1,
                        if Some(*index) == active {
                            " · Active"
                        } else {
                            ""
                        }
                    );
                    if ui
                        .selectable_label(draft.selected == Some(*index), label)
                        .on_hover_text(uuid)
                        .clicked()
                    {
                        draft.select(Some(*index));
                    }
                }
            });
        if let Some(index) = draft.selected {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(index > 0, egui::Button::new("Move up"))
                    .clicked()
                    && move_child(&mut draft.raw, index, -1)
                {
                    draft.select(Some(index - 1));
                    draft.changed();
                }
                if ui
                    .add_enabled(index + 1 < entries.len(), egui::Button::new("Move down"))
                    .clicked()
                    && move_child(&mut draft.raw, index, 1)
                {
                    draft.select(Some(index + 1));
                    draft.changed();
                }
                if ui.button("Remove action").clicked() && remove_child(&mut draft.raw, index) {
                    draft.select(None);
                    draft.changed();
                }
                if ui
                    .add_enabled(
                        !draft.dirty && Some(index) != active,
                        egui::Button::new("Activate action"),
                    )
                    .clicked()
                {
                    requests
                        .push(json!({"command":"stack_select","context":context,"index":index}));
                }
            });
        }
        ui.separator();
        let available: Vec<_> = actions.iter().filter(|a| allowed_action(a)).collect();
        if !available.iter().any(|a| a["uuid"] == draft.add_uuid) {
            draft.add_uuid = available
                .first()
                .and_then(|a| a["uuid"].as_str())
                .unwrap_or_default()
                .into();
        }
        let label = ui.label("Add dial action");
        let selected = available
            .iter()
            .find(|a| a["uuid"] == draft.add_uuid)
            .and_then(|a| a["name"].as_str())
            .unwrap_or("Choose an action");
        egui::ComboBox::from_id_salt("stack_add_action")
            .selected_text(selected)
            .height(260.)
            .show_ui(ui, |ui| {
                for action in &available {
                    if let Some(id) = action["uuid"].as_str() {
                        ui.selectable_value(
                            &mut draft.add_uuid,
                            id.to_owned(),
                            action["name"].as_str().unwrap_or(id),
                        );
                    }
                }
            })
            .response
            .labelled_by(label.id);
        if ui
            .add_enabled(!draft.add_uuid.is_empty(), egui::Button::new("Add action"))
            .clicked()
            && let Some(metadata) = available.iter().find(|a| a["uuid"] == draft.add_uuid)
        {
            match append_child(&mut draft.raw, new_child(metadata)) {
                Ok(index) => {
                    draft.select(Some(index));
                    draft.changed();
                }
                Err(error) => draft.error = Some(error),
            }
        }
    });
    if let Some(index) = draft.selected {
        ui.separator();
        let live = parent["stack_children"]
            .get(index)
            .cloned()
            .unwrap_or(Value::Null);
        let key = children(&draft.raw)
            .and_then(|v| v.get(index))
            .and_then(|v| v["ActionID"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| index.to_string());
        if key != draft.detail_key {
            let settings = children(&draft.raw)
                .and_then(|v| v.get(index))
                .map(|v| &v["Settings"])
                .unwrap_or(&Value::Null);
            draft.detail = SettingsDraft::new(settings);
            draft.detail_key = key;
        }
        if let Some(child) = children_mut(&mut draft.raw)
            .and_then(|v| v.get_mut(index))
            .filter(|v| v.is_object())
        {
            let uuid = child["UUID"].as_str().unwrap_or_default().to_owned();
            ui.heading(child["Name"].as_str().unwrap_or("Action settings"));
            if live["has_inspector"].as_bool() == Some(true) {
                let label = if Some(index) == active {
                    "Open plugin settings"
                } else {
                    "Activate & plugin settings"
                };
                if ui
                    .add_enabled(
                        !draft.dirty && !draft.detail.pending(),
                        egui::Button::new(label),
                    )
                    .clicked()
                    && let Some(child_context) = live["context"].as_str()
                {
                    if Some(index) != active {
                        requests.push(
                            json!({"command":"stack_select","context":context,"index":index}),
                        );
                    }
                    requests.push(json!({"command":"open_inspector","context":child_context}));
                }
                if draft.dirty {
                    ui.weak("Save this stack before opening plugin settings.");
                }
            }
            let mut settings = child.get("Settings").cloned().unwrap_or_else(|| json!({}));
            let mut changed = false;
            ui.add_enabled_ui(!draft.detail.json_dirty, |ui| {
                if let Some(result) =
                    crate::system_editor::edit(ui, &uuid, "Encoder", &mut settings)
                {
                    changed |= result;
                } else if let Some(result) =
                    crate::soundboard_editor::edit(ui, &uuid, &mut settings, outputs)
                {
                    changed |= result.changed;
                    if result.refresh_outputs {
                        requests.push(json!({"command":"soundboard_refresh"}));
                    }
                } else {
                    changed |= primitive_settings(ui, &mut settings, &mut draft.detail);
                }
            });
            changed |= settings_json(ui, &mut settings, &mut draft.detail);
            if changed {
                child["Settings"] = settings;
            }
            if changed || draft.detail.pending() {
                draft.changed();
            }
        }
    }
    if draft.detail.pending() {
        ui.weak("Apply or reset incomplete settings before changing actions or saving.");
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                draft.dirty && !draft.detail.pending() && !draft.saving,
                egui::Button::new("Save stack"),
            )
            .clicked()
        {
            requests.push(json!({"command":"stack_save","context":context,"raw":draft.raw}));
            draft.saving = true;
        }
        if ui
            .add_enabled(draft.dirty, egui::Button::new("Discard changes"))
            .clicked()
        {
            *draft = Draft::new(parent["raw"].clone());
        }
        if draft.saving {
            ui.label("Saving…");
        } else if draft.dirty {
            ui.label("Unsaved changes");
        }
    });
    if let Some(error) = &draft.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    requests
}

fn children(raw: &Value) -> Option<&Vec<Value>> {
    raw["Actions"].as_array()
}
fn children_mut(raw: &mut Value) -> Option<&mut Vec<Value>> {
    raw.get_mut("Actions")?.as_array_mut()
}
fn current_index(raw: &Value) -> Option<usize> {
    let count = children(raw)?.len();
    if count == 0 {
        return None;
    }
    Some(
        raw["Settings"]["CurrentIdx"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|&i| i < count)
            .unwrap_or(0),
    )
}
fn live_index(parent: &Value, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let value = parent
        .pointer("/settings/CurrentIdx")
        .or_else(|| parent.pointer("/Settings/CurrentIdx"))
        .or_else(|| parent.pointer("/raw/Settings/CurrentIdx"));
    Some(
        value
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n < count)
            .unwrap_or(0),
    )
}
fn set_option(raw: &mut Value, key: &str, value: Value) -> bool {
    if !raw.is_object() || raw.get("Settings").is_some_and(|v| !v.is_object()) {
        return false;
    }
    if raw.get("Settings").is_none() {
        raw["Settings"] = json!({});
    }
    raw["Settings"][key] = value;
    true
}
fn move_child(raw: &mut Value, index: usize, delta: isize) -> bool {
    let active = current_index(raw);
    let Some(actions) = children_mut(raw) else {
        return false;
    };
    let Some(target) = index
        .checked_add_signed(delta)
        .filter(|&i| i < actions.len())
    else {
        return false;
    };
    if index >= actions.len() {
        return false;
    }
    actions.swap(index, target);
    if let Some(active) = active {
        set_option(
            raw,
            "CurrentIdx",
            json!(if active == index {
                target
            } else if active == target {
                index
            } else {
                active
            }),
        );
    }
    true
}
fn remove_child(raw: &mut Value, index: usize) -> bool {
    let active = current_index(raw).unwrap_or(0);
    let Some(actions) = children_mut(raw) else {
        return false;
    };
    if index >= actions.len() {
        return false;
    }
    actions.remove(index);
    let count = actions.len();
    let next = if active > index { active - 1 } else { active }.min(count.saturating_sub(1));
    set_option(raw, "CurrentIdx", json!(next));
    true
}
fn append_child(raw: &mut Value, child: Value) -> Result<usize, String> {
    let Some(raw) = raw.as_object_mut() else {
        return Err("The imported stack structure is preserved and cannot be edited here.".into());
    };
    let actions = raw
        .entry("Actions")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("The imported child-action structure is not an array.")?;
    let index = actions.len();
    actions.push(child);
    Ok(index)
}
fn allowed_action(action: &Value) -> bool {
    action["controllers"]
        .as_array()
        .is_some_and(|v| v.iter().any(|c| c == "Encoder"))
        && action["uuid"]
            .as_str()
            .is_some_and(|id| id != STACK && !id.starts_with("com.elgato.streamdeck.multiactions."))
}
fn new_child(metadata: &Value) -> Value {
    let mut settings = metadata
        .get("defaults")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if metadata["uuid"] == MEDIA {
        if !settings.is_object() {
            settings = json!({});
        }
        settings["actionIdx"] = json!(18);
    }
    json!({"ActionID":uuid::Uuid::new_v4().to_string(),"UUID":metadata["uuid"],"Name":metadata["name"],
        "Settings":settings,"State":0,"States":metadata.get("states").cloned().unwrap_or_else(||json!([]))})
}

fn primitive_settings(ui: &mut egui::Ui, settings: &mut Value, draft: &mut SettingsDraft) -> bool {
    let Some(settings) = settings.as_object_mut() else {
        ui.weak("Use settings JSON to edit this imported value.");
        return false;
    };
    let mut changed = false;
    for (key, value) in settings {
        ui.push_id(key, |ui| match value {
            Value::Bool(value) => changed |= ui.checkbox(value, key).changed(),
            Value::String(value) => {
                let label = ui.label(key);
                changed |= ui
                    .text_edit_singleline(value)
                    .labelled_by(label.id)
                    .changed();
            }
            Value::Number(_) => {
                let text = draft
                    .numbers
                    .entry(key.clone())
                    .or_insert_with(|| value.to_string());
                let label = ui.label(key);
                if ui
                    .text_edit_singleline(text)
                    .labelled_by(label.id)
                    .changed()
                {
                    match serde_json::from_str::<Value>(text)
                        .ok()
                        .filter(Value::is_number)
                    {
                        Some(number) => {
                            *value = number;
                            draft.invalid_numbers.remove(key);
                            changed = true;
                        }
                        None => {
                            draft.invalid_numbers.insert(key.clone());
                        }
                    }
                }
                if draft.invalid_numbers.contains(key) {
                    ui.colored_label(ui.visuals().error_fg_color, "Enter a complete number.");
                }
            }
            _ => {
                ui.weak(format!("{key}: edit this value in settings JSON."));
            }
        });
    }
    changed
}
fn settings_json(ui: &mut egui::Ui, settings: &mut Value, draft: &mut SettingsDraft) -> bool {
    if !draft.json_dirty {
        draft.json = serde_json::to_string_pretty(settings).unwrap_or_default();
    }
    let mut changed = false;
    egui::CollapsingHeader::new("Settings JSON").show(ui, |ui| {
        if ui
            .add(
                egui::TextEdit::multiline(&mut draft.json)
                    .code_editor()
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            draft.json_dirty = true;
            draft.error = None;
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(draft.json_dirty, egui::Button::new("Apply settings JSON"))
                .clicked()
            {
                match serde_json::from_str::<Value>(&draft.json) {
                    Ok(value) => {
                        *settings = value;
                        *draft = SettingsDraft::new(settings);
                        changed = true;
                    }
                    Err(error) => draft.error = Some(error.to_string()),
                }
            }
            if ui
                .add_enabled(draft.pending(), egui::Button::new("Reset settings form"))
                .clicked()
            {
                *draft = SettingsDraft::new(settings);
            }
        });
        if let Some(error) = &draft.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw() -> Value {
        json!({"FutureParent":{"keep":true},"Settings":{"CurrentIdx":1,"FastNavigation":true,"ShowStackIcon":false,"FutureOption":3},"Actions":[
            {"ActionID":"a","UUID":"plugin.a","Name":"A","Settings":{"nested":{"keep":1}},"State":1,"States":[],"FutureChild":7},
            {"ActionID":"b","UUID":"plugin.b","Name":"B","State":0,"FutureChild":8},
            {"ActionID":"c","UUID":"plugin.c","Settings":{},"FutureChild":9}
        ]})
    }
    #[test]
    fn reorder_and_remove_preserve_active_child_and_unknown_fields() {
        let original = raw();
        let mut edited = original.clone();
        assert!(move_child(&mut edited, 1, -1));
        assert_eq!(current_index(&edited), Some(0));
        assert_eq!(edited["Actions"][0], original["Actions"][1]);
        assert_eq!(edited["Settings"]["FutureOption"], 3);
        assert!(remove_child(&mut edited, 2));
        assert_eq!(current_index(&edited), Some(0));
        assert!(remove_child(&mut edited, 0));
        assert_eq!(current_index(&edited), Some(0));
        assert_eq!(edited["Actions"][0], original["Actions"][0]);
        assert_eq!(edited["FutureParent"], original["FutureParent"]);
    }
    #[test]
    fn new_media_child_uses_dial_defaults_and_rejects_nested_containers() {
        let metadata = json!({"uuid":MEDIA,"name":"Media","controllers":["Encoder","Keypad"],"defaults":{"actionIdx":1,"Future":true},"states":[{"Image":"icon.svg"}]});
        let child = new_child(&metadata);
        assert_eq!(child["Settings"]["actionIdx"], 18);
        assert_eq!(child["Settings"]["Future"], true);
        assert!(uuid::Uuid::parse_str(child["ActionID"].as_str().unwrap()).is_ok());
        let mut edited = raw();
        let original = edited.clone();
        assert_eq!(append_child(&mut edited, child).unwrap(), 3);
        assert_eq!(edited["Settings"], original["Settings"]);
        assert!(allowed_action(&metadata));
        assert!(!allowed_action(
            &json!({"uuid":STACK,"controllers":["Encoder"]})
        ));
        assert!(!allowed_action(
            &json!({"uuid":"com.elgato.streamdeck.multiactions.routine","controllers":["Encoder"]})
        ));
        assert!(!allowed_action(
            &json!({"uuid":"only.keypad","controllers":["Keypad"]})
        ));
    }
    #[test]
    fn rendering_preserves_missing_settings_unknowns_and_unsaved_context_drafts() {
        let mut editor = Editor::default();
        let mut parent = json!({"context":"parent","uuid":STACK,"raw":raw(),"stack_children":[]});
        let mut draft = Draft::new(parent["raw"].clone());
        draft.selected = Some(1);
        editor.drafts.insert("parent".into(), draft);
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor.show(ui, &parent, &[], &[]).is_empty());
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw, parent["raw"]);
        assert!(!editor.drafts["parent"].dirty);
        let draft = editor.drafts.get_mut("parent").unwrap();
        draft.raw["FutureParent"]["local"] = json!(7);
        draft.dirty = true;
        parent["raw"]["FutureParent"]["remote"] = json!(8);
        let second = json!({"context":"other","uuid":STACK,"raw":raw()});
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            editor.show(ui, &second, &[], &[]);
            editor.show(ui, &parent, &[], &[]);
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw["FutureParent"]["local"], 7);
        assert!(
            editor.drafts["parent"].raw["FutureParent"]
                .get("remote")
                .is_none()
        );
        editor.saved("parent", false);
        assert!(editor.drafts["parent"].dirty);
        editor.saved("parent", true);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            editor.show(ui, &parent, &[], &[]);
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw, parent["raw"]);
    }
    #[test]
    fn invalid_current_selection_is_read_only_until_structural_edit() {
        let mut raw = raw();
        raw["Settings"]["CurrentIdx"] = json!(-1);
        let before = raw.clone();
        assert_eq!(current_index(&raw), Some(0));
        assert_eq!(raw, before);
        let mut malformed = json!({"Actions":{"Future":"format"}});
        let before = malformed.clone();
        assert!(append_child(&mut malformed, json!({})).is_err());
        assert_eq!(malformed, before);
    }
}
