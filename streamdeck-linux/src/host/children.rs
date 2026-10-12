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
        let multi = multiaction::container(&self.uuid);
        let stack = self.uuid == crate::dial_stack::UUID;
        if !multi && !stack {
            return;
        }
        for (branch, children) in self.children.iter_mut().enumerate() {
            for (index, child) in children.iter_mut().enumerate() {
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
                {
                    state["Title"] = json!(child.title);
                }
                let record = if stack {
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
        let multi = multiaction::container(&self.uuid);
        let stack = self.uuid == crate::dial_stack::UUID;
        if !multi && !stack {
            return Ok(());
        }
        if self.raw.get("Actions").is_none() {
            self.raw["Actions"] = if multi {
                json!([{"Actions":[]},{"Actions":[]}])
            } else {
                json!([])
            };
        }
        let groups: Vec<Vec<(String, String, Value)>> = if multi {
            crate::multi::Sequence::parse(&self.uuid, &self.raw)?
                .groups
                .into_iter()
                .map(|g| g.into_iter().map(|e| (e.id, e.uuid, e.raw)).collect())
                .collect()
        } else {
            let mut raw = self.raw.clone();
            raw["Settings"] = self.settings.clone();
            vec![
                crate::dial_stack::parse(&raw)?
                    .children
                    .into_iter()
                    .map(|e| (e.id, e.uuid, e.raw))
                    .collect(),
            ]
        };
        let old = std::mem::take(&mut self.children);
        self.children = groups
            .into_iter()
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|(id, uuid, raw)| {
                        let previous = old
                            .iter()
                            .flatten()
                            .find(|s| s.child_id == id && s.uuid == uuid);
                        if let Some(existing) = previous.filter(|s| s.raw == raw) {
                            let mut child = existing.clone();
                            child.in_multi_action = multi;
                            child.controller = if multi { "Keypad" } else { "Encoder" }.into();
                            child.index = self.index;
                            return child;
                        }
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
                            children: vec![],
                            child_id: id,
                            in_multi_action: multi,
                            controller: if multi { "Keypad" } else { "Encoder" }.into(),
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
                                .unwrap_or(if multi { "$X1" } else { "$B1" })
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
                    })
                    .collect()
            })
            .collect();
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
            if let Err(error) = parent.hydrate_children(&self.actions) {
                errors.push(format!(
                    "Container {} retained but invalid: {error}",
                    parent.name
                ));
            }
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
