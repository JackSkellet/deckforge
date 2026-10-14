//! Reversible field changes. Stable identities let unrelated array members keep
//! newer plugin data; unkeyed arrays are atomic because indices are not identity.
use anyhow::{Result, bail, ensure};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize)]
pub(super) enum Patch {
    Value {
        before: Option<Value>,
        after: Option<Value>,
    },
    Object(BTreeMap<String, Patch>),
    List {
        key: String,
        entries: BTreeMap<String, Patch>,
        order: Option<(Vec<String>, Vec<String>)>,
    },
}
fn keyed(values: &[Value], key: &str) -> Option<(Vec<String>, BTreeMap<String, Value>)> {
    let mut order = Vec::new();
    let mut map = BTreeMap::new();
    for value in values {
        let id = value.get(key)?.as_str()?.to_owned();
        if id.is_empty() || map.insert(id.clone(), value.clone()).is_some() {
            return None;
        }
        order.push(id);
    }
    Some((order, map))
}
fn same_identity(a: Option<&Value>, b: Option<&Value>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    ["context", "id", "ActionID"].iter().any(|key| {
        a.get(key)
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && b.get(key).and_then(Value::as_str) == Some(id))
    }) && a.get("uuid") == b.get("uuid")
        && a.get("UUID") == b.get("UUID")
}
impl Patch {
    pub(super) fn between(before: Option<&Value>, after: Option<&Value>) -> Option<Self> {
        if before == after {
            return None;
        }
        if let (Some(Value::Object(a)), Some(Value::Object(b))) = (before, after) {
            let changes = a
                .keys()
                .chain(b.keys())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter_map(|key| Self::between(a.get(key), b.get(key)).map(|p| (key.clone(), p)))
                .collect();
            return Some(Self::Object(changes));
        }
        if let (Some(Value::Array(a)), Some(Value::Array(b))) = (before, after) {
            for key in ["context", "id", "ActionID"] {
                if let (Some((ao, av)), Some((bo, bv))) = (keyed(a, key), keyed(b, key)) {
                    let entries = av
                        .keys()
                        .chain(bv.keys())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .filter_map(|id| {
                            Self::between(av.get(id), bv.get(id)).map(|p| (id.clone(), p))
                        })
                        .collect();
                    let order = (ao != bo).then_some((ao, bo));
                    return Some(Self::List {
                        key: key.into(),
                        entries,
                        order,
                    });
                }
            }
        }
        Some(Self::Value {
            before: before.cloned(),
            after: after.cloned(),
        })
    }
    pub(super) fn empty() -> Self {
        Self::Object(BTreeMap::new())
    }
    #[cfg(test)]
    pub(super) fn apply(&self, current: Option<&Value>, undo: bool) -> Result<Option<Value>> {
        self.apply_inner(current, undo, false)
    }
    pub(super) fn apply_history(
        &self,
        current: Option<&Value>,
        undo: bool,
    ) -> Result<Option<Value>> {
        self.apply_inner(current, undo, true)
    }
    fn apply_inner(
        &self,
        current: Option<&Value>,
        undo: bool,
        structural: bool,
    ) -> Result<Option<Value>> {
        match self {
            Self::Value { before, after } => {
                let (expected, desired) = if undo {
                    (after, before)
                } else {
                    (before, after)
                };
                ensure!(
                    current == expected.as_ref()
                        || current == desired.as_ref()
                        || (structural
                            && desired.is_none()
                            && same_identity(current, expected.as_ref())),
                    "This edit conflicts with a newer change; history cannot overwrite it"
                );
                Ok(desired.clone())
            }
            Self::Object(changes) => {
                let Some(Value::Object(current)) = current else {
                    bail!("An edited object is no longer available");
                };
                let mut next = current.clone();
                for (key, patch) in changes {
                    if let Some(value) = patch.apply_inner(next.get(key), undo, structural)? {
                        next.insert(key.clone(), value);
                    } else {
                        next.remove(key);
                    }
                }
                Ok(Some(Value::Object(next)))
            }
            Self::List {
                key,
                entries,
                order,
            } => {
                let Some(Value::Array(current)) = current else {
                    bail!("An edited list is no longer available");
                };
                let Some((mut ids, mut values)) = keyed(current, key) else {
                    bail!("Edited identities have changed");
                };
                if let Some((before, after)) = order {
                    let (expected, desired) = if undo {
                        (after, before)
                    } else {
                        (before, after)
                    };
                    ensure!(
                        &ids == expected || &ids == desired,
                        "The edited list order has changed"
                    );
                    ids = desired.clone();
                }
                for (id, patch) in entries {
                    if let Some(value) = patch.apply_inner(values.get(id), undo, structural)? {
                        values.insert(id.clone(), value);
                    } else {
                        values.remove(id);
                    }
                }
                let result = ids
                    .into_iter()
                    .map(|id| {
                        values
                            .remove(&id)
                            .ok_or_else(|| anyhow::anyhow!("Edited item is missing"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                ensure!(values.is_empty(), "History would discard unrelated items");
                Ok(Some(Value::Array(result)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn patch(a: &Value, b: &Value) -> Patch {
        Patch::between(Some(a), Some(b)).unwrap()
    }
    #[test]
    fn reverse_changes_preserve_newer_unrelated_fields_and_members() {
        let a = json!({"profiles":[{"id":"A","name":"Before","slots":[{"context":"one","settings":{"count":1,"label":"Old"}},{"context":"two","feedback":0}]}]});
        let mut b = a.clone();
        b["profiles"][0]["name"] = json!("After");
        b["profiles"][0]["slots"][0]["settings"]["label"] = json!("New");
        let change = patch(&a, &b);
        let mut live = b.clone();
        live["profiles"][0]["slots"][0]["settings"]["count"] = json!(8);
        live["profiles"][0]["slots"][1]["feedback"] = json!(123);
        live["profiles"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"B","name":"Added by another source"}));
        let restored = change.apply(Some(&live), true).unwrap().unwrap();
        assert_eq!(restored["profiles"][0]["name"], "Before");
        assert_eq!(
            restored["profiles"][0]["slots"][0]["settings"]["label"],
            "Old"
        );
        assert_eq!(restored["profiles"][0]["slots"][0]["settings"]["count"], 8);
        assert_eq!(restored["profiles"][0]["slots"][1]["feedback"], 123);
        assert_eq!(restored["profiles"][1], live["profiles"][1]);
        assert_eq!(change.apply(Some(&restored), false).unwrap(), Some(live));
    }
    #[test]
    fn deletion_insertion_and_reorder_reverse_with_complete_nested_payloads() {
        let a = json!([{"context":"A","children":[[{"context":"child","resources":{"file":"owned"}}]]},{"context":"B"}]);
        let b = json!([{"context":"B"},{"context":"C","settings":{"new":true}}]);
        let p = patch(&a, &b);
        assert_eq!(p.apply(Some(&b), true).unwrap(), Some(a.clone()));
        assert_eq!(p.apply(Some(&a), false).unwrap(), Some(b.clone()));
        let changed = json!([{"context":"B"},{"context":"C","settings":{"new":false}}]);
        assert!(p.apply(Some(&changed), true).is_err());
        assert!(
            p.apply(
                Some(&json!([{"context":"C","settings":{"new":true}},{"context":"B"}])),
                true
            )
            .is_err()
        );
    }
    #[test]
    fn conflicting_leaf_is_rejected_without_mutating_input() {
        let a = json!({"name":"Old","settings":{"value":1,"unchanged":2}});
        let b = json!({"name":"New","settings":{"value":2,"unchanged":2}});
        let live = json!({"name":"New","settings":{"value":3,"unchanged":2}});
        let copy = live.clone();
        assert!(patch(&a, &b).apply(Some(&live), true).is_err());
        assert_eq!(live, copy);
    }
    #[test]
    fn vendor_action_ids_keep_plugin_changes_attached_during_reorder() {
        let before = json!([
            {"ActionID":"A","UUID":"plugin.action","Settings":{"count":1}},
            {"ActionID":"B","UUID":"plugin.action","Settings":{"count":2}}
        ]);
        let after = json!([before[1], before[0]]);
        let change = patch(&before, &after);
        let mut live = after.clone();
        live[0]["Settings"]["count"] = json!(25);
        let restored = change.apply(Some(&live), true).unwrap().unwrap();
        assert_eq!(restored[0], before[0]);
        assert_eq!(restored[1]["ActionID"], "B");
        assert_eq!(restored[1]["Settings"]["count"], 25);
        assert_eq!(change.apply(Some(&restored), false).unwrap(), Some(live));
    }
    #[test]
    fn generated_edit_sequences_reverse_exactly_without_intermediate_mutation() {
        let mut document = json!({"profiles":[],"unknown":{"retained":[null,false,42,"data"]}});
        let original = document.clone();
        let mut changes = Vec::new();
        for index in 0..30 {
            let before = document.clone();
            document["profiles"].as_array_mut().unwrap().push(json!({
                "id":format!("profile-{index}"), "name":format!("Profile {index}"),
                "pages":[{"id":format!("page-{index}"),"slots":[{"context":format!("action-{index}"),"settings":{"counter":index}}]}]
            }));
            changes.push(patch(&before, &document));
            let before = document.clone();
            document["profiles"][index]["name"] = json!(format!("Renamed {index}"));
            changes.push(patch(&before, &document));
        }
        let edited = document.clone();
        for change in changes.iter().rev() {
            document = change.apply(Some(&document), true).unwrap().unwrap();
        }
        assert_eq!(document, original);
        for change in &changes {
            document = change.apply(Some(&document), false).unwrap().unwrap();
        }
        assert_eq!(document, edited);
    }
    #[test]
    fn arrays_without_unique_stable_identity_are_atomic_and_null_differs_from_missing() {
        for (a, b) in [
            (
                json!([{"ActionID":"same","n":1},{"ActionID":"same","n":2}]),
                json!([]),
            ),
            (json!([1, 2]), json!([2, 1])),
        ] {
            let p = patch(&a, &b);
            assert_eq!(p.apply(Some(&b), true).unwrap(), Some(a));
            assert!(p.apply(Some(&json!([9])), true).is_err());
        }
        let a = json!({});
        let b = json!({"key":null});
        let p = patch(&a, &b);
        assert_eq!(p.apply(Some(&b), true).unwrap(), Some(a.clone()));
        assert_eq!(p.apply(Some(&a), false).unwrap(), Some(b));
    }
}
