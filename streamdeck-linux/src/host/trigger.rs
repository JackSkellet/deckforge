//! Action Trigger: ordinary key instances driven by dial gestures.
use super::*;
use crate::action_trigger::{self, Event, Repeater};

const DIRECT: u8 = 1;
const PULSE: u8 = 2;
#[derive(Default)]
pub(super) struct Runtime {
    pub repeater: Repeater,
    targets: HashMap<String, usize>,
    // Store the dispatched instance, not a later edited replacement.
    held: HashMap<(String, usize), (Slot, u8)>,
    pending: HashMap<String, (usize, u8)>,
}
pub(super) fn action() -> Action {
    Action {
        uuid: action_trigger::UUID.into(),
        name: "Action Trigger".into(),
        plugin: "deckforge.triggers".into(),
        controllers: vec!["Encoder".into()],
        raw: json!({"SupportedInMultiActions":false,"Encoder":{"layout":"$X1"},"States":[{"Title":"Action Trigger"}]}),
    }
}
pub(super) fn picture(
    renderer: &Renderer,
    actions: &[Action],
    slot: &mut Slot,
    badges: &HashMap<String, feedback::Badge>,
    now: Duration,
) -> Result<DynamicImage> {
    let mut canvas = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        200,
        100,
        image::Rgba([14, 21, 29, 255]),
    ));
    for (index, (x, y)) in [(12, 29), (72, 14), (132, 29)].into_iter().enumerate() {
        if let Some(child) = slot.children.get_mut(index).and_then(|g| g.first_mut()) {
            let mut key = renderer.key_styled(
                &child.title,
                child.background.as_ref(),
                &appearance::style(child, actions),
            );
            feedback::paint_key(badges, &mut key, child, now);
            let mut png = Cursor::new(Vec::new());
            key.write_to(&mut png, image::ImageFormat::Png)?;
            child.preview = STANDARD.encode(png.into_inner());
            let mut key = key
                .resize_exact(56, 56, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            if index != 1 {
                for pixel in key.pixels_mut() {
                    pixel[3] = (u16::from(pixel[3]) * 3 / 5) as u8;
                }
            }
            image::imageops::overlay(&mut canvas, &key, x, y);
        }
    }
    Ok(canvas)
}
impl Host {
    fn trigger_child(&self, parent: &str, index: usize) -> Option<Slot> {
        self.visible_slot(parent)
            .filter(|s| s.uuid == action_trigger::UUID)
            .and_then(|s| s.children.get(index))
            .and_then(|g| g.first())
            .cloned()
    }
    fn trigger_edge(
        &mut self,
        parent: &str,
        index: usize,
        owner: u8,
        pressed: bool,
        cancelled: bool,
    ) -> Result<()> {
        let key = (parent.to_owned(), index);
        if pressed {
            if let Some((_, owners)) = self.triggers.held.get_mut(&key) {
                *owners |= owner;
                return Ok(());
            }
            let Some(child) = self.trigger_child(parent, index) else {
                return Ok(());
            };
            let action = self
                .actions
                .iter()
                .find(|a| a.uuid == child.uuid)
                .context("Action Trigger child is unavailable")?;
            if !action.controllers.iter().any(|c| c == "Keypad") {
                bail!("Action Trigger requires a key action");
            }
            self.virtual_action(&child, true, false)?;
            // A dispatched keyDown can cause navigation; do not retain a hidden owner.
            if self.trigger_child(parent, index).is_some() {
                self.triggers.held.insert(key, (child, owner));
            } else {
                self.virtual_action(&child, false, true)?;
            }
        } else if let Some((_, owners)) = self.triggers.held.get_mut(&key) {
            *owners &= !owner;
            if *owners == 0 {
                let (held, _) = self.triggers.held.remove(&key).unwrap();
                let child = self.visible_slot(&held.context).cloned().unwrap_or(held);
                self.virtual_action(&child, false, cancelled)?;
            }
        }
        Ok(())
    }
    pub(super) fn trigger_button(&mut self, slot: &Slot, pressed: bool) -> Result<()> {
        self.trigger_edge(&slot.context, 1, DIRECT, pressed, false)
    }
    pub(super) fn trigger_input(&mut self, slot: &Slot, name: &str, payload: Value) -> Result<()> {
        let (index, count) = if name == "dialRotate" {
            let ticks = i8::try_from(payload["ticks"].as_i64().context("Missing dial ticks")?)?;
            if ticks == 0 {
                return Ok(());
            }
            (if ticks < 0 { 0 } else { 2 }, ticks.unsigned_abs())
        } else {
            let x = u16::try_from(payload["tapPos"][0].as_u64().context("Missing tap x")?)?;
            let y = u16::try_from(payload["tapPos"][1].as_u64().context("Missing tap y")?)?;
            let Some(index) = action_trigger::point(x, y) else {
                return Ok(());
            };
            (index, 1)
        };
        let Some(child) = self.trigger_child(&slot.context, index) else {
            return Ok(());
        };
        if matches!(
            child.uuid.as_str(),
            crate::system_actions::HOTKEY | crate::system_actions::MULTIMEDIA
        ) {
            let worker = self
                .system_actions
                .get_or_insert_with(crate::system_actions::Worker::new);
            if !worker.keyboard_ready() {
                if !self.multi_preparing {
                    worker.submit(&slot.context, crate::system_actions::Plan::PrepareKeyboard)?;
                    self.multi_preparing = true;
                }
                let pending = self
                    .triggers
                    .pending
                    .entry(slot.context.clone())
                    .or_insert((index, 0));
                if pending.0 != index {
                    *pending = (index, 0);
                }
                pending.1 = pending.1.saturating_add(count).min(3);
                return Ok(());
            }
        }
        self.trigger_request(&slot.context, index, count)
    }
    fn trigger_request(&mut self, parent: &str, index: usize, count: u8) -> Result<()> {
        self.triggers.targets.insert(parent.into(), index);
        let events =
            self.triggers
                .repeater
                .request(parent, index, count, self.multi_clock.elapsed());
        self.dispatch_triggers(events)?;
        self.multi_notify.notify_one();
        Ok(())
    }
    pub(super) fn trigger_keyboard_prepared(&mut self, success: bool) {
        let pending = std::mem::take(&mut self.triggers.pending);
        if success {
            for (parent, (index, count)) in pending {
                if self.trigger_child(&parent, index).is_some()
                    && let Err(error) = self.trigger_request(&parent, index, count)
                {
                    self.cancel_trigger(&parent);
                    self.log(format!("Action Trigger: {error:#}"));
                }
            }
        }
    }
    fn dispatch_triggers(&mut self, events: Vec<Event>) -> Result<()> {
        let mut first_error = None;
        for event in events {
            let (parent, index, pressed) = match event {
                Event::Press { parent, index } => (parent, index, true),
                Event::Release { parent, index } => (parent, index, false),
            };
            if pressed && self.triggers.targets.get(&parent) != Some(&index) {
                continue;
            }
            if let Err(error) = self.trigger_edge(&parent, index, PULSE, pressed, false) {
                self.cancel_trigger(&parent);
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    pub(super) fn poll_triggers(&mut self) {
        let events = self.triggers.repeater.due(self.multi_clock.elapsed());
        if let Err(error) = self.dispatch_triggers(events) {
            self.log(format!("Action Trigger: {error:#}"));
        }
    }
    pub(super) fn disconnect_trigger_buttons(&mut self) {
        let held: Vec<_> = self
            .triggers
            .held
            .iter()
            .filter(|(_, (_, owners))| *owners & DIRECT != 0)
            .map(|((parent, index), _)| (parent.clone(), *index))
            .collect();
        for (parent, index) in held {
            if let Err(error) = self.trigger_edge(&parent, index, DIRECT, false, true) {
                self.log(format!("Disconnected dial release: {error:#}"));
            }
        }
    }
    pub(super) fn cancel_trigger(&mut self, parent: &str) {
        self.triggers.targets.remove(parent);
        self.triggers.pending.remove(parent);
        let _ = self.triggers.repeater.cancel(parent);
        let keys: Vec<_> = self
            .triggers
            .held
            .keys()
            .filter(|(p, _)| p == parent)
            .cloned()
            .collect();
        for key in keys {
            if let Some((child, _)) = self.triggers.held.remove(&key)
                && let Err(error) = self.virtual_action(&child, false, true)
            {
                self.log(format!("Action Trigger release: {error:#}"));
            }
        }
        self.multi_notify.notify_one();
    }
    pub(super) fn cancel_triggers_for_child(&mut self, context: &str) {
        let parents: HashSet<_> = self
            .triggers
            .targets
            .keys()
            .chain(self.triggers.pending.keys())
            .chain(self.triggers.held.keys().map(|(parent, _)| parent))
            .filter(|parent| {
                self.slot_any(parent)
                    .is_some_and(|slot| slot.find(context).is_some())
            })
            .cloned()
            .collect();
        for parent in parents {
            self.cancel_trigger(&parent);
        }
    }
    pub(super) fn cancel_changed_triggers(&mut self, next: &[Slot]) {
        let changed: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| s.uuid == action_trigger::UUID)
            .filter(|old| {
                next.iter()
                    .flat_map(Slot::visible_flatten)
                    .find(|s| s.context == old.context)
                    .is_some_and(|new| new.raw != old.raw)
            })
            .map(|s| s.context.clone())
            .collect();
        for parent in changed {
            self.cancel_trigger(&parent);
            let sequences: Vec<_> = self
                .slot_any(&parent)
                .into_iter()
                .flat_map(Slot::flatten)
                .filter(|s| multiaction::container(&s.uuid))
                .map(|s| s.context.clone())
                .collect();
            for context in sequences {
                self.cancel_multi(&context);
            }
        }
    }
    pub(super) fn cancel_departing_triggers(&mut self, visible: &HashSet<String>) {
        let parents: HashSet<_> = self
            .triggers
            .targets
            .keys()
            .chain(self.triggers.pending.keys())
            .chain(self.triggers.held.keys().map(|(parent, _)| parent))
            .filter(|p| !visible.contains(*p))
            .cloned()
            .collect();
        for parent in parents {
            self.cancel_trigger(&parent);
        }
    }
    pub(super) fn save_trigger(&mut self, context: &str, raw: Value) -> Result<()> {
        self.editable()?;
        let mut parent = self
            .visible_slot(context)
            .cloned()
            .context("Action Trigger is no longer visible")?;
        if parent.uuid != action_trigger::UUID {
            bail!("This control is not an Action Trigger");
        }
        action_trigger::parse(&raw)?;
        let old = parent.clone();
        parent.raw = raw;
        parent.settings = parent
            .raw
            .get("Settings")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or(json!({}));
        parent.hydrate_children(&self.actions)?;
        for child in parent.children.iter().flatten() {
            if self
                .actions
                .iter()
                .find(|a| a.uuid == child.uuid)
                .is_some_and(|a| !a.controllers.iter().any(|c| c == "Keypad"))
            {
                bail!("{} does not support keys", child.name);
            }
        }
        self.cancel_trigger(context);
        for child in old.flatten() {
            if multiaction::container(&child.uuid) {
                self.cancel_multi(&child.context);
            }
        }
        self.sync_visible();
        self.library.update(&parent);
        self.refresh_page()?;
        for child in parent.flatten().into_iter().skip(1) {
            if old
                .find(&child.context)
                .is_some_and(|s| s.settings != child.settings)
            {
                self.emit(child, "didReceiveSettings", json!({}));
                self.notify_inspectors(&child.context, self.settings_event(child, None), None);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (
        tempfile::TempDir,
        Host,
        Slot,
        mpsc::UnboundedReceiver<Value>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.actions.push(action());
        host.actions[0].raw["SupportedInMultiActions"] = json!(false);
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.assign("Encoder", 2, action_trigger::UUID).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.uuid == action_trigger::UUID)
            .unwrap()
            .context
            .clone();
        host.save_trigger(&parent,json!({"Actions":[{"UUID":"test.action","ActionID":"left","Settings":{}},{"UUID":"test.action","ActionID":"middle","Settings":{}},{"UUID":"test.action","ActionID":"right","Settings":{}}]})).unwrap();
        let slot = host.visible_slot(&parent).unwrap().clone();
        while rx.try_recv().is_ok() {}
        (temp, host, slot, rx)
    }
    #[test]
    fn direct_and_touch_ownership_are_balanced_and_use_normal_key_payloads() {
        let (_temp, mut host, parent, mut rx) = setup();
        host.trigger_button(&parent, true).unwrap();
        let down = rx.try_recv().unwrap();
        assert_eq!(down["event"], "keyDown");
        assert_eq!(down["payload"]["controller"], "Keypad");
        assert_eq!(down["payload"]["coordinates"], json!({"column":2,"row":0}));
        assert_eq!(down["payload"]["isInMultiAction"], false);
        assert!(down["payload"].get("userDesiredState").is_none());
        host.trigger_request(&parent.context, 1, 1).unwrap();
        host.trigger_button(&parent, false).unwrap();
        assert!(rx.try_recv().is_err());
        let events = host.triggers.repeater.due(Duration::from_secs(2));
        host.dispatch_triggers(events).unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        host.cancel_trigger(&parent.context);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn cancelled_prefetched_pulse_never_presses_or_releases_a_new_child() {
        let (_temp, mut host, parent, mut rx) = setup();
        host.trigger_request(&parent.context, 0, 2).unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        let events = host.triggers.repeater.due(Duration::from_secs(2));
        host.dispatch_triggers(events).unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        let events = host.triggers.repeater.due(Duration::from_secs(3));
        host.cancel_trigger(&parent.context);
        host.dispatch_triggers(events).unwrap();
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn editing_held_child_releases_old_instance_and_cancels_keyboard_wait() {
        let (_temp, mut host, parent, mut rx) = setup();
        host.trigger_button(&parent, true).unwrap();
        let down = rx.try_recv().unwrap();
        host.triggers.pending.insert(parent.context.clone(), (0, 3));
        host.save_trigger(
            &parent.context,
            json!({"Actions":[{"Actions":[]},{"Actions":[]},{"Actions":[]}]}),
        )
        .unwrap();
        let up = rx.try_recv().unwrap();
        assert_eq!(up["event"], "keyUp");
        assert_eq!(up["context"], down["context"]);
        host.trigger_keyboard_prepared(true);
        assert!(host.triggers.repeater.next_due().is_none());
        assert!(host.triggers.held.is_empty());
        assert_eq!(
            host.visible_slot(&parent.context).unwrap().children.len(),
            3
        );
    }
    #[test]
    fn changing_sequence_through_stack_cancels_its_existing_run() {
        let (_temp, mut host, _, mut rx) = setup();
        host.actions[0].raw["SupportedInMultiActions"] = json!(true);
        host.actions.extend(multiaction::definitions());
        host.actions.push(stack::action());
        host.assign("Encoder", 0, crate::dial_stack::UUID).unwrap();
        let context = host
            .slots
            .iter()
            .find(|s| s.uuid == crate::dial_stack::UUID)
            .unwrap()
            .context
            .clone();
        let mut raw = json!({"Actions":[{"UUID":action_trigger::UUID,"ActionID":"trigger","Actions":[{"UUID":multiaction::ROUTINE,"ActionID":"sequence","Actions":[{"Actions":[{"UUID":"test.action","ActionID":"step","Settings":{"before":true}}]}]},{"Actions":[]},{"Actions":[]}]}]});
        host.save_stack(&context, raw.clone()).unwrap();
        let sequence = host.visible_slot(&context).unwrap().children[0][0].children[0][0].clone();
        while rx.try_recv().is_ok() {}
        host.start_multi(&sequence).unwrap();
        host.poll_multi();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        raw["Actions"][0]["Actions"][0]["Actions"][0]["Actions"][0]["Settings"] =
            json!({"after":true});
        host.save_stack(&context, raw).unwrap();
        assert!(!host.multi_running(&sequence.context));
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        assert_eq!(
            host.visible_slot(&context).unwrap().children[0][0].children[0][0].context,
            sequence.context
        );
    }
    #[test]
    fn editing_a_nested_sequence_during_pulse_gap_drops_remaining_work() {
        let (_temp, mut host, parent, mut rx) = setup();
        host.actions[0].raw["SupportedInMultiActions"] = json!(true);
        host.actions.extend(multiaction::definitions());
        let raw = json!({"Actions":[{"UUID":multiaction::ROUTINE,"ActionID":"sequence","Actions":[{"Actions":[{"UUID":"test.action","Settings":{}}]}]},{"Actions":[]},{"Actions":[]}]});
        host.save_trigger(&parent.context, raw).unwrap();
        let sequence = host.visible_slot(&parent.context).unwrap().children[0][0].clone();
        while rx.try_recv().is_ok() {}
        host.trigger_request(&parent.context, 0, 2).unwrap();
        let events = host.triggers.repeater.due(Duration::from_secs(2));
        host.dispatch_triggers(events).unwrap();
        assert!(host.multi_running(&sequence.context));
        assert!(host.triggers.held.is_empty());
        host.save_multi(&sequence.context, sequence.raw.clone())
            .unwrap();
        assert!(!host.multi_running(&sequence.context));
        let events = host.triggers.repeater.due(Duration::from_secs(3));
        assert!(events.is_empty());
        host.poll_multi();
        assert!(!host.multi_running(&sequence.context));
        while let Ok(event) = rx.try_recv() {
            assert_ne!(event["event"], "keyDown");
        }
    }
    #[test]
    fn nested_multi_starts_on_release_and_second_press_cancels_without_restart() {
        let (_temp, mut host, parent, mut rx) = setup();
        host.actions[0].raw["SupportedInMultiActions"] = json!(true);
        host.actions.extend(multiaction::definitions());
        host.save_trigger(&parent.context,json!({"Actions":[{"Actions":[]},{"UUID":multiaction::ROUTINE,"ActionID":"sequence","Actions":[{"Actions":[{"UUID":"test.action","Settings":{}}]}]},{"Actions":[]}]})).unwrap();
        let parent = host.visible_slot(&parent.context).unwrap().clone();
        let sequence = &parent.children[1][0];
        while rx.try_recv().is_ok() {}
        host.trigger_button(&parent, true).unwrap();
        assert!(!host.multi_running(&sequence.context));
        host.trigger_button(&parent, false).unwrap();
        host.poll_multi();
        let down = rx.try_recv().unwrap();
        assert_eq!(down["event"], "keyDown");
        assert_eq!(down["payload"]["isInMultiAction"], true);
        host.trigger_button(&parent, true).unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        host.trigger_button(&parent, false).unwrap();
        host.poll_multi();
        assert!(!host.multi_running(&sequence.context));
        assert!(rx.try_recv().is_err());
    }
}
