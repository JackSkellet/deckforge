//! Stream Deck display brightness, recovered from the original 7.6 action.
//! This module calculates values and edits settings; it never accesses hardware.
use anyhow::{Result, bail, ensure};
use eframe::egui;
use serde_json::{Value, json};

pub const UUID: &str = "com.elgato.streamdeck.system.keybrightness";
pub const DEFAULT: u8 = 80;

const MODES: [&str; 7] = [
    "Increase by 10%",
    "Decrease by 10%",
    "Set to 100%",
    "Set to 75%",
    "Set to 50%",
    "Set to 25%",
    "Set to 0%",
];

/// Null settings occur in shipped profiles and use the missing-setting default.
/// Return only supported modes for editor validation. Execution additionally
/// preserves the original no-op behavior for out-of-range integer modes.
pub fn mode(settings: &Value) -> Result<u8> {
    let value = configured_mode(settings)?;
    ensure!(
        (0..=6).contains(&value),
        "Unsupported brightness action: {value}"
    );
    Ok(value as u8)
}

fn configured_mode(settings: &Value) -> Result<i64> {
    ensure!(
        settings.is_null() || settings.is_object(),
        "Brightness settings must be an object or null"
    );
    let Some(value) = settings.get("actionIdx") else {
        return Ok(0);
    };
    // The native getter defaults nonnumeric/nonstring JSON values to zero.
    if !value.is_number() && !value.is_string() {
        return Ok(0);
    }
    let value_number = value
        .as_i64()
        .or_else(|| {
            value.as_f64().and_then(|value| {
                // Native floating-point conversion truncates toward zero.
                (value.is_finite() && value >= i64::MIN as f64 && value < i64::MAX as f64)
                    .then_some(value as i64)
            })
        })
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()));
    match value_number {
        Some(value) => Ok(value),
        _ => bail!("Brightness action must be an integer: {value}"),
    }
}

/// Apply one key-down or one repeat of an increase/decrease action.
pub fn key_value(settings: &Value, current: u8) -> Result<u8> {
    let current = current.min(100);
    Ok(match configured_mode(settings)? {
        0 => current.saturating_add(10).min(100),
        1 => current.saturating_sub(10),
        2 => 100,
        3 => 75,
        4 => 50,
        5 => 25,
        6 => 0,
        _ => current,
    })
}

/// Dial rotation ignores actionIdx. Unpressed turns advance to the next 5%
/// boundary in their direction; pressed turns change brightness by 1% per tick.
pub fn rotate(current: u8, ticks: i8, pressed: bool) -> u8 {
    let current = i16::from(current.min(100));
    let ticks = i16::from(ticks);
    let value = if pressed || ticks == 0 {
        current + ticks
    } else if ticks > 0 {
        current / 5 * 5 + ticks * 5
    } else {
        (current + 4) / 5 * 5 + ticks * 5
    };
    value.clamp(0, 100) as u8
}

/// Render without inserting defaults or changing unknown imported fields.
/// Only an explicit key-mode selection creates settings for a null import.
pub fn edit(ui: &mut egui::Ui, controller: &str, settings: &mut Value) -> bool {
    if controller == "Encoder" {
        ui.label("Turn to adjust display brightness in 5% steps.");
        ui.weak(
            "Hold the dial while turning for 1% steps. Press and tap do not change brightness.",
        );
        return false;
    }
    if controller != "Keypad" {
        return false;
    }
    if !settings.is_null() && !settings.is_object() {
        ui.weak("This imported configuration needs the advanced settings editor.");
        return false;
    }
    let selected = mode(settings).ok();
    let selected_text = selected
        .map(|index| MODES[usize::from(index)].to_owned())
        .unwrap_or_else(|| format!("Unsupported imported mode: {}", settings["actionIdx"]));
    let mut chosen = None;
    egui::ComboBox::from_id_salt("brightness_mode")
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for (index, label) in MODES.iter().enumerate() {
                if ui
                    .selectable_label(selected == Some(index as u8), *label)
                    .clicked()
                {
                    chosen = Some(index as u8);
                }
            }
        });
    ui.weak("Controls the Stream Deck's display brightness.");
    chosen.is_some_and(|chosen| set_mode(settings, chosen))
}

