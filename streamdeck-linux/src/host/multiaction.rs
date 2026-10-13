//! Multi Action child ownership and nonblocking sequence dispatch.
use super::*;
use crate::multi::{Entry, Event, Sequence};

pub(super) const ROUTINE: &str = "com.elgato.streamdeck.multiactions.routine";
pub(super) const SWITCH: &str = "com.elgato.streamdeck.multiactions.routine2";
pub(super) const RANDOM: &str = "com.elgato.streamdeck.multiactions.random";
pub(super) const DELAY: &str = "com.elgato.streamdeck.multiactions.delay";
pub(super) fn container(uuid: &str) -> bool {
    matches!(uuid, ROUTINE | SWITCH | RANDOM)
}
pub(super) fn definitions() -> Vec<Action> {
    [(ROUTINE,"Multi Action"),(SWITCH,"Multi Action Switch"),(RANDOM,"Random Action"),(DELAY,"Delay")].into_iter().map(|(uuid,name)|Action{
        uuid:uuid.into(),name:name.into(),plugin:"deckforge.multi".into(),controllers:vec!["Keypad".into()],
        raw:json!({"SupportedInMultiActions":uuid==DELAY,"States":if uuid==SWITCH {json!([{"Title":"Sequence 1"},{"Title":"Sequence 2"}])}else{json!([{"Title":name}])}}),
    }).collect()
}
impl Host {
    fn sequence(&self, parent: &Slot) -> Result<Sequence> {
        let mut sequence = Sequence::parse(&parent.uuid, &parent.raw)?;
        for entry in sequence.groups.iter_mut().flatten() {
            let action = self
                .actions
                .iter()
                .find(|a| a.uuid == entry.uuid)
                .with_context(|| format!("Multi Action needs unavailable action {}", entry.uuid))?;
            ensure_multi_allowed(action)?;
            if entry.raw.get("MultiActionDelays").is_none()
                && let Some(timing) = action.raw.get("MultiActionDelays")
            {
                entry.press_ms = timing["Delay1"]
                    .as_u64()
                    .filter(|v| (1..=100).contains(v))
                    .context("Invalid manifest virtual press delay")?;
                entry.gap_ms = timing["Delay2"]
                    .as_u64()
                    .filter(|v| (1..=100).contains(v))
                    .context("Invalid manifest next-action delay")?;
            }
        }
        Ok(sequence)
    }
    pub(super) fn start_multi(&mut self, parent: &Slot) -> Result<()> {
        let sequence = self.sequence(parent)?;
        let branch = if parent.uuid == SWITCH {
            (parent.state % 2) as usize
        } else {
            0
        };
        let mut steps = sequence.groups.get(branch).cloned().unwrap_or_default();
        if parent.uuid == RANDOM && !steps.is_empty() {
            // Rejection sampling avoids modulo bias and permits repeated choices.
            let n = steps.len() as u64;
            let range = 1u64 << 48;
            let cutoff = range - range % n;
            let index = loop {
                let id = uuid::Uuid::new_v4();
                let random = id.as_bytes()[..6]
                    .iter()
                    .fold(0u64, |v, b| (v << 8) | u64::from(*b));
                if random < cutoff {
                    break (random % n) as usize;
                }
            };
            steps = vec![steps.remove(index)];
        }
        let needs_keyboard = steps.iter().any(|e| {
            matches!(
                e.uuid.as_str(),
                crate::system_actions::HOTKEY | crate::system_actions::MULTIMEDIA
            )
        });
        if needs_keyboard {
            let worker = self
                .system_actions
                .get_or_insert_with(crate::system_actions::Worker::new);
            if !worker.keyboard_ready() {
                if !self.multi_preparing {
                    worker.submit(
                        &parent.context,
                        crate::system_actions::Plan::PrepareKeyboard,
                    )?;
                    self.multi_preparing = true;
                }
                self.multi_owned.insert(parent.context.clone());
                self.multi_pending.insert(parent.context.clone(), steps);
                self.system_status = "Preparing keyboard for Multi Action".into();
                return Ok(());
            }
        }
        self.multi
            .start(&parent.context, steps, self.multi_clock.elapsed())?;
        self.multi_owned.insert(parent.context.clone());
        self.multi_notify.notify_one();
        Ok(())
    }
    pub(super) fn multi_running(&self, context: &str) -> bool {
        self.multi_owned.contains(context) || self.multi.running(context)
    }
    pub(super) fn multi_keyboard_prepared(&mut self, success: bool) {
        self.multi_preparing = false;
        let pending = std::mem::take(&mut self.multi_pending);
        if !success {
            for parent in pending.keys() {
                self.multi_owned.remove(parent);
            }
        }
        if success {
            for (parent, steps) in pending {
                if self.visible_slot(&parent).is_some()
                    && let Err(error) = self.multi.start(&parent, steps, self.multi_clock.elapsed())
                {
                    self.multi_owned.remove(&parent);
                    self.log(format!("Multi Action start failed: {error}"));
                }
            }
            self.multi_notify.notify_one();
        }
    }
    fn child_for(&self, parent: &str, entry: &Entry) -> Result<Slot> {
        self.slot_any(parent)
            .and_then(|p| p.children.iter().flatten().find(|s| s.child_id == entry.id))
            .cloned()
            .context("Multi Action step disappeared")
    }
    pub(super) fn dispatch_multi(&mut self, event: Event, cancelled: bool) -> Result<()> {
        match event {
            Event::Press { parent, entry } => {
                if !self.multi_owned.contains(&parent) {
                    return Ok(());
                }
                let child = self.child_for(&parent, &entry)?;
                self.virtual_action(&child, true, false)?;
                self.multi_held.insert(parent, entry);
            }
            Event::Release { parent, entry } => {
                if self
                    .multi_held
                    .get(&parent)
                    .is_none_or(|held| held.id != entry.id)
                {
                    return Ok(());
                }
                self.multi_held.remove(&parent);
                let child = self.child_for(&parent, &entry)?;
                self.virtual_action(&child, false, cancelled)?;
            }
            Event::Finished { parent } => {
                if !self.multi_owned.remove(&parent) {
                    return Ok(());
                }
                if let Some(mut slot) = self.slot_any(&parent).cloned() {
                    if slot.uuid == SWITCH {
                        slot.state = (slot.state + 1) % 2;
                        slot.title = appearance::title(&slot, &self.actions);
                        slot.background = self.resolve_state_image(&slot, slot.state);
                        slot.title_changed = false;
                        slot.image_changed = false;
                        self.store_slot(slot);
                        self.render()?;
                        self.persist()?;
                    }
                    self.system_status = "Multi Action finished".into();
                }
            }
        }
        Ok(())
    }
    pub(super) fn poll_multi(&mut self) {
        let events = self.multi.due(self.multi_clock.elapsed());
        for event in events {
            let parent = match &event {
                Event::Press { parent, .. }
                | Event::Release { parent, .. }
                | Event::Finished { parent } => parent.clone(),
            };
            if let Err(error) = self.dispatch_multi(event, false) {
                self.cancel_multi(&parent);
                self.system_status = format!("Multi Action failed: {error:#}");
                self.log(self.system_status.clone());
            }
        }
    }
    pub(super) fn cancel_multi(&mut self, parent: &str) {
        self.multi_pending.remove(parent);
        self.multi_owned.remove(parent);
        let _ = self.multi.cancel(parent);
        // Scheduler events may already be in a due batch. Release only what the
        // host actually dispatched, never a queued-but-unexecuted press.
        if let Some(entry) = self.multi_held.get(parent).cloned()
            && let Err(error) = self.dispatch_multi(
                Event::Release {
                    parent: parent.into(),
                    entry,
                },
                true,
            )
        {
            self.log(format!("Multi Action release: {error:#}"));
        }
        self.multi_notify.notify_one();
    }
    pub(super) fn cancel_departing_multi(&mut self, visible: &HashSet<String>) {
        let parents: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| !visible.contains(&s.context) && container(&s.uuid))
            .map(|s| s.context.clone())
            .collect();
        for parent in parents {
            self.cancel_multi(&parent);
        }
    }
    pub(super) fn cancel_all_multi(&mut self) {
        let parents: Vec<_> = self.multi_owned.iter().cloned().collect();
        for parent in parents {
            self.cancel_multi(&parent);
        }
        self.multi_pending.clear();
        let _ = self.multi.cancel_all();
        self.multi_notify.notify_one();
    }
    pub(super) fn virtual_action(
        &mut self,
        slot: &Slot,
        pressed: bool,
        cancelled: bool,
    ) -> Result<()> {
        if container(&slot.uuid) {
            if cancelled {
                self.cancel_multi(&slot.context);
                self.virtual_suppressed.remove(&slot.context);
            } else if pressed {
                if self.multi_running(&slot.context) {
                    self.cancel_multi(&slot.context);
                    self.virtual_suppressed.insert(slot.context.clone());
                }
            } else if !self.virtual_suppressed.remove(&slot.context) {
                self.start_multi(slot)?;
            }
            return Ok(());
        }
        if mixer_actions::supported(&slot.uuid) || crate::system_actions::supported(&slot.uuid) {
            if cancelled
                && !matches!(
                    slot.uuid.as_str(),
                    crate::system_actions::HOTKEY | crate::brightness::UUID
                )
            {
                return Ok(());
            }
            return self.run_system_action(
                slot,
                if pressed {
                    crate::system_actions::Trigger::Down
                } else {
                    crate::system_actions::Trigger::Up
                },
            );
        }
        if crate::audio_output::supported(&slot.uuid) {
            if !pressed && !cancelled {
                self.soundboard_action(slot)?;
            }
            return Ok(());
        }
        if !pressed && !cancelled {
            match slot.uuid.as_str() {
                library::OPEN_FOLDER => {
                    return self.select_page(
                        slot.settings["ProfileUUID"]
                            .as_str()
                            .context("Folder target missing")?,
                        true,
                    );
                }
                library::BACK_FOLDER => return self.folder_back(),
                "com.elgato.streamdeck.page.goto" => return self.goto_page(&slot.settings),
                "com.elgato.streamdeck.page.next" => return self.step_page(1),
                "com.elgato.streamdeck.page.previous" => return self.step_page(-1),
                _ => {}
            }
        }
        if let Some(action) = self.actions.iter().find(|a| a.uuid == slot.uuid) {
            if action.plugin.starts_with("deckforge.") {
                return Ok(());
            }
            if !self
                .plugins
                .iter()
                .any(|p| p.id == action.plugin && p.sender.is_some())
            {
                bail!("Plugin for {} is disconnected", slot.name);
            }
        }
        self.emit(
            slot,
            if pressed { "keyDown" } else { "keyUp" },
            if slot.in_multi_action {
                json!({"userDesiredState":slot.raw["OverrideState"].as_u64().unwrap_or(0)})
            } else {
                json!({})
            },
        );
        if !pressed && !cancelled {
            self.advance_key_state(slot)?;
        }
        Ok(())
    }
    pub(super) fn save_multi(&mut self, context: &str, raw: Value) -> Result<()> {
        self.editable()?;
        let mut slot = self
            .visible_slot(context)
            .cloned()
            .context("Multi Action is no longer visible")?;
        if !container(&slot.uuid) {
            bail!("This control is not a Multi Action");
        }
        let mut candidate = slot.clone();
        candidate.raw = raw;
        Sequence::parse(&candidate.uuid, &candidate.raw)?;
        // Preserve unsupported imported steps; execution validates availability.
        candidate.hydrate_children(&self.actions)?;
        self.cancel_triggers_for_child(context);
        self.cancel_multi(context);
        // Stable instances remain; removed/replaced children receive lifecycle end.
        let old: HashSet<_> = slot.flatten().iter().map(|s| s.context.clone()).collect();
        let new: HashSet<_> = candidate
            .flatten()
            .iter()
            .map(|s| s.context.clone())
            .collect();
        for child in slot.children.iter().flatten() {
            if !new.contains(&child.context) {
                self.close_inspectors(Some(&child.context));
                self.emit(child, "willDisappear", json!({}));
                self.remember_removed(&child.context, &child.uuid);
            }
        }
        slot = candidate;
        self.store_slot(slot.clone());
        self.persist()?;
        for child in slot.children.iter().flatten() {
            if !old.contains(&child.context) {
                self.emit(child, "willAppear", json!({}));
            } else {
                self.emit(child, "didReceiveSettings", json!({}));
                self.notify_inspectors(&child.context, self.settings_event(child, None), None);
            }
        }
        self.render()
    }
}
fn ensure_multi_allowed(action: &Action) -> Result<()> {
    if action.raw["SupportedInMultiActions"] == false
        || !action.controllers.iter().any(|c| c == "Keypad")
        || container(&action.uuid)
    {
        bail!("{} does not support Multi Actions", action.name);
    }
    Ok(())
}
