//! Native Soundboard configuration. Numeric mode/fade contracts are recovered
//! in evidence/dial-system-contracts.json; behavior and duration are described
//! at https://help.elgato.com/hc/en-us/articles/360059017631 .
use eframe::egui;
use serde_json::{Value, json};

const PLAY: &str = "com.elgato.streamdeck.soundboard.playaudio";
const STOP: &str = "com.elgato.streamdeck.soundboard.stopaudioplay";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct EditResult {
    pub changed: bool,
    pub refresh_outputs: bool,
}

/// Edits draft settings only. Device enumeration and playback belong to the host.
/// Unknown fields and imported device identities remain unchanged on display.
pub fn edit(
    ui: &mut egui::Ui,
    uuid: &str,
    settings: &mut Value,
    outputs: &[(String, String)],
) -> Option<EditResult> {
    if uuid != PLAY && uuid != STOP {
        return None;
    }
    if uuid == STOP {
        ui.label("Stops soundboard playback.");
        return Some(EditResult::default());
    }
    if !settings.is_object() {
        ui.weak("This imported configuration needs the advanced settings editor.");
        return Some(EditResult::default());
    }
    Some(ui.push_id(uuid,|ui| {
        let mut result = EditResult::default();
        let mut path = settings["path"].as_str().unwrap_or_default().to_owned();
        let label = ui.label("Audio file");
        if ui.add(egui::TextEdit::singleline(&mut path).desired_width(f32::INFINITY)
            .hint_text("Choose an audio file on this computer")).labelled_by(label.id).changed() {
            result.changed |= set_field(settings,"path",json!(path));
        }
        if ui.button("Choose audio file…").clicked()
            && let Some(path) = rfd::FileDialog::new().set_title("Choose soundboard audio")
                .add_filter("Audio", &["mp3","wav","flac","m4a"])
                .add_filter("All files", &["*"]).pick_file() {
            result.changed |= set_field(settings,"path",json!(path.to_string_lossy()));
        }
        let path = settings["path"].as_str().unwrap_or_default().trim().trim_matches('"');
        if path.starts_with("\\\\") || (path.as_bytes().get(1)==Some(&b':') &&
            path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)) {
            ui.label("Choose a Linux audio file to replace this imported Windows path.");
        }
        result.changed |= enum_field(ui,settings,"actionType","Playback",1,&[
            (1,"Play / stop"),(0,"Play / overlap"),(2,"Play / restart"),(3,"Loop / stop"),
        ]);
        let mut volume = settings["volume"].as_f64().unwrap_or(50.);
        if ui.add(egui::Slider::new(&mut volume,0. ..=100.).clamping(egui::SliderClamping::Edits).integer().text("Volume").suffix(" %")).changed() {
            result.changed |= set_field(settings,"volume",json!(volume.round() as u64));
        }
        if settings["volume"].as_f64().is_some_and(|v| !(0. ..=100.).contains(&v)) {
            ui.weak("The imported volume is outside 0–100%. Move the slider to choose a supported value.");
        }
        result.changed |= enum_field(ui,settings,"fadeType","Fade",0,&[
            (0,"None"),(1,"Fade in"),(2,"Fade out"),(3,"Fade in and out"),
        ]);
        if matches!(settings["fadeType"].as_i64(),Some(1..=3)) {
            let mut seconds = settings["fadeLen"].as_f64().unwrap_or(1.);
            let label = ui.label("Fade duration");
            if ui.add(egui::Slider::new(&mut seconds,1. ..=5.).clamping(egui::SliderClamping::Edits).step_by(0.1).suffix(" seconds"))
                .labelled_by(label.id).changed() {
                result.changed |= set_field(settings,"fadeLen",json!(seconds));
            }
            if settings["fadeLen"].as_f64().is_some_and(|v| !(1. ..=5.).contains(&v)) {
                ui.weak("The imported duration is outside 1–5 seconds. Move the slider to choose a supported value.");
            }
        }
        ui.separator();
        let label = ui.label("Audio output");
        let mut selection = linux_selection(settings);
        let before = selection.clone();
        let mut selected_explicitly = false;
        egui::ComboBox::from_id_salt("soundboard_output")
            .selected_text(output_label(settings,outputs))
            .show_ui(ui,|ui| {
                selected_explicitly |= ui.selectable_value(&mut selection,Some(String::new()),"System default").clicked();
                for (name,description) in outputs {
                    if name.is_empty() { continue; }
                    selected_explicitly |= ui.selectable_value(&mut selection,Some(name.clone()),
                        if description.is_empty() {name} else {description}).clicked();
                }
            }).response.labelled_by(label.id);
        if (selected_explicitly || selection != before) && let Some(name) = selection {
            result.changed |= set_linux_output(settings,&name);
        }
        result.refresh_outputs = ui.button("Refresh audio outputs").clicked();
        if linux_selection(settings).is_none() {
            ui.weak("The imported output is kept. Select a Linux output to map this action.");
        } else if outputs.is_empty() {
            ui.weak("Refresh to list available audio outputs.");
        }
        result
    }).inner)
}

