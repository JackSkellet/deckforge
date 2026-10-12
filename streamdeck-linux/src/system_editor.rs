//! Native configuration controls for the first supported system actions.
//! Fields are grounded in evidence/system-action-contracts.json and Elgato's
//! system-action guide. Qt/Windows key identities use their official enums.
use eframe::egui;
use serde_json::{Value, json};

const WEBSITE: &str = "com.elgato.streamdeck.system.website";
const OPEN: &str = "com.elgato.streamdeck.system.open";
const HOTKEY: &str = "com.elgato.streamdeck.system.hotkey";
const MEDIA: &str = "com.elgato.streamdeck.system.multimedia";

/// None leaves configuration to the generic editor. Some(false) is a supported
/// editor with no changes; merely displaying imported settings never rewrites them.
pub fn edit(ui: &mut egui::Ui, uuid: &str, controller: &str, settings: &mut Value) -> Option<bool> {
    if uuid == crate::brightness::UUID {
        return Some(crate::brightness::edit(ui, controller, settings));
    }
    if (controller != "Keypad" && !(controller == "Encoder" && matches!(uuid, HOTKEY | MEDIA)))
        || !matches!(uuid, WEBSITE | OPEN | HOTKEY | MEDIA)
    {
        return None;
    }
    if !settings.is_object() {
        ui.weak("This imported configuration needs the advanced settings editor.");
        return Some(false);
    }
    Some(
        ui.push_id(uuid, |ui| match uuid {
            WEBSITE => website(ui, settings),
            OPEN => open(ui, settings),
            HOTKEY => hotkey(ui, controller, settings),
            MEDIA => multimedia(ui, controller, settings),
            _ => unreachable!(),
        })
        .inner,
    )
}

fn string_field(
    ui: &mut egui::Ui,
    settings: &mut Value,
    key: &str,
    label: &str,
    hint: &str,
) -> bool {
    let mut value = settings[key].as_str().unwrap_or_default().to_owned();
    let label = ui.label(label);
    let response = ui
        .add(
            egui::TextEdit::singleline(&mut value)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
    if response.changed() {
        settings[key] = json!(value);
    }
    response.changed()
}

fn website(ui: &mut egui::Ui, settings: &mut Value) -> bool {
    let mut changed = string_field(
        ui,
        settings,
        "path",
        "Website address",
        "https://example.com",
    );
    let mut browser = settings["openInBrowser"].as_bool().unwrap_or(true);
    if ui.checkbox(&mut browser, "Open in a browser").changed() {
        settings["openInBrowser"] = json!(browser);
        changed = true;
    }
    if browser {
        changed |= string_field(
            ui,
            settings,
            "browser",
            "Browser application",
            "Default browser",
        );
        ui.weak("Leave the browser application empty to use your desktop default.");
    } else {
        ui.weak("Send a request in the background without opening a browser window.");
    }
    changed
}

fn open(ui: &mut egui::Ui, settings: &mut Value) -> bool {
    let mut changed = string_field(
        ui,
        settings,
        "path",
        "Application, file, or folder",
        "Choose a Linux target",
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button("Choose file…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Choose an application or file")
                .pick_file()
        {
            settings["path"] = json!(path.to_string_lossy());
            changed = true;
        }
        if ui.button("Choose folder…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Choose a folder")
                .pick_folder()
        {
            settings["path"] = json!(path.to_string_lossy());
            changed = true;
        }
    });
    let target = settings["path"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .trim_matches('"');
    if windows_path(target) {
        ui.label("This Windows path needs a Linux application, file, or folder. Choose its replacement above.");
    } else {
        ui.weak("Files and folders open with your desktop's default application.");
    }
    changed
}

fn windows_path(path: &str) -> bool {
    path.starts_with("\\\\")
        || (path.as_bytes().get(1) == Some(&b':')
            && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic))
}

#[derive(Clone, Debug)]
struct KeyChoice {
    label: String,
    qt: i64,
    virtual_key: i64,
}