fn set_mode(settings: &mut Value, chosen: u8) -> bool {
    let value = json!(chosen);
    if settings.get("actionIdx") == Some(&value) {
        return false;
    }
    if settings.is_null() {
        *settings = json!({});
    }
    settings["actionIdx"] = value;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_modes_defaults_and_clamping_match_the_original_action() {
        assert_eq!(DEFAULT, 80);
        for settings in [
            Value::Null,
            json!({}),
            json!({"future":true}),
            json!({"actionIdx":null}),
            json!({"actionIdx":"0"}),
        ] {
            assert_eq!(mode(&settings).unwrap(), 0);
            assert_eq!(key_value(&settings, 80).unwrap(), 90);
        }
        for (index, expected) in [57, 37, 100, 75, 50, 25, 0].into_iter().enumerate() {
            assert_eq!(
                key_value(&json!({"actionIdx":index}), 47).unwrap(),
                expected
            );
        }
        assert_eq!(key_value(&json!({}), 95).unwrap(), 100);
        assert_eq!(key_value(&json!({"actionIdx":1}), 5).unwrap(), 0);
        assert_eq!(key_value(&json!({}), 255).unwrap(), 100);
    }

    #[test]
    fn unknown_modes_are_noops_and_malformed_strings_are_errors() {
        for value in [json!(7), json!(-1), json!("invalid")] {
            assert!(mode(&json!({"actionIdx":value})).is_err());
        }
        for settings in [json!([]), json!(0), json!("brightness")] {
            assert!(mode(&settings).is_err());
        }
        for value in [json!(-1), json!(7), json!(255), json!("99")] {
            assert_eq!(key_value(&json!({"actionIdx":value}), 47).unwrap(), 47);
        }
        for value in [json!("0.5"), json!("invalid"), json!("2suffix")] {
            assert!(key_value(&json!({"actionIdx":value}), 47).is_err());
        }
    }

    #[test]
    fn native_json_conversion_truncates_floats_and_defaults_other_types() {
        for value in [json!(true), json!(false), json!([]), json!({}), Value::Null] {
            assert_eq!(mode(&json!({"actionIdx":value})).unwrap(), 0);
        }
        for (value, expected) in [(2.9, 100), (3.9, 75), (-0.9, 57), (-1.9, 47)] {
            assert_eq!(
                key_value(&json!({"actionIdx":value}), 47).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn rotation_uses_directional_grid_rounding_and_pressed_fine_steps() {
        for (current, ticks, pressed, expected) in [
            (47, 1, false, 50),
            (47, -1, false, 45),
            (47, 2, false, 55),
            (47, -2, false, 40),
            (50, 1, false, 55),
            (50, -1, false, 45),
            (47, 0, false, 47),
            (47, 1, true, 48),
            (47, -1, true, 46),
            (99, 1, false, 100),
            (1, -1, false, 0),
            (80, 127, false, 100),
            (80, -128, false, 0),
            (80, -128, true, 0),
        ] {
            assert_eq!(rotate(current, ticks, pressed), expected);
        }
    }

    #[test]
    fn batched_ticks_match_repeated_single_ticks_across_the_whole_range() {
        for current in 0..=100 {
            for ticks in i8::MIN..=i8::MAX {
                for pressed in [false, true] {
                    let mut repeated = current;
                    for _ in 0..i16::from(ticks).abs() {
                        repeated = rotate(repeated, ticks.signum(), pressed);
                    }
                    assert_eq!(rotate(current, ticks, pressed), repeated);
                }
            }
        }
    }

    #[test]
    fn explicit_selection_preserves_unknown_fields_and_replaces_null_only_when_edited() {
        let mut settings = json!({"actionIdx":99,"future":{"keep":[1,2,3]}});
        assert!(set_mode(&mut settings, 4));
        assert_eq!(settings, json!({"actionIdx":4,"future":{"keep":[1,2,3]}}));
        assert!(!set_mode(&mut settings, 4));
        let mut settings = Value::Null;
        assert!(set_mode(&mut settings, 0));
        assert_eq!(settings, json!({"actionIdx":0}));
    }

    #[test]
    fn rendering_without_input_preserves_null_defaults_and_unknown_values() {
        for controller in ["Keypad", "Encoder"] {
            for mut settings in [
                Value::Null,
                json!({}),
                json!({"actionIdx":3,"future":true}),
                json!({"actionIdx":99}),
                json!({"actionIdx":null}),
                json!([]),
            ] {
                let original = settings.clone();
                let context = egui::Context::default();
                let output = context.run_ui(egui::RawInput::default(), |ui| {
                    assert!(!edit(ui, controller, &mut settings));
                });
                output.drop_without_applying_deltas();
                assert_eq!(settings, original);
            }
        }
    }
}
