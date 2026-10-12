//! Native ordered Multi Action editing. Runtime execution stays in the host.
//! Preserve complete child records, including fields this editor does not know.
use eframe::egui;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const ROUTINE: &str = "com.elgato.streamdeck.multiactions.routine";
const SWITCH: &str = "com.elgato.streamdeck.multiactions.routine2";
const DELAY: &str = "com.elgato.streamdeck.multiactions.delay";
const RANDOM: &str = "com.elgato.streamdeck.multiactions.random";

#[derive(Default)]
pub struct Editor {
    drafts: HashMap<String, Draft>,
}

struct Draft {
    raw: Value,
    dirty: bool,
    branch: usize,
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
            branch: 0,
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
    /// Call after the host processes multi_save. Failed saves retain the draft.
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

    /// Returns explicit host requests. Rendering never starts a multi action or
    /// writes settings; switching context cannot erase another unsaved draft.
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
        let uuid = parent["uuid"].as_str().unwrap_or_default();
        if !matches!(uuid, ROUTINE | SWITCH | RANDOM) {
            ui.weak("This action's sequence format is preserved. Editing is not available yet.");
            return vec![];
        }
        let draft = self
            .drafts
            .entry(context.to_owned())
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
    let switching = parent["uuid"] == SWITCH;
    ui.heading(if switching {
        "Multi Action Switch"
    } else if parent["uuid"] == RANDOM {
        "Random Action"
    } else {
        "Multi Action"
    });
    if parent["multi_running"].as_bool() == Some(true) {
        ui.label("Running");
    }
    if switching {
        ui.add_enabled_ui(!draft.detail.pending(), |ui| {
            ui.horizontal(|ui| {
                for branch in 0..2 {
                    if ui
                        .selectable_label(
                            draft.branch == branch,
                            format!("Sequence {}", branch + 1),
                        )
                        .clicked()
                    {
                        draft.branch = branch;
                        draft.select(None);
                        draft.detail_key.clear();
                    }
                }
            });
        });
    } else {
        draft.branch = 0;
    }
    let entries: Vec<_> = steps(&draft.raw, draft.branch)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, step)| {
            let title = step["Name"]
                .as_str()
                .or_else(|| {
                    actions
                        .iter()
                        .find(|a| a["uuid"] == step["UUID"])
                        .and_then(|a| a["name"].as_str())
                })
                .unwrap_or("Unavailable action")
                .to_owned();
            (
                i,
                title,
                step["UUID"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    if draft.selected.is_some_and(|i| i >= entries.len()) {
        draft.select(None);
    }
    if entries.is_empty() {
        ui.weak(if parent["uuid"] == RANDOM {
            "Add actions to choose from. Each press runs one action."
        } else {
            "Add actions in the order you want them to run."
        });
    }
    ui.add_enabled_ui(!draft.detail.pending(), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("multi_step_list")
            .max_height(200.)
            .show(ui, |ui| {
                for (index, name, uuid) in &entries {
                    if ui
                        .selectable_label(
                            draft.selected == Some(*index),
                            format!("{}. {name}", index + 1),
                        )
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
                    && move_step(&mut draft.raw, draft.branch, index, -1)
                {
                    draft.select(Some(index - 1));
                    draft.changed();
                }
                if ui
                    .add_enabled(index + 1 < entries.len(), egui::Button::new("Move down"))
                    .clicked()
                    && move_step(&mut draft.raw, draft.branch, index, 1)
                {
                    draft.select(Some(index + 1));
                    draft.changed();
                }
                if ui.button("Remove step").clicked()
                    && remove_step(&mut draft.raw, draft.branch, index)
                {
                    draft.select(None);
                    draft.changed();
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
                .to_owned();
        }
        let selected = available
            .iter()
            .find(|a| a["uuid"] == draft.add_uuid)
            .and_then(|a| a["name"].as_str())
            .unwrap_or("Choose an action");
        let label = ui.label("Add action");
        egui::ComboBox::from_id_salt("multi_add_action")
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
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!draft.add_uuid.is_empty(), egui::Button::new("Add step"))
                .clicked()
                && let Some(metadata) = available.iter().find(|a| a["uuid"] == draft.add_uuid)
            {
                match append_step(&mut draft.raw, draft.branch, new_action(metadata)) {
                    Ok(index) => {
                        draft.select(Some(index));
                        draft.changed();
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
            if ui.button("Add delay").clicked() {
                let metadata =
                    json!({"uuid":DELAY,"name":"Delay","defaults":{"delay":1000},"states":[]});
                match append_step(&mut draft.raw, draft.branch, new_action(&metadata)) {
                    Ok(index) => {
                        draft.select(Some(index));
                        draft.changed();
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
        });
    });
    if let Some(index) = draft.selected {
        ui.separator();
        let live = parent["multi_children"]
            .get(draft.branch)
            .and_then(|v| v.get(index))
            .cloned()
            .unwrap_or(Value::Null);
        let branch = draft.branch;
        let child_id = steps(&draft.raw, branch)
            .and_then(|s| s.get(index))
            .and_then(|s| s["ActionID"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{branch}:{index}"));
        if child_id != draft.detail_key {
            let settings = steps(&draft.raw, branch)
                .and_then(|s| s.get(index))
                .map(|s| &s["Settings"])
                .unwrap_or(&Value::Null);
            draft.detail = SettingsDraft::new(settings);
            draft.detail_key = child_id;
        }
        if let Some(child) = steps_mut_existing(&mut draft.raw, branch)
            .and_then(|s| s.get_mut(index))
            .filter(|child| child.is_object())
        {
            let uuid = child["UUID"].as_str().unwrap_or_default().to_owned();
            let metadata = actions.iter().find(|a| a["uuid"] == uuid);
            let mut changed = false;
            let mut settings_changed = false;
            let mut settings = child.get("Settings").cloned().unwrap_or_else(|| json!({}));
            ui.heading(child["Name"].as_str().unwrap_or("Step settings"));
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
                    ui.weak("Save this sequence before opening plugin settings.");
                }
            }
            ui.add_enabled_ui(!draft.detail.json_dirty, |ui| {
                if uuid == DELAY {
                    let mut milliseconds = settings["delay"].as_u64().unwrap_or(1000);
                    let label = ui.label("Wait before the next step");
                    if ui
                        .add(
                            egui::DragValue::new(&mut milliseconds)
                                .range(0..=86_400_000u64)
                                .clamp_existing_to_range(false)
                                .suffix(" ms"),
                        )
                        .labelled_by(label.id)
                        .changed()
                    {
                        if !settings.is_object() {
                            settings = json!({});
                        }
                        settings["delay"] = json!(milliseconds);
                        settings_changed = true;
                    }
                } else if let Some(result) =
                    crate::soundboard_editor::edit(ui, &uuid, &mut settings, outputs)
                {
                    settings_changed |= result.changed;
                    if result.refresh_outputs {
                        requests.push(json!({"command":"soundboard_refresh"}));
                    }
                } else if let Some(result) =
                    crate::system_editor::edit(ui, &uuid, "Keypad", &mut settings)
                {
                    settings_changed |= result;
                } else {
                    settings_changed |= primitive_settings(ui, &mut settings, &mut draft.detail);
                }
                if uuid != DELAY {
                    changed |= timing(ui, child);
                    changed |= desired_state(ui, child, metadata);
                }
            });
            settings_changed |= settings_json(ui, &mut settings, &mut draft.detail);
            if settings_changed {
                child["Settings"] = settings;
                changed = true;
            }
            if changed || draft.detail.pending() {
                draft.changed();
            }
        }
    }
    if draft.detail.pending() {
        ui.weak("Apply or reset incomplete settings before changing steps or saving.");
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                draft.dirty && !draft.detail.pending() && !draft.saving,
                egui::Button::new("Save sequence"),
            )
            .clicked()
        {
            requests.push(json!({"command":"multi_save","context":context,"raw":draft.raw}));
            draft.saving = true;
        }
        if ui
            .add_enabled(draft.dirty, egui::Button::new("Discard changes"))
            .clicked()
        {
            let branch = draft.branch;
            *draft = Draft::new(parent["raw"].clone());
            draft.branch = branch;
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

fn steps(raw: &Value, branch: usize) -> Option<&Vec<Value>> {
    raw["Actions"].get(branch)?["Actions"].as_array()
}
fn steps_mut_existing(raw: &mut Value, branch: usize) -> Option<&mut Vec<Value>> {
    raw.get_mut("Actions")?
        .get_mut(branch)?
        .get_mut("Actions")?
        .as_array_mut()
}
fn append_step(raw: &mut Value, branch: usize, action: Value) -> Result<usize, String> {
    let Some(object) = raw.as_object_mut() else {
        return Err(
            "The imported sequence structure is preserved and cannot be edited here.".into(),
        );
    };
    let groups = object
        .entry("Actions")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("The imported action groups are not an array.")?;
    if groups
        .get(branch)
        .is_some_and(|v| !v.is_object() || v.get("Actions").is_some_and(|a| !a.is_array()))
    {
        return Err(
            "The imported action group structure is preserved and cannot be edited here.".into(),
        );
    }
    groups.resize_with(groups.len().max(branch + 1), || json!({"Actions":[]}));
    let actions = groups[branch]
        .as_object_mut()
        .unwrap()
        .entry("Actions")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap();
    let index = actions.len();
    actions.push(action);
    Ok(index)
}
fn move_step(raw: &mut Value, branch: usize, index: usize, delta: isize) -> bool {
    let Some(actions) = steps_mut_existing(raw, branch) else {
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
    true
}
fn remove_step(raw: &mut Value, branch: usize, index: usize) -> bool {
    let Some(actions) = steps_mut_existing(raw, branch) else {
        return false;
    };
    if index >= actions.len() {
        return false;
    }
    actions.remove(index);
    true
}
fn allowed_action(action: &Value) -> bool {
    action["supported_in_multi"].as_bool() == Some(true)
        && action["controllers"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "Keypad"))
        && !matches!(
            action["uuid"].as_str(),
            Some(ROUTINE | SWITCH | RANDOM | DELAY)
        )
}
fn new_action(metadata: &Value) -> Value {
    json!({"ActionID":uuid::Uuid::new_v4().to_string(),"UUID":metadata["uuid"],
        "Name":metadata["name"],"Settings":metadata.get("defaults").cloned().unwrap_or_else(||json!({})),
        "State":0,"States":metadata.get("states").cloned().unwrap_or_else(||json!([]))})
}

fn timing(ui: &mut egui::Ui, child: &mut Value) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Step timing").show(ui, |ui| {
        if child
            .get("MultiActionDelays")
            .is_some_and(|v| !v.is_object())
        {
            ui.weak("The imported timing format is preserved.");
            return;
        }
        for (field, label) in [("Delay1", "Hold key"), ("Delay2", "Next step gap")] {
            let mut milliseconds = child["MultiActionDelays"][field].as_i64().unwrap_or(100);
            if ui
                .add(
                    egui::Slider::new(&mut milliseconds, 1..=100)
                        .clamping(egui::SliderClamping::Edits)
                        .text(label)
                        .suffix(" ms"),
                )
                .changed()
            {
                if child.get("MultiActionDelays").is_none() {
                    child["MultiActionDelays"] = json!({});
                }
                child["MultiActionDelays"][field] = json!(milliseconds);
                changed = true;
            }
        }
    });
    changed
}
fn desired_state(ui: &mut egui::Ui, child: &mut Value, metadata: Option<&Value>) -> bool {
    let Some(states) = metadata
        .and_then(|m| m["states"].as_array())
        .filter(|a| a.len() > 1)
    else {
        return false;
    };
    let mut selected = child
        .get("OverrideState")
        .map(|v| v.as_u64())
        .unwrap_or(Some(0));
    let text = selected
        .filter(|&i| (i as usize) < states.len())
        .map(|i| format!("State {}", i + 1))
        .unwrap_or("Imported state".into());
    let label = ui.label("Desired action state");
    let mut changed = false;
    egui::ComboBox::from_id_salt("multi_desired_state")
        .selected_text(text)
        .show_ui(ui, |ui| {
            for (index, state) in states.iter().enumerate() {
                let title = state["Title"].as_str().filter(|s| !s.is_empty());
                let label = title
                    .map(|t| format!("State {} · {t}", index + 1))
                    .unwrap_or_else(|| format!("State {}", index + 1));
                changed |= ui
                    .selectable_value(&mut selected, Some(index as u64), label)
                    .clicked();
            }
        })
        .response
        .labelled_by(label.id);
    if changed && let Some(selected) = selected {
        child["OverrideState"] = json!(selected);
    }
    changed
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
        json!({"FutureParent":{"keep":true},"Actions":[
            {"FutureGroup":9,"Actions":[{"ActionID":"a","UUID":"unknown.a","Settings":{"nested":{"keep":1}},"States":[],"State":1,"FutureChild":2},
            {"ActionID":"b","UUID":"unknown.b","Settings":{},"States":[],"State":0}]},
            {"Actions":[{"ActionID":"c","UUID":"unknown.c","Settings":{},"Future":3}]},
            {"FutureUnrecognizedGroup":[1,2,3]}
        ]})
    }

    #[test]
    fn reorder_remove_and_add_preserve_other_groups_and_unknown_record_fields() {
        let original = raw();
        let mut edited = original.clone();
        assert!(move_step(&mut edited, 0, 0, 1));
        assert_eq!(
            edited["Actions"][0]["Actions"][1],
            original["Actions"][0]["Actions"][0]
        );
        assert!(remove_step(&mut edited, 0, 0));
        let child =
            new_action(&json!({"uuid":DELAY,"name":"Delay","defaults":{"delay":1234},"states":[]}));
        assert!(uuid::Uuid::parse_str(child["ActionID"].as_str().unwrap()).is_ok());
        assert_eq!(append_step(&mut edited, 0, child).unwrap(), 1);
        assert_eq!(
            edited["Actions"][0]["Actions"][1]["Settings"]["delay"],
            1234
        );
        assert_eq!(edited["Actions"][0]["FutureGroup"], 9);
        assert_eq!(edited["FutureParent"], original["FutureParent"]);
        for group in [1, 2] {
            assert_eq!(edited["Actions"][group], original["Actions"][group]);
        }
    }

    #[test]
    fn no_input_render_preserves_raw_and_dirty_drafts_across_snapshots_and_contexts() {
        let mut editor = Editor::default();
        let mut parent = json!({"context":"parent","uuid":SWITCH,"raw":raw(),"multi_children":[[],[]],"multi_running":false});
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor.show(ui, &parent, &[], &[]).is_empty());
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw, parent["raw"]);
        let draft = editor.drafts.get_mut("parent").unwrap();
        draft.selected = Some(0);
        draft.raw["FutureParent"]["local"] = json!(7);
        draft.dirty = true;
        parent["raw"]["FutureParent"]["remote"] = json!(8);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor.show(ui, &parent, &[], &[]).is_empty());
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw["FutureParent"]["local"], 7);
        assert!(
            editor.drafts["parent"].raw["FutureParent"]
                .get("remote")
                .is_none()
        );
        let second = json!({"context":"other","uuid":ROUTINE,"raw":raw()});
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor.show(ui, &second, &[], &[]).is_empty());
        });
        output.drop_without_applying_deltas();
        assert!(editor.drafts["parent"].dirty);
        editor.saved("parent", false);
        assert!(editor.drafts["parent"].dirty);
        editor.saved("parent", true);
        assert!(!editor.drafts["parent"].dirty);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            editor.show(ui, &parent, &[], &[]);
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw, parent["raw"]);
    }

    #[test]
    fn selected_child_without_settings_is_not_changed_by_rendering() {
        let mut editor = Editor::default();
        let parent = json!({"context":"parent","uuid":ROUTINE,"raw":{"Actions":[{"Actions":[
            {"ActionID":"child","UUID":"unknown.plugin","State":1,"Future":true}
        ]}]}});
        let mut draft = Draft::new(parent["raw"].clone());
        draft.selected = Some(0);
        editor.drafts.insert("parent".into(), draft);
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor.show(ui, &parent, &[], &[]).is_empty());
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["parent"].raw, parent["raw"]);
        assert!(!editor.drafts["parent"].dirty);
    }

    #[test]
    fn random_action_uses_the_native_editor_without_changing_its_raw_groups() {
        let mut editor = Editor::default();
        let parent = json!({"context":"random","uuid":RANDOM,"raw":raw()});
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            assert!(editor.show(ui, &parent, &[], &[]).is_empty());
        });
        output.drop_without_applying_deltas();
        assert_eq!(editor.drafts["random"].raw, parent["raw"]);
        assert!(!editor.drafts["random"].dirty);
    }

    #[test]
    fn malformed_imported_groups_are_not_replaced_and_nested_actions_are_filtered() {
        let mut raw = json!({"Actions":{"Future":"shape"}});
        let before = raw.clone();
        assert!(append_step(&mut raw, 0, json!({})).is_err());
        assert_eq!(raw, before);
        assert!(!allowed_action(
            &json!({"uuid":SWITCH,"supported_in_multi":true,"controllers":["Keypad"]})
        ));
        assert!(!allowed_action(
            &json!({"uuid":"dial.action","supported_in_multi":true,"controllers":["Encoder"]})
        ));
        assert!(allowed_action(
            &json!({"uuid":"plugin.action","supported_in_multi":true,"controllers":["Keypad"]})
        ));
    }
}
