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
    pub(super) fn save_multi_feedback(&mut self, request: &Value) -> Result<()> {
        let enabled = request["enabled"]
            .as_bool()
            .context("Feedback preference must be a boolean")?;
        if request["baseline"].as_bool() != Some(!self.suppress_multi_feedback) {
            bail!("Multi Action feedback preference changed; reload before saving");
        }
        let previous = std::mem::replace(&mut self.suppress_multi_feedback, !enabled);
        if let Err(error) = self.persist() {
            self.suppress_multi_feedback = previous;
            return Err(error);
        }
        for device in self.device_ids() {
            self.with_device(&device, |host| host.render())?;
        }
        Ok(())
    }
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
        if steps.is_empty() {
            self.system_status = "Multi Action has no steps".into();
            return Ok(());
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
                        self.store_slot(slot.clone());
                        self.persist()?;
                    }
                    if !self.suppress_multi_feedback {
                        self.show_feedback(&slot, crate::feedback::Kind::Ok)?;
                    } else if slot.uuid == SWITCH {
                        self.render()?;
                    }
                    self.system_status = "Multi Action finished".into();
                }
            }
        }
        Ok(())
    }
    pub(super) fn poll_multi(&mut self) {
        let previous = self.multi.progress();
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
        if !self.suppress_multi_feedback
            && self.multi.progress() != previous
            && let Err(error) = self.render()
        {
            self.log(format!("Multi Action progress redraw: {error:#}"));
        }
    }
    pub(super) fn cancel_multi(&mut self, parent: &str) {
        let had_progress = self
            .multi
            .progress()
            .get(parent)
            .is_some_and(|percent| *percent > 0);
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
        if had_progress
            && !self.suppress_multi_feedback
            && let Err(error) = self.render()
        {
            self.log(format!("Multi Action cancellation redraw: {error:#}"));
        }
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

#[cfg(test)]
mod feedback_tests {
    use super::*;
    fn setup(root: &Path, uuid: &str) -> (Host, Slot, mpsc::UnboundedReceiver<Value>) {
        let mut host = super::super::tests::fixture(root);
        let (sender, receiver) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.actions.extend(definitions());
        host.assign("Keypad", 0, uuid).unwrap();
        let context = host.slots[0].context.clone();
        host.save_multi(&context, json!({"Actions":[{"Actions":[{"UUID":"test.action","ActionID":"one","States":[{}]}]},{"Actions":[{"UUID":"test.action","ActionID":"two","States":[{}]}]}]})).unwrap();
        let parent = host.slots[0].clone();
        (host, parent, receiver)
    }
    fn finish(host: &mut Host) {
        for millis in [1000, 1100, 1200] {
            for event in host.multi.due(Duration::from_millis(millis)) {
                host.dispatch_multi(event, false).unwrap();
            }
        }
    }
    #[test]
    fn successful_dispatch_shows_parent_check_and_switches_only_after_completion() {
        for uuid in [ROUTINE, SWITCH, RANDOM] {
            let root = tempfile::tempdir().unwrap();
            let (mut host, parent, _receiver) = setup(root.path(), uuid);
            host.start_multi(&parent).unwrap();
            assert!(!host.badges.contains_key(&parent.context));
            assert_eq!(host.slots[0].state, 0);
            finish(&mut host);
            assert!(!host.multi_running(&parent.context));
            assert!(host.badges.contains_key(&parent.context));
            assert_eq!(host.slots[0].state, u64::from(uuid == SWITCH));
        }
    }
    #[test]
    fn suppressed_and_cancelled_runs_never_add_completion_feedback() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, parent, _receiver) = setup(root.path(), SWITCH);
        host.suppress_multi_feedback = true;
        host.start_multi(&parent).unwrap();
        finish(&mut host);
        assert!(host.badges.is_empty());
        assert_eq!(host.slots[0].state, 1);
        host.suppress_multi_feedback = false;
        let parent = host.slots[0].clone();
        host.start_multi(&parent).unwrap();
        host.cancel_multi(&parent.context);
        finish(&mut host);
        assert!(host.badges.is_empty());
        assert_eq!(host.slots[0].state, 1);
    }
    #[test]
    fn preference_is_global_persistent_and_rejects_stale_or_invalid_updates() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, _, _receiver) = setup(root.path(), ROUTINE);
        assert!(!host.suppress_multi_feedback);
        host.save_multi_feedback(&json!({"enabled":false,"baseline":true}))
            .unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.suppress_multi_feedback);
        assert!(
            host.save_multi_feedback(&json!({"enabled":true,"baseline":true}))
                .is_err()
        );
        assert!(
            host.save_multi_feedback(&json!({"enabled":"yes","baseline":false}))
                .is_err()
        );
        assert!(host.suppress_multi_feedback);
        host.save_multi_feedback(&json!({"enabled":true,"baseline":false}))
            .unwrap();
        assert!(!host.suppress_multi_feedback);
        host.state_path = root.path().join("not-a-state-file");
        fs::create_dir(&host.state_path).unwrap();
        assert!(
            host.save_multi_feedback(&json!({"enabled":false,"baseline":true}))
                .is_err()
        );
        assert!(
            !host.suppress_multi_feedback,
            "Failed persistence must not change the active preference"
        );
    }
    #[test]
    fn empty_switch_branch_does_not_advance_or_report_completion() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, parent, _receiver) = setup(root.path(), SWITCH);
        host.save_multi(
            &parent.context,
            json!({"Actions":[{"Actions":[]},{"Actions":[]}]}),
        )
        .unwrap();
        let parent = host.slots[0].clone();
        host.start_multi(&parent).unwrap();
        finish(&mut host);
        assert!(!host.multi_running(&parent.context));
        assert!(host.badges.is_empty());
        assert_eq!(host.slots[0].state, 0);
    }
    #[test]
    fn progress_is_in_hardware_frames_and_cancellation_restores_the_original() {
        let root = tempfile::tempdir().unwrap();
        let (mut host, parent, _receiver) = setup(root.path(), ROUTINE);
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        let frame = |receiver: &std::sync::mpsc::Receiver<Draw>| {
            receiver
                .try_iter()
                .find_map(|draw| match draw {
                    Draw::Key(0, image) => Some(image.to_rgba8()),
                    _ => None,
                })
                .unwrap()
        };
        host.render().unwrap();
        let original = frame(&receiver);
        let mut steps = host.sequence(&parent).unwrap().groups[0].clone();
        let mut second = steps[0].clone();
        second.id = "second".into();
        steps.push(second);
        host.multi
            .start(&parent.context, steps, Duration::ZERO)
            .unwrap();
        host.multi.due(Duration::ZERO);
        host.multi.due(Duration::from_millis(100));
        host.multi.due(Duration::from_millis(200));
        assert_eq!(host.multi.progress()[&parent.context], 50);
        host.render().unwrap();
        let partial = frame(&receiver);
        assert_ne!(partial, original);
        for y in 0..original.height() {
            for x in 0..original.width() / 2 {
                assert_eq!(partial.get_pixel(x, y), original.get_pixel(x, y));
            }
        }
        host.cancel_multi(&parent.context);
        assert_eq!(frame(&receiver), original);
        assert!(host.badges.is_empty());
    }
}
