//! Native Action Trigger editor. Three semantic positions are fixed; editing
//! never reorders them or executes a child action.
use eframe::egui;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const TRIGGER: &str = "com.elgato.streamdeck.keys.adaptor";
const STACK: &str = "com.elgato.streamdeck.dial.stack";
const POSITIONS: [&str; 3] = ["Turn left", "Press or tap", "Turn right"];

#[derive(Default)]
pub struct Editor {
    drafts: HashMap<String, Draft>,
}
struct Draft {
    raw: Value,
    dirty: bool,
    selected: usize,
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
            selected: 0,
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
}
impl Editor {
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
        if parent["uuid"] != TRIGGER {
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
    ui.heading("Action Trigger");
    ui.weak("Assign a key action to each dial gesture.");
    ui.add_enabled_ui(!draft.detail.pending(), |ui| {
        ui.horizontal_wrapped(|ui| {
            for (index, label) in POSITIONS.iter().enumerate() {
                if ui
                    .selectable_label(draft.selected == index, *label)
                    .clicked()
                {
                    if draft.selected != index {
                        draft.detail_key.clear();
                    }
                    draft.selected = index;
                }
            }
        });
        let name = child_at(&draft.raw, draft.selected).and_then(|c| {
            c["Name"].as_str().or_else(|| {
                actions
                    .iter()
                    .find(|a| a["uuid"] == c["UUID"])
                    .and_then(|a| a["name"].as_str())
            })
        });
        let assigned =
            child_at(&draft.raw, draft.selected).is_some_and(|c| c["UUID"].as_str().is_some());
        ui.label(name.unwrap_or(if assigned {
            "Unavailable action"
        } else {
            "No action assigned"
        }));
        let available: Vec<_> = actions.iter().filter(|a| allowed_action(a)).collect();
        if !available.iter().any(|a| a["uuid"] == draft.add_uuid) {
            draft.add_uuid = available
                .first()
                .and_then(|a| a["uuid"].as_str())
                .unwrap_or_default()
                .into();
        }
        let label = ui.label("Key action");
        let selected = available
            .iter()
            .find(|a| a["uuid"] == draft.add_uuid)
            .and_then(|a| a["name"].as_str())
            .unwrap_or("Choose an action");
        egui::ComboBox::from_id_salt("trigger_action")
            .selected_text(selected)
            .height(260.)
            .show_ui(ui, |ui| {
                for metadata in &available {
                    if let Some(id) = metadata["uuid"].as_str() {
                        ui.selectable_value(
                            &mut draft.add_uuid,
                            id.to_owned(),
                            metadata["name"].as_str().unwrap_or(id),
                        );
                    }
                }
            })
            .response
            .labelled_by(label.id);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !draft.add_uuid.is_empty(),
                    egui::Button::new(if assigned {
                        "Replace action"
                    } else {
                        "Assign action"
                    }),
                )
                .clicked()
                && let Some(metadata) = available.iter().find(|a| a["uuid"] == draft.add_uuid)
            {
                match replace_child(&mut draft.raw, draft.selected, new_child(metadata)) {
                    Ok(()) => {
                        draft.detail_key.clear();
                        draft.changed();
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
            if ui
                .add_enabled(assigned, egui::Button::new("Clear gesture"))
                .clicked()
            {
                match replace_child(&mut draft.raw, draft.selected, json!({"Actions":[]})) {
                    Ok(()) => {
                        draft.detail_key.clear();
                        draft.changed();
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
        });
        ui.weak("Dial Stacks and other Action Triggers cannot be assigned here.");
    });
    if child_at(&draft.raw, draft.selected).is_some_and(|c| c["UUID"].as_str().is_some()) {
        ui.separator();
        let live = parent["trigger_children"]
            .get(draft.selected)
            .cloned()
            .unwrap_or(Value::Null);
        let key = format!(
            "{}:{}",
            draft.selected,
            child_at(&draft.raw, draft.selected)
                .and_then(|c| c["ActionID"].as_str())
                .unwrap_or("imported")
        );
        if key != draft.detail_key {
            let settings = child_at(&draft.raw, draft.selected)
                .map(|c| &c["Settings"])
                .unwrap_or(&Value::Null);
            draft.detail = SettingsDraft::new(settings);
            draft.detail_key = key;
        }
        if let Some(child) = draft
            .raw
            .get_mut("Actions")
            .and_then(Value::as_array_mut)
            .and_then(|a| a.get_mut(draft.selected))
            .filter(|c| c.is_object())
        {
            let uuid = child["UUID"].as_str().unwrap_or_default().to_owned();
            if container(&uuid) {
                ui.weak("This imported nested action is preserved. Its configuration is not editable here yet.");
            } else if multi(&uuid) {
                let child_context = live["context"].as_str();
                if ui
                    .add_enabled(
                        !draft.dirty && !draft.detail.pending() && child_context.is_some(),
                        egui::Button::new("Edit sequence…"),
                    )
                    .clicked()
                    && let Some(child_context) = child_context
                {
                    requests.push(json!({"command":"edit_nested_multi","context":child_context}));
                }
                if draft.dirty {
                    ui.weak("Save this trigger before editing its sequence.");
                }
            } else {
                if live["has_inspector"].as_bool() == Some(true) {
                    if ui
                        .add_enabled(
                            !draft.dirty && !draft.detail.pending(),
                            egui::Button::new("Open plugin settings"),
                        )
                        .clicked()
                        && let Some(child_context) = live["context"].as_str()
                    {
                        requests.push(json!({"command":"open_inspector","context":child_context}));
                    }
                    if draft.dirty {
                        ui.weak("Save this trigger before opening plugin settings.");
                    }
                }
                let mut settings = child.get("Settings").cloned().unwrap_or_else(|| json!({}));
                let mut changed = false;
                ui.add_enabled_ui(!draft.detail.json_dirty, |ui| {
                    if let Some(result) =
                        crate::system_editor::edit(ui, &uuid, "Keypad", &mut settings)
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
    }
    if draft.detail.pending() {
        ui.weak("Apply or reset incomplete settings before changing gestures or saving.");
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                draft.dirty && !draft.detail.pending() && !draft.saving,
                egui::Button::new("Save trigger"),
            )
            .clicked()
        {
            requests.push(json!({"command":"trigger_save","context":context,"raw":draft.raw}));
            draft.saving = true;
        }
        if ui
            .add_enabled(draft.dirty, egui::Button::new("Discard changes"))
            .clicked()
        {
            let selected = draft.selected;
            *draft = Draft::new(parent["raw"].clone());
            draft.selected = selected;
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
fn child_at(raw: &Value, index: usize) -> Option<&Value> {
    raw["Actions"].as_array()?.get(index)
}
fn replace_child(raw: &mut Value, index: usize, child: Value) -> Result<(), String> {
    if index >= 3 {
        return Err("Action Trigger has exactly three gesture positions.".into());
    }
    let Some(raw) = raw.as_object_mut() else {
        return Err(
            "The imported trigger structure is preserved and cannot be edited here.".into(),
        );
    };
    let actions = raw
        .entry("Actions")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("The imported gesture actions are not an array.")?;
    actions.resize_with(actions.len().max(3), || json!({"Actions":[]}));
    actions[index] = child;
    Ok(())
}
fn container(uuid: &str) -> bool {
    uuid == TRIGGER || uuid == STACK
}
fn multi(uuid: &str) -> bool {
    matches!(
        uuid,
        "com.elgato.streamdeck.multiactions.routine"
            | "com.elgato.streamdeck.multiactions.routine2"
            | "com.elgato.streamdeck.multiactions.random"
    )
}
fn allowed_action(action: &Value) -> bool {
    action["controllers"]
        .as_array()
        .is_some_and(|v| v.iter().any(|c| c == "Keypad"))
        && action["uuid"].as_str().is_some_and(|id| !container(id))
}
fn new_child(metadata: &Value) -> Value {
    let mut child = json!({"ActionID":uuid::Uuid::new_v4().to_string(),"UUID":metadata["uuid"],"Name":metadata["name"],
        "Settings":metadata.get("defaults").cloned().unwrap_or_else(||json!({})),"State":0,
        "States":metadata.get("states").cloned().unwrap_or_else(||json!([]))});
    if metadata["uuid"].as_str().is_some_and(multi) {
        child["Actions"] = json!([{"Actions":[]},{"Actions":[]}]);
    }
    child
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
        json!({"FutureParent":{"keep":true},"Settings":{"FutureSetting":2},"Actions":[
            {"Actions":[],"FuturePlaceholder":3},
            {"ActionID":"middle","UUID":"plugin.counter","Name":"Counter","Settings":{"count":7},"State":1,"FutureChild":4},
            {"Actions":[]},
            {"UnknownExtraPosition":{"keep":true}}
        ]})
    }
    #[test]
    fn semantic_positions_assign_and_clear_without_reordering_other_raw_records() {
        let mut edited = raw();
        let before = edited.clone();
        let child = new_child(
            &json!({"uuid":"plugin.counter","name":"Counter","defaults":{"count":0},"states":[]}),
        );
        assert!(uuid::Uuid::parse_str(child["ActionID"].as_str().unwrap()).is_ok());
        replace_child(&mut edited, 2, child).unwrap();
        for index in [0, 1, 3] {
            assert_eq!(edited["Actions"][index], before["Actions"][index]);
        }
        assert_eq!(edited["Settings"], before["Settings"]);
        assert_eq!(edited["FutureParent"], before["FutureParent"]);
        replace_child(&mut edited, 1, json!({"Actions":[]})).unwrap();
        assert_eq!(edited["Actions"][1], json!({"Actions":[]}));
        assert_eq!(edited["Actions"][2]["UUID"], "plugin.counter");
        assert_eq!(edited["Actions"][3], before["Actions"][3]);
        let mut sparse = json!({"Future":true});
        replace_child(&mut sparse, 2, json!({"UUID":"right"})).unwrap();
        assert_eq!(sparse["Actions"][0], json!({"Actions":[]}));
        assert_eq!(sparse["Actions"][1], json!({"Actions":[]}));
        assert_eq!(sparse["Actions"][2]["UUID"], "right");
    }
    #[test]
    fn counter_is_allowed_even_when_multi_action_support_is_false() {
        assert!(allowed_action(
            &json!({"uuid":"com.elgato.counter.action","controllers":["Keypad"],"supported_in_multi":false})
        ));
        for uuid in [TRIGGER, STACK] {
            assert!(!allowed_action(
                &json!({"uuid":uuid,"controllers":["Keypad"]})
            ));
        }
        assert!(!allowed_action(
            &json!({"uuid":"only.dial","controllers":["Encoder"]})
        ));
    }
    #[test]
    fn nested_multi_actions_are_assignable_and_have_a_valid_empty_sequence() {
        for uuid in [
            "com.elgato.streamdeck.multiactions.routine",
            "com.elgato.streamdeck.multiactions.routine2",
            "com.elgato.streamdeck.multiactions.random",
        ] {
            let metadata =
                json!({"uuid":uuid,"name":"Sequence","controllers":["Keypad"],"states":[{}]});
            assert!(allowed_action(&metadata));
            let child = new_child(&metadata);
            let parsed = crate::multi::Sequence::parse(uuid, &child).unwrap();
            assert_eq!(parsed.groups.len(), 2);
            assert!(parsed.groups.iter().all(Vec::is_empty));
            assert_eq!(child["Settings"], json!({}));
        }
    }

    #[test]
    fn no_input_render_preserves_blank_positions_missing_settings_and_failed_drafts() {
        let mut editor = Editor::default();
        let mut parent = json!({"context":"parent","uuid":TRIGGER,"raw":raw(),"trigger_children":[null,null,null]});
        let context = egui::Context::default();
        for position in 0..3 {
            let mut draft = Draft::new(parent["raw"].clone());
            draft.selected = position;
            editor.drafts.insert("parent".into(), draft);
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                assert!(editor.show(ui, &parent, &[], &[]).is_empty());
            });
            output.drop_without_applying_deltas();
            assert_eq!(editor.drafts["parent"].raw, parent["raw"]);
            assert!(!editor.drafts["parent"].dirty);
        }
        let draft = editor.drafts.get_mut("parent").unwrap();
        draft.raw["FutureParent"]["local"] = json!(7);
        draft.dirty = true;
        parent["raw"]["FutureParent"]["remote"] = json!(8);
        let second = json!({"context":"other","uuid":TRIGGER,"raw":{"Actions":[{"UUID":"unknown.plugin","ActionID":"without-settings"}]}});
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            editor.show(ui, &second, &[], &[]);
            editor.show(ui, &parent, &[], &[]);
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["other"].raw, second["raw"]);
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
    fn malformed_imports_are_not_replaced_by_convenience_controls() {
        let mut malformed = json!({"Actions":{"Future":"shape"}});
        let before = malformed.clone();
        assert!(replace_child(&mut malformed, 0, json!({})).is_err());
        assert_eq!(malformed, before);
        let mut value = raw();
        let before = value.clone();
        assert!(replace_child(&mut value, 3, json!({})).is_err());
        assert_eq!(value, before);
    }
}
