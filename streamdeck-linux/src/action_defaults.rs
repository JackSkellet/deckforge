//! Materialize effective first-party OS-action defaults at import/export boundaries.
//! Existing saved sessions keep their established missing-mode paste behavior.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Defaults {
    Native,
    Linux,
}

pub fn import_defaults(bundle: &crate::profile::ProfileBundle) -> Defaults {
    let package = bundle
        .files()
        .find(|(name, _)| *name == "package.json")
        .and_then(|(_, bytes)| serde_json::from_slice::<Value>(bytes).ok());
    if package
        .as_ref()
        .and_then(|p| p["OSType"].as_str())
        .is_some_and(|os| os.eq_ignore_ascii_case("linux"))
    {
        Defaults::Linux
    } else {
        Defaults::Native
    }
}

pub fn materialize(settings: &mut Value, defaults: Defaults) {
    let Some(settings) = settings.as_object_mut() else {
        return;
    };
    if settings.contains_key("isTypingMode") {
        return;
    }
    let nonempty = match settings.get("pastedText") {
        Some(Value::String(text)) => !text.is_empty(),
        None => false,
        // Preserve invalid imported settings rather than repairing them silently.
        _ => return,
    };
    settings.insert(
        "isTypingMode".into(),
        Value::Bool(defaults == Defaults::Native && nonempty),
    );
}

pub fn materialize_action(raw: &mut Value, defaults: Defaults, depth: usize) {
    fn visit(raw: &mut Value, defaults: Defaults, depth: usize) {
        if depth > 8 {
            return;
        }
        let Some(uuid) = raw.get("UUID").and_then(Value::as_str) else {
            return;
        };
        if uuid == crate::system_actions::TEXT {
            if let Some(settings) = raw.get_mut("Settings") {
                materialize(settings, defaults);
            }
            return;
        }
        if uuid == crate::system_actions::CLOSE {
            if let Some(settings) = raw.get_mut("Settings").and_then(Value::as_object_mut)
                && !settings.contains_key("linuxCloseMethod")
            {
                settings.insert(
                    "linuxCloseMethod".into(),
                    Value::String(
                        if defaults == Defaults::Native {
                            "windows"
                        } else {
                            "automatic"
                        }
                        .into(),
                    ),
                );
            }
            return;
        }
        let multi = matches!(
            uuid,
            crate::multi::ROUTINE | crate::multi::SWITCH | crate::multi::RANDOM
        );
        let flat = matches!(
            uuid,
            crate::action_trigger::UUID
                | crate::key_logic::UUID
                | crate::dial_stack::UUID
                | crate::action_wheel::UUID
        );
        let trigger = matches!(uuid, crate::action_trigger::UUID | crate::key_logic::UUID);
        // Do not interpret plugin payloads, unknown containers, or unrelated metadata.
        if !multi && !flat {
            return;
        }
        let Some(actions) = raw.get_mut("Actions").and_then(Value::as_array_mut) else {
            return;
        };
        if actions.len() > if trigger { 3 } else { 512 } {
            return;
        }
        if multi {
            let total = actions.iter().try_fold(0usize, |total, group| {
                total.checked_add(group.get("Actions")?.as_array()?.len())
            });
            if total.is_none_or(|total| total > 512) {
                return;
            }
        }
        for action in actions {
            if multi {
                if let Some(children) = action.get_mut("Actions").and_then(Value::as_array_mut) {
                    for child in children {
                        visit(child, defaults, depth + 1);
                    }
                }
            } else {
                visit(action, defaults, depth + 1);
            }
        }
    }
    visit(raw, defaults, depth);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn close_defaults_keep_legacy_linux_background_behavior_and_native_window_behavior() {
        for (defaults, method) in [
            (Defaults::Native, "windows"),
            (Defaults::Linux, "automatic"),
        ] {
            let mut action = json!({"UUID":crate::system_actions::CLOSE,"Settings":{"path":"/bin/sleep","forceQuit":false,"Future":3}});
            materialize_action(&mut action, defaults, 0);
            assert_eq!(
                action["Settings"],
                json!({"path":"/bin/sleep","forceQuit":false,"Future":3,"linuxCloseMethod":method})
            );
        }
        let mut explicit = json!({"UUID":crate::system_actions::CLOSE,"Settings":{"linuxCloseMethod":"terminate"}});
        materialize_action(&mut explicit, Defaults::Native, 0);
        assert_eq!(explicit["Settings"]["linuxCloseMethod"], "terminate");
    }
    #[test]
    fn native_defaults_depend_on_nonempty_text_and_linux_defaults_preserve_paste() {
        for (text, native) in [("", false), (" ", true), ("🌍\n", true)] {
            for defaults in [Defaults::Native, Defaults::Linux] {
                let mut settings =
                    json!({"pastedText":text,"Future":{"keep":9},"isSendingEnter":true});
                materialize(&mut settings, defaults);
                assert_eq!(
                    settings,
                    json!({"pastedText":text,"Future":{"keep":9},"isSendingEnter":true,"isTypingMode":defaults==Defaults::Native&&native})
                );
            }
        }
        let mut enter = json!({"isSendingEnter":true});
        materialize(&mut enter, Defaults::Native);
        assert_eq!(enter["isTypingMode"], false);
        for original in [
            json!({"pastedText":"text","isTypingMode":false}),
            json!({"pastedText":"text","isTypingMode":true}),
            json!({"pastedText":"text","isTypingMode":null}),
            json!({"pastedText":7}),
            Value::Null,
            json!([]),
        ] {
            let mut settings = original.clone();
            materialize(&mut settings, Defaults::Native);
            assert_eq!(settings, original);
        }
    }
    #[test]
    fn walks_only_recognized_action_trees_and_preserves_opaque_payloads() {
        let text = json!({"UUID":crate::system_actions::TEXT,"ActionID":"text","Settings":{"pastedText":"value","Future":1},"States":[{"Title":"label"}],"FutureRoot":2});
        let opaque = json!({"UUID":"unknown.container","Actions":[text.clone()],"Settings":{"nested":text.clone()}});
        for parent in [
            crate::action_trigger::UUID,
            crate::key_logic::UUID,
            crate::dial_stack::UUID,
            crate::action_wheel::UUID,
        ] {
            let mut root = json!({"UUID":crate::multi::ROUTINE,"Actions":[{"Actions":[{"UUID":parent,"Actions":[text.clone(),opaque.clone()],"Future":3}]}],"Future":text.clone()});
            let mut expected = root.clone();
            expected["Actions"][0]["Actions"][0]["Actions"][0]["Settings"]["isTypingMode"] =
                json!(true);
            materialize_action(&mut root, Defaults::Native, 0);
            assert_eq!(root, expected);
        }
    }
}