fn keys() -> Vec<KeyChoice> {
    let mut keys = Vec::new();
    for key in b'A'..=b'Z' {
        keys.push(KeyChoice {
            label: (key as char).to_string(),
            qt: key.into(),
            virtual_key: key.into(),
        });
    }
    for key in b'0'..=b'9' {
        keys.push(KeyChoice {
            label: (key as char).to_string(),
            qt: key.into(),
            virtual_key: key.into(),
        });
    }
    // https://doc.qt.io/qt-6/qt.html#Key-enum
    // https://learn.microsoft.com/en-us/windows/win32/inputdev/virtual-key-codes
    for n in 0..24 {
        keys.push(KeyChoice {
            label: format!("F{}", n + 1),
            qt: 0x01000030 + n,
            virtual_key: 0x70 + n,
        });
    }
    for (label, qt, virtual_key) in [
        ("Space", 0x20, 0x20),
        ("Enter", 0x01000004, 0x0d),
        ("Tab", 0x01000001, 0x09),
        ("Escape", 0x01000000, 0x1b),
        ("Backspace", 0x01000003, 0x08),
        ("Insert", 0x01000006, 0x2d),
        ("Delete", 0x01000007, 0x2e),
        ("Home", 0x01000010, 0x24),
        ("End", 0x01000011, 0x23),
        ("Left arrow", 0x01000012, 0x25),
        ("Up arrow", 0x01000013, 0x26),
        ("Right arrow", 0x01000014, 0x27),
        ("Down arrow", 0x01000015, 0x28),
        ("Page up", 0x01000016, 0x21),
        ("Page down", 0x01000017, 0x22),
    ] {
        keys.push(KeyChoice {
            label: label.into(),
            qt,
            virtual_key,
        });
    }
    keys
}

fn selected_key(chord: &Value, keys: &[KeyChoice]) -> Option<usize> {
    if let Some(qt) = chord["QTKeyCode"]
        .as_i64()
        .filter(|q| *q != 0 && *q != 0x01ffffff)
    {
        // An unknown Qt identity remains unknown even if a legacy field could be
        // interpreted differently. No imported key is silently reassigned.
        return keys.iter().position(|key| key.qt == qt);
    }
    chord["VKeyCode"]
        .as_i64()
        .and_then(|vk| keys.iter().position(|key| key.virtual_key == vk))
}

const MODIFIERS: [(&str, &str, u64); 4] = [
    ("KeyCtrl", "Ctrl", 2),
    ("KeyShift", "Shift", 1),
    ("KeyOption", "Alt", 4),
    ("KeyCmd", "Super", 8),
];

fn modifier_values(chord: &Value) -> [bool; 4] {
    let flags = modifier_flags(&chord["KeyModifiers"]);
    MODIFIERS.map(|(field, _, bit)| chord[field].as_bool().unwrap_or(flags & bit != 0))
}

fn modifier_flags(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|v| v as u64))
        .unwrap_or_default()
}

fn update_hotkey(
    settings: &mut Value,
    index: usize,
    modifiers: Option<[bool; 4]>,
    key: Option<&KeyChoice>,
) -> bool {
    if modifiers.is_none() && key.is_none() {
        return false;
    }
    // Never discard a future/nonstandard Hotkeys shape in a convenience editor.
    if !settings.is_object()
        || settings
            .get("Hotkeys")
            .is_some_and(|v| !v.is_array() && !v.is_null())
    {
        return false;
    }
    if settings["Hotkeys"].is_null() {
        settings["Hotkeys"] = json!([]);
    }
    let chords = settings["Hotkeys"].as_array_mut().unwrap();
    if chords.get(index).is_some_and(|v| !v.is_object()) {
        return false;
    }
    chords.resize_with(chords.len().max(index + 1), || json!({}));
    let first = &mut chords[index];
    if let Some(values) = modifiers {
        let signed = first["KeyModifiers"]
            .as_i64()
            .is_some_and(|value| value < 0);
        let mut flags = modifier_flags(&first["KeyModifiers"]) & !0xf;
        for ((field, _, bit), value) in MODIFIERS.into_iter().zip(values) {
            first[field] = json!(value);
            if value {
                flags |= bit;
            }
        }
        first["KeyModifiers"] = if signed {
            json!(flags as i64)
        } else {
            json!(flags)
        };
    }
    if let Some(key) = key {
        // Choosing a standard key replaces imported numpad/OEM identity. Keep
        // only the displayed modifiers, not stale platform-native key metadata.
        let values = modifier_values(first);
        let mut flags = 0u64;
        for ((field, _, bit), value) in MODIFIERS.into_iter().zip(values) {
            first[field] = json!(value);
            if value {
                flags |= bit;
            }
        }
        first["KeyModifiers"] = json!(flags);
        first["QTKeyCode"] = json!(key.qt);
        first["VKeyCode"] = json!(key.virtual_key);
        first.as_object_mut().unwrap().remove("NativeCode");
    }
    true
}

