//! Apply editor intent to current plugin settings without reverting live fields.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn patch(current: &Value, base: &Value, edited: &Value) -> Result<Value> {
    let current = current
        .as_object()
        .context("Current settings must be an object")?;
    let base = base
        .as_object()
        .context("Settings baseline must be an object")?;
    let edited = edited
        .as_object()
        .context("Edited settings must be an object")?;
    let mut result = current.clone();
    let keys: BTreeSet<_> = base.keys().chain(edited.keys()).collect();
    for key in keys {
        let before = base.get(key);
        let after = edited.get(key);
        if before == after {
            continue;
        }
        match after {
            None => {
                result.remove(key);
            }
            Some(after) if after.is_object() && before.is_some_and(Value::is_object) => {
                let live = result
                    .get(key)
                    .context("A settings object was removed while editing; reload it")?;
                ensure!(
                    live.is_object(),
                    "Settings field {key} changed type while editing; reload it"
                );
                result.insert(key.clone(), patch(live, before.unwrap(), after)?);
            }
            Some(after)
                if after.is_object()
                    && before.is_none()
                    && result.get(key).is_some_and(Value::is_object) =>
            {
                result.insert(key.clone(), patch(&result[key], &json!({}), after)?);
            }
            Some(after) => {
                result.insert(key.clone(), after.clone());
            }
        }
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edited_step_keeps_live_count_and_new_plugin_fields() {
        let base = json!({"count":7,"incrementBy":1,"nested":{"a":1,"b":2}});
        let edited = json!({"count":7,"incrementBy":12,"nested":{"a":3,"b":2}});
        let live =
            json!({"count":10,"incrementBy":1,"nested":{"a":1,"b":9,"new":true},"runtime":42});
        assert_eq!(
            patch(&live, &base, &edited).unwrap(),
            json!({"count":10,"incrementBy":12,"nested":{"a":3,"b":9,"new":true},"runtime":42})
        );
    }
    #[test]
    fn explicit_edits_deletions_arrays_and_structural_conflicts_are_distinct() {
        let base = json!({"count":7,"remove":1,"array":[1,2],"nested":{"x":1}});
        let edited = json!({"count":20,"array":[3],"nested":{"x":1}});
        assert_eq!(
            patch(
                &json!({"count":10,"remove":2,"array":[4],"nested":{"x":9}}),
                &base,
                &edited
            )
            .unwrap(),
            json!({"count":20,"array":[3],"nested":{"x":9}})
        );
        assert!(
            patch(
                &json!({"nested":false}),
                &json!({"nested":{"x":1}}),
                &json!({"nested":{"x":2}})
            )
            .is_err()
        );
    }
}