fn set_field(settings: &mut Value, key: &str, value: Value) -> bool {
    let Some(properties) = settings.as_object_mut() else {
        return false;
    };
    if properties.get(key) == Some(&value) {
        return false;
    }
    properties.insert(key.to_owned(), value);
    true
}

fn enum_field(
    ui: &mut egui::Ui,
    settings: &mut Value,
    key: &str,
    label: &str,
    default: i64,
    choices: &[(i64, &str)],
) -> bool {
    let mut selected = settings
        .get(key)
        .map(|v| v.as_i64())
        .unwrap_or(Some(default));
    let text = choices
        .iter()
        .find(|(id, _)| Some(*id) == selected)
        .map(|(_, label)| *label)
        .unwrap_or("Imported setting");
    let label = ui.label(label);
    let mut explicit = false;
    egui::ComboBox::from_id_salt(key)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for &(id, label) in choices {
                explicit |= ui
                    .selectable_value(&mut selected, Some(id), label)
                    .clicked();
            }
        })
        .response
        .labelled_by(label.id);
    if explicit && let Some(value) = selected {
        return set_field(settings, key, json!(value));
    }
    false
}

/// None means an unmapped/unsupported imported identity. Some("") is the known
/// default, or the user's explicit choice to override an original outputType.
fn linux_selection(settings: &Value) -> Option<String> {
    if let Some(value) = settings.get("linuxOutput") {
        return value.as_str().map(str::to_owned);
    }
    match settings.get("outputType") {
        None => Some(String::new()),
        Some(value) if value.as_str() == Some("") => Some(String::new()),
        _ => None,
    }
}

fn output_label(settings: &Value, outputs: &[(String, String)]) -> String {
    match linux_selection(settings) {
        None => "Imported output · choose a Linux device".into(),
        Some(name) if name.is_empty() => "System default".into(),
        Some(name) => outputs
            .iter()
            .find(|(id, _)| id == &name)
            .map(|(_, description)| {
                if description.is_empty() {
                    name.clone()
                } else {
                    description.clone()
                }
            })
            .unwrap_or_else(|| format!("Unavailable output: {name}")),
    }
}

fn set_linux_output(settings: &mut Value, name: &str) -> bool {
    // Preserve the foreign ID for lossless profile export. The Linux host gives
    // this explicit choice priority, including an explicit empty/default value.
    set_field(settings, "linuxOutput", json!(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_edits_preserve_foreign_output_and_unknown_settings() {
        let mut settings = json!({"outputType":"{foreign-device-id}","actionType":99,"Future":{"a":1},
            "path":"C:/Audio/song.wav","volume":50,"fadeType":1,"fadeLen":2});
        assert_eq!(linux_selection(&settings), None);
        assert!(set_linux_output(&mut settings, "alsa_output.usb-speaker"));
        assert_eq!(settings["outputType"], "{foreign-device-id}");
        assert_eq!(
            linux_selection(&settings).as_deref(),
            Some("alsa_output.usb-speaker")
        );
        assert!(set_field(&mut settings, "actionType", json!(2)));
        assert!(set_field(
            &mut settings,
            "path",
            json!("/home/jack/Music/song.wav")
        ));
        assert_eq!(settings["Future"], json!({"a":1}));
        assert_eq!(settings["fadeLen"], 2);
        assert!(set_linux_output(&mut settings, ""));
        assert_eq!(settings["outputType"], "{foreign-device-id}");
        assert_eq!(linux_selection(&settings).as_deref(), Some(""));
        assert!(!set_linux_output(&mut settings, ""));
    }

    #[test]
    fn available_unknown_and_disappeared_outputs_are_distinguished() {
        let outputs = vec![("sink.usb".into(), "USB speakers".into())];
        assert_eq!(
            output_label(&json!({"outputType":""}), &outputs),
            "System default"
        );
        assert_eq!(
            output_label(&json!({"linuxOutput":"sink.usb"}), &outputs),
            "USB speakers"
        );
        assert!(
            output_label(&json!({"linuxOutput":"missing"}), &outputs)
                .starts_with("Unavailable output:")
        );
        assert!(
            output_label(&json!({"outputType":"windows-id"}), &outputs)
                .starts_with("Imported output")
        );
        assert_eq!(linux_selection(&json!({"linuxOutput":false})), None);
    }

    #[test]
    fn rendering_does_not_normalize_unknown_values_or_refresh_outputs() {
        for (uuid, mut settings) in [
            (
                PLAY,
                json!({"path":"C:/sounds/a.wav","actionType":99,"outputType":"foreign","volume":150,"fadeType":2,"fadeLen":8,"Future":true}),
            ),
            (PLAY, json!({"outputType":"","Future":[1,2]})),
            (
                PLAY,
                json!({"linuxOutput":"disconnected","volume":33.5,"fadeType":999}),
            ),
            (PLAY, Value::Null),
            (STOP, json!({"UnrecognizedStopField":7})),
        ] {
            let before = settings.clone();
            let context = egui::Context::default();
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(
                    edit(ui, uuid, &mut settings, &[]),
                    Some(EditResult::default())
                );
            });
            output.drop_without_applying_deltas();
            assert_eq!(settings, before, "{uuid}");
        }
    }
}