fn hotkey(ui: &mut egui::Ui, controller: &str, settings: &mut Value) -> bool {
    if settings
        .get("Hotkeys")
        .is_some_and(|v| !v.is_array() && !v.is_null())
    {
        ui.weak("This imported shortcut format is preserved. Use advanced settings to edit it.");
        return false;
    }
    let mut changed = false;
    if controller == "Encoder" {
        // Recovered action dispatch: counterclockwise=0, clockwise=1,
        // press/tap=2. Each trigger is a separate chord, never a sequence.
        for (index, label) in [(0, "Turn left"), (1, "Turn right"), (2, "Press or tap")] {
            ui.push_id(index, |ui| {
                egui::CollapsingHeader::new(label)
                    .default_open(true)
                    .show(ui, |ui| {
                        changed |= hotkey_chord(ui, settings, index);
                    });
            });
        }
        ui.weak("Each dial gesture has its own shortcut. Additional imported entries are kept.");
    } else {
        changed = hotkey_chord(ui, settings, 0);
        ui.weak("This edits the first shortcut. Additional imported shortcuts are kept.");
    }
    changed
}

fn hotkey_chord(ui: &mut egui::Ui, settings: &mut Value, index: usize) -> bool {
    if settings["Hotkeys"]
        .get(index)
        .is_some_and(|v| !v.is_object())
    {
        ui.weak("This imported shortcut format is preserved. Use advanced settings to edit it.");
        return false;
    }
    let chord = settings["Hotkeys"]
        .get(index)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut modifiers = modifier_values(&chord);
    let mut modifiers_changed = false;
    ui.label("Shortcut modifiers");
    ui.horizontal_wrapped(|ui| {
        for ((_, label, _), value) in MODIFIERS.into_iter().zip(&mut modifiers) {
            modifiers_changed |= ui.checkbox(value, label).changed();
        }
    });
    let keys = keys();
    let mut selected = selected_key(&chord, &keys);
    let label = selected.map(|i| keys[i].label.clone()).unwrap_or_else(|| {
        if chord.as_object().is_none_or(|o| o.is_empty()) {
            "Choose a key".into()
        } else {
            "Imported shortcut".into()
        }
    });
    let mut key_changed = false;
    let key_label = ui.label("Shortcut key");
    egui::ComboBox::from_id_salt("shortcut_key")
        .selected_text(label)
        .height(260.0)
        .show_ui(ui, |ui| {
            for (index, key) in keys.iter().enumerate() {
                key_changed |= ui
                    .selectable_value(&mut selected, Some(index), &key.label)
                    .clicked();
            }
        })
        .response
        .labelled_by(key_label.id);
    if selected.is_none() && !chord.as_object().is_none_or(|o| o.is_empty()) {
        ui.weak("The imported shortcut stays unchanged until you edit it.");
    }
    update_hotkey(
        settings,
        index,
        modifiers_changed.then_some(modifiers),
        key_changed.then(|| selected.map(|i| &keys[i])).flatten(),
    )
}

const MEDIA_ACTIONS: [(i64, &str); 9] = [
    (0, "Previous track"),
    (1, "Play / pause"),
    (2, "Next track"),
    (3, "Stop"),
    (13, "Fast forward"),
    (14, "Rewind"),
    (4, "Mute / unmute"),
    (5, "Volume up"),
    (6, "Volume down"),
];

