//! Desktop text clipboard and focused-application clipboard shortcuts.
use crate::system_actions::Plan;
use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub const UUID: &str = "com.deckforge.system.clipboard";

pub fn plan(settings: &Value) -> Result<Plan> {
    let operation = settings
        .get("operation")
        .map(|v| v.as_str().context("Clipboard operation must be text"))
        .transpose()?
        .unwrap_or("copy_text");
    if operation == "copy_text" {
        let text = settings["text"]
            .as_str()
            .context("Clipboard text must be a string")?;
        ensure!(
            text.len() <= 1024 * 1024 && !text.contains('\0'),
            "Clipboard text must be at most 1 MiB without NUL characters"
        );
        return Ok(Plan::Clipboard(text.into()));
    }
    let terminal = settings
        .get("terminal_shortcuts")
        .map(|v| v.as_bool().context("Terminal shortcuts must be boolean"))
        .transpose()?
        .unwrap_or(false);
    let key = match operation {
        "copy" => 46,
        "cut" => {
            ensure!(!terminal, "Terminals do not have a standard Cut shortcut");
            45
        }
        "paste" => 47,
        _ => anyhow::bail!("Unknown clipboard operation"),
    };
    Ok(Plan::Chord(crate::system_keyboard::Chord::from_settings(
        if terminal {
            vec![29, 42, key]
        } else {
            vec![29, key]
        },
        settings,
    )?))
}

pub fn set_text(clipboard: &mut Option<arboard::Clipboard>, text: String) -> Result<()> {
    if clipboard.is_none() {
        *clipboard = Some(arboard::Clipboard::new().context("Open desktop clipboard")?);
    }
    let result = clipboard
        .as_mut()
        .unwrap()
        .set_text(text)
        .context("Set text clipboard");
    if result.is_err() {
        *clipboard = None;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn text_and_shortcuts_preserve_contents_and_select_terminal_chords() {
        assert_eq!(
            plan(&json!({"text":"  Hei 🌍\næøå  "})).unwrap(),
            Plan::Clipboard("  Hei 🌍\næøå  ".into())
        );
        assert_eq!(
            plan(&json!({"text":""})).unwrap(),
            Plan::Clipboard(String::new())
        );
        for (operation, key) in [("copy", 46), ("cut", 45), ("paste", 47)] {
            assert_eq!(
                plan(&json!({"operation":operation})).unwrap(),
                Plan::Chord(vec![29, key].into())
            );
        }
        assert_eq!(
            plan(&json!({"operation":"paste","terminal_shortcuts":true})).unwrap(),
            Plan::Chord(vec![29, 42, 47].into())
        );
        assert_eq!(
            plan(&json!({"operation":"copy","terminal_shortcuts":true})).unwrap(),
            Plan::Chord(vec![29, 42, 46].into())
        );
        for settings in [
            json!({"operation":"cut","terminal_shortcuts":true}),
            json!({"operation":"unknown"}),
            json!({"text":"bad\u{0}"}),
            json!({"operation":"paste","terminal_shortcuts":"yes"}),
        ] {
            assert!(plan(&settings).is_err());
        }
    }
}

#[cfg(test)]
#[path = "system_clipboard/mime_reference.rs"]
mod mime_reference;

#[cfg(test)]
#[path = "system_clipboard/mime_wayland.rs"]
mod mime_wayland;
