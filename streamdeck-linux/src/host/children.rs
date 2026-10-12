//! Shared ownership for container actions. Raw formats remain container-specific.
use super::*;
impl Slot {
    pub(super) fn flatten(&self) -> Vec<&Self> {
        let mut slots = vec![self];
        for child in self.children.iter().flatten() {
            slots.extend(child.flatten());
        }
        slots
    }
    pub(super) fn active_stack_index(&self) -> usize {
        self.settings["CurrentIdx"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .filter(|&v| self.children.first().is_some_and(|c| v < c.len()))
            .unwrap_or(0)
    }
    pub(super) fn visible_flatten(&self) -> Vec<&Self> {
        let mut slots = vec![self];
        if self.uuid == crate::dial_stack::UUID {
            if let Some(child) = self
                .children
                .first()
                .and_then(|c| c.get(self.active_stack_index()))
            {
                slots.extend(child.visible_flatten());
            }
        } else {
            for child in self.children.iter().flatten() {
                slots.extend(child.visible_flatten());
            }
        }
        slots
    }
    pub(super) fn find(&self, context: &str) -> Option<&Self> {
        if self.context == context {
            Some(self)
        } else {
            self.children.iter().flatten().find_map(|s| s.find(context))
        }
    }
    pub(super) fn find_mut(&mut self, context: &str) -> Option<&mut Self> {
        if self.context == context {
            Some(self)
        } else {
            self.children
                .iter_mut()
                .flatten()
                .find_map(|s| s.find_mut(context))
        }
    }
    pub(super) fn sync_children_raw(&mut self) {
        self.sync_children_raw_at(0);
    }
    fn sync_children_raw_at(&mut self, depth: usize) {
        if depth > 8 {
            return;
        }
        let multi = multiaction::container(&self.uuid);
        let stack = self.uuid == crate::dial_stack::UUID;
        let trigger = self.uuid == crate::action_trigger::UUID;
        if !multi && !stack && !trigger {
            return;
        }
        for (branch, children) in self.children.iter_mut().enumerate() {
            for (index, child) in children.iter_mut().enumerate() {
                child.sync_children_raw_at(depth + 1);
                if !child.raw.is_object() {
                    continue;
                }
                if (child.raw.get("Settings").is_some() || child.settings != json!({}))
                    && !(child.raw.get("Settings") == Some(&Value::Null)
                        && child.settings == json!({}))
                {
                    child.raw["Settings"] = child.settings.clone();
                }
                if child.raw.get("State").is_some() || child.state != 0 {
                    child.raw["State"] = json!(child.state);
                }
                if child.title_changed
                    && let Some(state) = child.raw["States"].get_mut(child.state as usize)
                    && state.is_object()
                {
                    state["Title"] = json!(child.title);
                }
                let record = if trigger {
                    self.raw.get_mut("Actions").and_then(|v| v.get_mut(branch))
                } else if stack {
                    self.raw.get_mut("Actions").and_then(|v| v.get_mut(index))
                } else {
                    self.raw
                        .get_mut("Actions")
                        .and_then(|v| v.get_mut(branch))
                        .and_then(|v| v.get_mut("Actions"))
                        .and_then(|v| v.get_mut(index))
                };
                if let Some(record) = record {
                    *record = child.raw.clone();
                }
            }
        }
    }
    pub(super) fn hydrate_children(&mut self, actions: &[Action]) -> Result<()> {
        self.hydrate_children_at(actions, 0)
    }
    fn hydrate_children_at(&mut self, actions: &[Action], depth: usize) -> Result<()> {
        anyhow::ensure!(depth <= 8, "Container nesting exceeds eight levels");
        let multi = multiaction::container(&self.uuid);
        let stack = self.uuid == crate::dial_stack::UUID;
        let trigger = self.uuid == crate::action_trigger::UUID;
        if !multi && !stack && !trigger {
            // Unknown containers keep their raw Actions. They do not activate a
            // stale private runtime child tree from another action type.
            self.children.clear();
            return Ok(());
        }
        let mut parent_raw = self.raw.clone();
        if parent_raw.is_null() {
            parent_raw = json!({});
        }
        anyhow::ensure!(
            parent_raw.is_object(),
            "Container action data must be an object"
        );
        if parent_raw.get("Actions").is_none() {
            parent_raw["Actions"] = if multi {
                json!([{"Actions":[]},{"Actions":[]}])
            } else if trigger {
                json!([{"Actions":[]},{"Actions":[]},{"Actions":[]}])
            } else {
                json!([])
            };
        }
        let groups: Vec<Vec<(String, String, Value)>> = if multi {
            crate::multi::Sequence::parse(&self.uuid, &parent_raw)?
                .groups
                .into_iter()
                .map(|g| g.into_iter().map(|e| (e.id, e.uuid, e.raw)).collect())
                .collect()
        } else if trigger {
            crate::action_trigger::parse(&parent_raw)?
                .children
                .into_iter()
                .map(|entry| entry.into_iter().map(|e| (e.id, e.uuid, e.raw)).collect())
                .collect()
        } else {
            let mut model = parent_raw.clone();
            model["Settings"] = self.settings.clone();
            vec![
                crate::dial_stack::parse(&model)?
                    .children
                    .into_iter()
                    .map(|e| (e.id, e.uuid, e.raw))
                    .collect(),
            ]
        };
        let keypad = multi || trigger;
        // Build a complete candidate before assigning it. Any invalid descendant
        // leaves the previous tree, contexts and raw record intact.
        let mut hydrated = Vec::with_capacity(groups.len());
        for entries in groups {
            let mut group = Vec::with_capacity(entries.len());
            for (id, uuid, raw) in entries {
                let previous = self
                    .children
                    .iter()
                    .flatten()
                    .find(|s| s.child_id == id && s.uuid == uuid);
                let mut child = if let Some(existing) = previous.filter(|s| s.raw == raw) {
                    existing.clone()
                } else {
                    let action = actions.iter().find(|a| a.uuid == uuid);
                    let state = raw["State"].as_u64().unwrap_or(0);
                    let states = raw
                        .get("States")
                        .cloned()
                        .or_else(|| action.map(|a| a.raw["States"].clone()))
                        .unwrap_or(json!([]));
                    Slot {
                        context: previous
                            .map(|s| s.context.clone())
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                        // Reuse descendant identities even when this container's
                        // own settings or child records have just been edited.
                        children: previous.map(|s| s.children.clone()).unwrap_or_default(),
                        child_id: id,
                        appearances: previous.map(|s| s.appearances.clone()).unwrap_or_default(),
                        asset_source: previous
                            .and_then(|s| s.asset_source.clone())
                            .or_else(|| self.asset_source.clone()),
                        resources: previous
                            .filter(|s| s.raw.get("Resources") == raw.get("Resources"))
                            .and_then(|s| s.resources.clone()),
                        in_multi_action: multi,
                        controller: if keypad { "Keypad" } else { "Encoder" }.into(),
                        index: self.index,
                        uuid: uuid.clone(),
                        name: raw["Name"].as_str().unwrap_or(&uuid).into(),
                        settings: raw
                            .get("Settings")
                            .filter(|s| !s.is_null())
                            .cloned()
                            .unwrap_or(json!({})),
                        title: states[state as usize]["Title"]
                            .as_str()
                            .unwrap_or("")
                            .into(),
                        state,
                        feedback: json!({}),
                        layout: action
                            .and_then(|a| a.raw["Encoder"]["layout"].as_str())
                            .unwrap_or(if keypad { "$X1" } else { "$B1" })
                            .into(),
                        raw,
                        image_changed: false,
                        title_changed: false,
                        pinned: false,
                        states,
                        background: None,
                        layout_cache: None,
                        image_cache: HashMap::new(),
                        display_warnings: vec![],
                        preview: String::new(),
                    }
                };
                child.in_multi_action = multi;
                child.controller = if keypad { "Keypad" } else { "Encoder" }.into();
                child.index = self.index;
                child.hydrate_children_at(actions, depth + 1)?;
                group.push(child);
            }
            hydrated.push(group);
        }
        self.raw = parent_raw;
        self.children = hydrated;
        Ok(())
    }
}
impl Host {
    pub(super) fn hydrate_library_children(&mut self) {
        let mut errors = vec![];
        for parent in self
            .library
            .profiles
            .iter_mut()
            .flat_map(|p| &mut p.pages)
            .flat_map(|p| &mut p.slots)
        {
            appearance::migrate(parent, &self.actions);
            if let Err(error) = parent.hydrate_children(&self.actions) {
                errors.push(format!(
                    "Container {} retained but invalid: {error}",
                    parent.name
                ));
            }
        }
        for parent in self
            .library
            .profiles
            .iter_mut()
            .flat_map(|p| &mut p.pages)
            .flat_map(|p| &mut p.slots)
        {
            appearance::migrate(parent, &self.actions);
        }
        for error in errors {
            if !self.logs.contains(&error) {
                self.log(error);
            }
        }
    }
    pub(super) fn visible_slot(&self, context: &str) -> Option<&Slot> {
        self.slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .find(|s| s.context == context)
    }
    pub(super) fn slot_any(&self, context: &str) -> Option<&Slot> {
        self.slots
            .iter()
            .find_map(|s| s.find(context))
            .or_else(|| self.library.slot(context))
    }
    pub(super) fn store_slot(&mut self, slot: Slot) {
        if let Some(active) = self
            .slots
            .iter_mut()
            .find_map(|s| s.find_mut(&slot.context))
        {
            *active = slot.clone();
        }
        self.library.update(&slot);
        for parent in &mut self.slots {
            parent.sync_children_raw();
        }
        for parent in self
            .library
            .profiles
            .iter_mut()
            .flat_map(|p| &mut p.pages)
            .flat_map(|p| &mut p.slots)
        {
            parent.sync_children_raw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Slot {
        let raw = json!({"UUID":crate::action_trigger::UUID,"ActionID":"parent","Settings":{},"States":[{}],"FutureRoot":1,"Actions":[
            {"Actions":[],"FutureBlank":{"keep":true}},
            {"UUID":crate::multi::ROUTINE,"ActionID":"sequence","Settings":null,"States":[{}],"FutureSequence":2,"Actions":[
                {"FutureGroup":3,"Actions":[
                    {"UUID":"test.action","ActionID":"null","Settings":null,"State":0,"States":[{"Title":"Before","FutureState":4}],"FutureLeaf":5},
                    {"UUID":"test.action","ActionID":"edit","Settings":{"value":1},"State":0,"States":[{}]}
                ]},{"Actions":[]}
            ]},{"Actions":[]}
        ]});
        serde_json::from_value(json!({"context":"root-context","controller":"Encoder","index":2,"uuid":crate::action_trigger::UUID,
            "name":"Trigger","settings":{},"title":"","state":0,"feedback":{},"layout":"$Y2","states":[{}],"raw":raw})).unwrap()
    }
    #[test]
    fn trigger_hydrates_positional_keypad_children_and_recursive_multi_flags() {
        let mut slot = fixture();
        let raw = slot.raw.clone();
        slot.hydrate_children(&[]).unwrap();
        assert_eq!(
            slot.children.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        let sequence = &slot.children[1][0];
        assert_eq!(sequence.controller, "Keypad");
        assert_eq!(sequence.index, 2);
        assert!(!sequence.in_multi_action);
        assert_eq!(sequence.settings, json!({}));
        assert_eq!(sequence.raw["Settings"], Value::Null);
        for child in &sequence.children[0] {
            assert_eq!(child.controller, "Keypad");
            assert_eq!(child.index, 2);
            assert!(child.in_multi_action);
        }
        assert_eq!(sequence.children[0][0].settings, json!({}));
        assert_eq!(slot.raw, raw);
    }
    #[test]
    fn container_edits_keep_descendant_contexts_and_refresh_coordinates() {
        let mut slot = fixture();
        slot.hydrate_children(&[]).unwrap();
        let contexts = slot
            .flatten()
            .into_iter()
            .map(|s| s.context.clone())
            .collect::<Vec<_>>();
        slot.raw["Actions"][1]["Settings"] = json!({"option":true});
        slot.raw["Actions"][1]["Actions"][0]["Actions"][1]["Settings"] = json!({"value":2});
        slot.index = 3;
        slot.hydrate_children(&[]).unwrap();
        assert_eq!(
            slot.flatten()
                .into_iter()
                .map(|s| s.context.clone())
                .collect::<Vec<_>>(),
            contexts
        );
        assert!(slot.flatten().into_iter().all(|s| s.index == 3));
        assert_eq!(slot.children[1][0].children[0][1].settings["value"], 2);
    }
    #[test]
    fn descendant_updates_sync_through_all_raw_ancestors_without_changing_blanks() {
        let mut slot = fixture();
        let blank = slot.raw["Actions"][0].clone();
        slot.hydrate_children(&[]).unwrap();
        let child = &mut slot.children[1][0].children[0][1];
        child.settings = json!({"value":77});
        slot.sync_children_raw();
        assert_eq!(
            slot.raw["Actions"][1]["Actions"][0]["Actions"][1]["Settings"]["value"],
            77
        );
        assert_eq!(
            slot.raw["Actions"][1]["Actions"][0]["Actions"][0]["Settings"],
            Value::Null
        );
        assert_eq!(slot.raw["Actions"][1]["Settings"], Value::Null);
        assert_eq!(slot.raw["Actions"][0], blank);
        assert_eq!(slot.raw["Actions"][1]["Actions"][0]["FutureGroup"], 3);
    }
    #[test]
    fn invalid_recursive_hydration_does_not_destroy_previous_contexts() {
        let mut slot = fixture();
        slot.hydrate_children(&[]).unwrap();
        let old = serde_json::to_value(&slot.children).unwrap();
        slot.raw["Actions"][1]["Actions"][0]["Actions"][0]["MultiActionDelays"] =
            json!({"Delay1":0});
        let invalid = slot.raw.clone();
        assert!(slot.hydrate_children(&[]).is_err());
        assert_eq!(serde_json::to_value(&slot.children).unwrap(), old);
        assert_eq!(slot.raw, invalid);
        assert!(slot.hydrate_children_at(&[], 9).is_err());
    }
    #[test]
    fn storing_a_nested_leaf_updates_active_and_library_raw_trees() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let mut parent = fixture();
        parent.hydrate_children(&host.actions).unwrap();
        let context = parent.children[1][0].children[0][1].context.clone();
        host.slots = vec![parent.clone()];
        host.library = Library::legacy(vec![parent], "test-device");
        let mut leaf = host.slot_any(&context).unwrap().clone();
        leaf.settings = json!({"value":123});
        host.store_slot(leaf);
        let pointer = "/Actions/1/Actions/0/Actions/1/Settings/value";
        assert_eq!(host.slots[0].raw.pointer(pointer), Some(&json!(123)));
        assert_eq!(
            host.library
                .slot("root-context")
                .unwrap()
                .raw
                .pointer(pointer),
            Some(&json!(123))
        );
        assert_eq!(
            host.library.slot(&context).unwrap().settings,
            json!({"value":123})
        );
    }
}