fn multimedia(ui: &mut egui::Ui, controller: &str, settings: &mut Value) -> bool {
    let actions: &[(i64, &str)] = if controller == "Encoder" {
        &[(18, "System volume")]
    } else {
        &MEDIA_ACTIONS
    };
    let mut selected = settings["actionIdx"].as_i64();
    let label = actions
        .iter()
        .find(|(id, _)| Some(*id) == selected)
        .map(|(_, label)| *label)
        .unwrap_or(if settings.get("actionIdx").is_some() {
            "Imported media action"
        } else {
            "Choose a media action"
        });
    let mut changed = false;
    let action_label = ui.label("Media control");
    egui::ComboBox::from_id_salt("media_action")
        .selected_text(label)
        .show_ui(ui, |ui| {
            for &(id, label) in actions {
                changed |= ui
                    .selectable_value(&mut selected, Some(id), label)
                    .clicked();
            }
        })
        .response
        .labelled_by(action_label.id);
    if changed && let Some(id) = selected {
        settings["actionIdx"] = json!(id);
    }
    if controller == "Encoder" && selected == Some(18) {
        let mut step = settings["linuxVolumeStep"].as_f64().unwrap_or(5.);
        let label = ui.label("Volume step per tick");
        if ui
            .add(
                egui::DragValue::new(&mut step)
                    .range(0.1..=100.0)
                    .clamp_existing_to_range(false)
                    .speed(0.1)
                    .min_decimals(1)
                    .suffix(" % per dial tick"),
            )
            .labelled_by(label.id)
            .changed()
        {
            settings["linuxVolumeStep"] = json!(step);
            changed = true;
        }
        ui.weak("Turn to adjust system volume; press or tap to mute.");
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_shortcut_edit_preserves_unknown_fields_and_other_chords() {
        let mut settings = json!({"Coalesce":true,"FutureSettings":{"x":1},"Hotkeys":[
            {"KeyModifiers":0x1008,"KeyCmd":true,"QTKeyCode":0x0100ffff,"VKeyCode":-1,
             "NativeCode":999,"FutureChord":{"keep":true}},
            {"QTKeyCode":81,"KeyModifiers":7,"OtherChord":"exact"}
        ]});
        let original = settings.clone();
        let choices = keys();
        let f24 = choices.iter().find(|k| k.label == "F24").unwrap();
        assert!(update_hotkey(
            &mut settings,
            0,
            Some([true, true, false, false]),
            Some(f24)
        ));
        assert_eq!(settings["Hotkeys"][0]["QTKeyCode"], 0x01000047);
        assert_eq!(settings["Hotkeys"][0]["VKeyCode"], 0x87);
        assert_eq!(settings["Hotkeys"][0]["KeyModifiers"], 3);
        assert!(settings["Hotkeys"][0].get("NativeCode").is_none());
        assert_eq!(
            settings["Hotkeys"][0]["FutureChord"],
            original["Hotkeys"][0]["FutureChord"]
        );
        assert_eq!(settings["Hotkeys"][1], original["Hotkeys"][1]);
        assert_eq!(settings["FutureSettings"], original["FutureSettings"]);
        assert_eq!(settings["Coalesce"], true);
    }

    #[test]
    fn reading_unknown_or_macos_shortcuts_never_reassigns_them() {
        let choices = keys();
        let known = json!({"QTKeyCode":71,"VKeyCode":-1,"KeyCmd":true,"KeyModifiers":8});
        assert_eq!(choices[selected_key(&known, &choices).unwrap()].label, "G");
        assert_eq!(modifier_values(&known), [false, false, false, true]);
        let unknown = json!({"QTKeyCode":0x0100ffff,"VKeyCode":65});
        assert!(selected_key(&unknown, &choices).is_none());
        let mut settings = json!({"Hotkeys":[unknown.clone(),{"sentinel":true}]});
        let before = settings.clone();
        assert!(!update_hotkey(&mut settings, 0, None, None));
        assert_eq!(settings, before);
        assert!(update_hotkey(
            &mut settings,
            0,
            Some([true, false, false, false]),
            None
        ));
        assert_eq!(settings["Hotkeys"][0]["QTKeyCode"], unknown["QTKeyCode"]);
        assert_eq!(settings["Hotkeys"][0]["VKeyCode"], 65);
    }

    #[test]
    fn malformed_shortcut_shapes_are_preserved_and_new_keys_are_created_on_edit() {
        let mut malformed = json!({"Hotkeys":{"future":"format"}});
        let before = malformed.clone();
        assert!(!update_hotkey(&mut malformed, 0, Some([true; 4]), None));
        assert_eq!(malformed, before);
        let mut empty = json!({"Other":true});
        assert!(!update_hotkey(&mut empty, 0, None, None));
        assert!(empty.get("Hotkeys").is_none());
        let choice = keys().into_iter().find(|k| k.label == "A").unwrap();
        assert!(update_hotkey(&mut empty, 0, None, Some(&choice)));
        assert_eq!(empty["Hotkeys"][0]["QTKeyCode"], 65);
        assert_eq!(empty["Hotkeys"][0]["VKeyCode"], 65);
        assert_eq!(empty["Hotkeys"][0]["KeyModifiers"], 0);
        assert_eq!(empty["Other"], true);
    }

    #[test]
    fn dial_trigger_edits_only_its_mapped_chord() {
        let mut settings = json!({"Hotkeys":[
            {"QTKeyCode":65,"Future":"left"},
            {"QTKeyCode":66,"Future":"right"},
            {"QTKeyCode":67,"Future":"press"},
            {"Future":"unrecognized fourth entry"}
        ]});
        let before = settings.clone();
        let choices = keys();
        let home = choices.iter().find(|k| k.label == "Home").unwrap();
        assert!(update_hotkey(&mut settings, 1, None, Some(home)));
        assert_eq!(settings["Hotkeys"][1]["QTKeyCode"], 0x01000010);
        assert_eq!(settings["Hotkeys"][1]["Future"], "right");
        for index in [0, 2, 3] {
            assert_eq!(settings["Hotkeys"][index], before["Hotkeys"][index]);
        }
        let mut new = json!({"Other":true});
        assert!(update_hotkey(&mut new, 2, None, Some(home)));
        assert_eq!(new["Hotkeys"].as_array().unwrap().len(), 3);
        assert_eq!(new["Hotkeys"][0], json!({}));
        assert_eq!(new["Hotkeys"][2]["VKeyCode"], 0x24);
    }

    #[test]
    fn signed_identity_flags_survive_modifier_edits() {
        let mut settings = json!({"Hotkeys":[{"KeyModifiers":-2147483640i64,"QTKeyCode":65}]});
        assert!(update_hotkey(
            &mut settings,
            0,
            Some([true, false, false, false]),
            None
        ));
        assert_eq!(settings["Hotkeys"][0]["KeyModifiers"], -2147483646i64);
        assert_eq!(settings["Hotkeys"][0]["QTKeyCode"], 65);
    }

    #[test]
    fn selecting_a_standard_key_removes_stale_keypad_identity_and_translates() {
        let mut settings = json!({"Hotkeys":[{"QTKeyCode":0x31,"VKeyCode":0x61,"NativeCode":79,
            "KeyModifiers":0x4002,"KeyCtrl":true,"Future":"keep"}]});
        let choice = keys().into_iter().find(|k| k.label == "A").unwrap();
        assert!(update_hotkey(&mut settings, 0, None, Some(&choice)));
        assert_eq!(settings["Hotkeys"][0]["KeyModifiers"], 2);
        assert!(settings["Hotkeys"][0].get("NativeCode").is_none());
        assert_eq!(settings["Hotkeys"][0]["Future"], "keep");
        assert_eq!(
            crate::system_keyboard::imported_hotkey(&settings, 0).unwrap(),
            vec![29, 30]
        );
    }

    #[test]
    fn displaying_builtin_editors_preserves_imported_settings_without_input() {
        for (uuid, controller, mut settings) in [
            (
                WEBSITE,
                "Keypad",
                json!({"path":"example.com","Future":true}),
            ),
            (
                OPEN,
                "Keypad",
                json!({"path":"C:/Program Files/Tool.exe","Future":true}),
            ),
            (MEDIA, "Keypad", json!({"actionIdx":777,"Future":true})),
            (MEDIA, "Encoder", json!({"actionIdx":14,"Future":true})),
            (
                HOTKEY,
                "Encoder",
                json!({"Hotkeys":[{"QTKeyCode":0x0100ffff,"KeyModifiers":0x4000},{"QTKeyCode":66},{}],"Future":true}),
            ),
        ] {
            let before = settings.clone();
            let context = egui::Context::default();
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(edit(ui, uuid, controller, &mut settings), Some(false));
            });
            output.drop_without_applying_deltas();
            assert_eq!(settings, before, "{uuid} {controller}");
        }
    }

    #[test]
    fn volume_step_editor_preserves_absent_fractional_and_broader_imported_values() {
        for step in [
            None,
            Some(json!(0.125)),
            Some(json!(5)),
            Some(json!(30.5)),
            Some(json!(100)),
            Some(json!(150)),
        ] {
            let mut settings = json!({"actionIdx":18,"Future":{"preserve":true}});
            if let Some(step) = step {
                settings["linuxVolumeStep"] = step;
            }
            let before = settings.clone();
            let context = egui::Context::default();
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(edit(ui, MEDIA, "Encoder", &mut settings), Some(false));
            });
            output.drop_without_applying_deltas();
            assert_eq!(settings, before);
        }
    }

    #[test]
    fn windows_targets_are_identified_without_treating_linux_paths_as_windows() {
        assert!(windows_path(r"C:\\Program Files\\App.exe"));
        assert!(windows_path("D:/data/report.pdf"));
        assert!(windows_path(r"\\server\share"));
        assert!(!windows_path("/home/jack/file.txt"));
        assert!(!windows_path("firefox"));
    }
}
