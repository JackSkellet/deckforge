//! Original action-level AutoExit settings and device-owned folder deadlines.
use super::*;

pub(super) struct BackPress {
    started: Duration,
    profile: String,
    page: String,
    folders: Vec<String>,
}
impl BackPress {
    fn held_long(&self, now: Duration) -> bool {
        now.saturating_sub(self.started) > Duration::from_millis(500)
    }
    fn belongs_to(&self, library: &Library) -> bool {
        self.profile == library.active_profile
            && self.page == library.active_page
            && self.folders == library.folders
    }
}

#[derive(Clone)]
pub(super) struct Pending {
    profile: String,
    page: String,
    folders: Vec<String>,
    back: String,
    pub due: Duration,
    started: Duration,
    last_percent: u8,
    next_frame: Duration,
}

impl Pending {
    fn percent(&self, now: Duration) -> u8 {
        let total = self.due.saturating_sub(self.started).as_secs_f64();
        if total <= 0.0 {
            return 100;
        }
        (now.saturating_sub(self.started).as_secs_f64() / total * 100.0).clamp(0.0, 100.0) as u8
    }
}

fn excluded(uuid: &str) -> bool {
    matches!(
        uuid,
        library::OPEN_FOLDER
            | library::BACK_FOLDER
            | "com.elgato.streamdeck.page.next"
            | "com.elgato.streamdeck.page.previous"
            | "com.elgato.streamdeck.page.goto"
            | "com.elgato.streamdeck.page.indicator"
    )
}

fn delay(raw: &Value, parent: &Value) -> Option<Duration> {
    // Absence means inheritance; an explicit disabled object overrides the folder.
    let value = raw
        .get("AutoExit")
        .filter(|v| !v.is_null())
        .unwrap_or(&parent["AutoExit"]);
    if value["Enabled"] != true {
        return None;
    }
    // Imported records are kept verbatim. Invalid or out-of-range delays never
    // become an accidental immediate exit. New edits use the native 0..60 range.
    let seconds = value.get("Delay").map_or(Some(0), Value::as_u64)?;
    (seconds <= 60).then(|| Duration::from_secs(seconds))
}

impl Host {
    pub(super) fn folder_back_button(
        &mut self,
        context: &str,
        pressed: bool,
        owned_release: bool,
    ) -> Result<()> {
        if pressed {
            self.folder_back_holds
                .entry(context.into())
                .or_insert_with(|| BackPress {
                    started: self.multi_clock.elapsed(),
                    profile: self.library.active_profile.clone(),
                    page: self.library.active_page.clone(),
                    folders: self.library.folders.clone(),
                });
        } else if let Some(hold) = self.folder_back_holds.remove(context)
            && owned_release
            && hold.belongs_to(&self.library)
        {
            if hold.held_long(self.multi_clock.elapsed()) {
                if let Some(root) = self.library.folders.first().cloned() {
                    self.select_page(&root, false)?;
                }
            } else {
                self.folder_back()?;
            }
        }
        Ok(())
    }
    pub(super) fn reset_folder_exits(&mut self, request: &Value) -> Result<()> {
        self.editable()?;
        let context = request["context"].as_str().context("Back action missing")?;
        anyhow::ensure!(
            request["confirm"] == true,
            "Confirm resetting action overrides"
        );
        anyhow::ensure!(
            !self.library.folders.is_empty()
                && request["page"].as_str() == Some(self.library.active_page.as_str())
                && self
                    .slots
                    .iter()
                    .any(|s| s.context == context && s.uuid == library::BACK_FOLDER),
            "The folder changed; reopen its Back action before resetting overrides"
        );
        self.sync_visible();
        let originals: Vec<_> = self
            .library
            .current()?
            .pages
            .iter()
            .find(|p| p.id == self.library.active_page)
            .context("Folder missing")?
            .slots
            .iter()
            .filter(|s| s.context != context && s.raw.get("AutoExit").is_some())
            .cloned()
            .collect();
        for original in &originals {
            let mut slot = original.clone();
            slot.raw.as_object_mut().unwrap().remove("AutoExit");
            self.store_slot(slot);
        }
        if let Err(error) = self.persist() {
            for original in originals {
                self.store_slot(original);
            }
            return Err(error);
        }
        self.folder_exit = None;
        self.render()
    }
    pub(super) fn folder_exit_input(&mut self, event: &InputEvent) -> Option<Pending> {
        if self.library.folders.is_empty() {
            return None;
        }
        let (controller, index, pressed, owned) = match *event {
            InputEvent::Key { index, pressed } => (
                "Keypad",
                index,
                pressed,
                self.held.contains_key(&HeldControl::Key(index)),
            ),
            InputEvent::DialPress { index, pressed } => (
                "Encoder",
                index,
                pressed,
                self.held.contains_key(&HeldControl::Dial(index)),
            ),
            InputEvent::DialRotate { index, ticks } if ticks != 0 => {
                ("Encoder", index, false, true)
            }
            InputEvent::Touch { x, .. }
                if self.model.dial_count() > 0 && self.model.capabilities().touchscreen =>
            {
                (
                    "Encoder",
                    (u32::from(x) / (self.model.strip_width() / u32::from(self.model.dial_count())))
                        as u8,
                    false,
                    true,
                )
            }
            _ => return None,
        };
        let slot = self
            .slots
            .iter()
            .find(|s| s.controller == controller && s.index == index)?;
        // Native activity cancels an existing countdown before starting another.
        if (pressed || owned) && self.folder_exit.take().is_some() {
            // Dispatch renders after this borrow ends, restoring the Back image.
            self.multi_notify.notify_one();
        }
        if pressed || !owned || excluded(&slot.uuid) {
            return None;
        }
        let back = self.slots.iter().find(|s| s.uuid == library::BACK_FOLDER)?;
        let duration = delay(&slot.raw, &back.raw)?;
        Some(Pending {
            profile: self.library.active_profile.clone(),
            page: self.library.active_page.clone(),
            folders: self.library.folders.clone(),
            back: back.context.clone(),
            due: self.multi_clock.elapsed().saturating_add(duration),
            started: self.multi_clock.elapsed(),
            last_percent: 0,
            next_frame: self.multi_clock.elapsed() + Duration::from_millis(16),
        })
    }
    fn folder_exit_valid(&self, pending: &Pending) -> bool {
        pending.profile == self.library.active_profile
            && pending.page == self.library.active_page
            && pending.folders == self.library.folders
            && self
                .slots
                .iter()
                .any(|s| s.context == pending.back && s.uuid == library::BACK_FOLDER)
    }
    pub(super) fn retain_folder_exit(&mut self) {
        self.folder_back_holds.retain(|context, hold| {
            hold.belongs_to(&self.library)
                && self
                    .slots
                    .iter()
                    .flat_map(Slot::visible_flatten)
                    .any(|s| s.context == *context && s.uuid == library::BACK_FOLDER)
        });
        if self
            .folder_exit
            .as_ref()
            .is_some_and(|p| !self.folder_exit_valid(p))
        {
            self.folder_exit = None;
        }
    }
    pub(super) fn arm_folder_exit(&mut self, pending: Option<Pending>) {
        if let Some(pending) = pending.filter(|p| self.folder_exit_valid(p)) {
            self.folder_exit = Some(pending);
            self.multi_notify.notify_one();
            self.poll_folder_exit();
        }
    }
    pub(super) fn folder_exit_percent(&self) -> Option<(&str, u8)> {
        self.folder_exit
            .as_ref()
            .map(|p| (p.back.as_str(), p.percent(self.multi_clock.elapsed())))
    }
    pub(super) fn next_folder_exit(&self) -> Option<Duration> {
        self.folder_exit.as_ref().map(|p| p.next_frame.min(p.due))
    }
    pub(super) fn poll_folder_exit(&mut self) {
        self.retain_folder_exit();
        if self
            .folder_exit
            .as_ref()
            .is_some_and(|p| p.due <= self.multi_clock.elapsed())
        {
            self.folder_exit = None;
            if let Err(error) = self.folder_back() {
                self.log(format!("Folder auto-exit: {error:#}"));
            }
        } else if let Some(pending) = &mut self.folder_exit {
            let now = self.multi_clock.elapsed();
            if now < pending.next_frame {
                return;
            }
            let percent = pending.percent(now);
            let duration = pending.due.saturating_sub(pending.started);
            pending.next_frame = (pending.started
                + duration.mul_f64(f64::from(percent.saturating_add(1)) / 100.0))
            .max(now + Duration::from_millis(16))
            .min(pending.due);
            if percent != pending.last_percent {
                pending.last_percent = percent;
                if let Err(error) = self.render() {
                    self.log(format!("Folder countdown: {error:#}"));
                }
            }
        }
    }
    pub(super) fn save_folder_exit(&mut self, request: &Value) -> Result<()> {
        self.editable()?;
        let context = request["context"]
            .as_str()
            .context("Action context missing")?;
        let mut slot = self
            .slots
            .iter()
            .find(|s| s.context == context)
            .cloned()
            .context("Select a visible action to configure folder exit")?;
        anyhow::ensure!(
            slot.uuid == library::BACK_FOLDER || !excluded(&slot.uuid),
            "Navigation actions do not trigger folder auto-exit"
        );
        let value = &request["auto_exit"];
        anyhow::ensure!(
            value.is_null()
                || (value.is_object()
                    && value["Enabled"].is_boolean()
                    && value["Delay"].as_u64().is_some_and(|n| n <= 60)),
            "Folder exit requires Enabled and a whole delay from 0 to 60 seconds"
        );
        if !slot.raw.is_object() {
            slot.raw = json!({});
        }
        if value.is_null() {
            slot.raw.as_object_mut().unwrap().remove("AutoExit");
        } else {
            let mut saved = slot.raw["AutoExit"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            saved.insert("Enabled".into(), value["Enabled"].clone());
            saved.insert("Delay".into(), value["Delay"].clone());
            slot.raw["AutoExit"] = Value::Object(saved);
        }
        self.store_slot(slot);
        self.folder_exit = None;
        self.persist()?;
        self.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Host {
        let mut host = super::super::tests::fixture(root);
        host.actions.extend(library::navigation_actions());
        host.library_command(&json!({"command":"folder_create","name":"Nested","index":1}))
            .unwrap();
        let page = host
            .slots
            .iter()
            .find(|s| s.uuid == library::OPEN_FOLDER)
            .unwrap()
            .settings["ProfileUUID"]
            .as_str()
            .unwrap()
            .to_owned();
        host.select_page(&page, true).unwrap();
        host.assign("Keypad", 1, "test.action").unwrap();
        let context = host
            .slots
            .iter()
            .find(|s| s.uuid == library::BACK_FOLDER)
            .unwrap()
            .context
            .clone();
        host.save_folder_exit(&json!({"context":context,"auto_exit":{"Enabled":true,"Delay":1}}))
            .unwrap();
        host
    }
    #[test]
    fn release_starts_press_cancels_and_return_goes_to_exact_parent() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let parent = host.library.folders.last().unwrap().clone();
        let key = |pressed| InputEvent::Key { index: 1, pressed };
        host.input(key(false)).unwrap();
        assert!(
            host.folder_exit.is_none(),
            "Unowned release cannot start a timer"
        );
        host.input(key(true)).unwrap();
        assert!(host.folder_exit.is_none());
        host.input(key(false)).unwrap();
        assert!(host.folder_exit.is_some());
        host.input(key(true)).unwrap();
        assert!(host.folder_exit.is_none());
        host.input(key(false)).unwrap();
        let due = host.folder_exit.as_ref().unwrap().due;
        host.multi_clock = std::time::Instant::now() - due;
        host.poll_folder_exit();
        assert_eq!(host.library.active_page, parent);
        assert!(host.library.folders.is_empty());
        assert!(host.folder_exit.is_none());
    }
    #[test]
    fn page_changes_disconnect_and_explicit_override_retire_deadlines() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let action = host
            .slots
            .iter()
            .find(|s| s.uuid == "test.action")
            .unwrap()
            .context
            .clone();
        host.save_folder_exit(&json!({"context":action,"auto_exit":{"Enabled":false,"Delay":0}}))
            .unwrap();
        host.input(InputEvent::Key {
            index: 1,
            pressed: true,
        })
        .unwrap();
        host.input(InputEvent::Key {
            index: 1,
            pressed: false,
        })
        .unwrap();
        assert!(host.folder_exit.is_none());
        host.save_folder_exit(&json!({"context":action,"auto_exit":null}))
            .unwrap();
        for disconnect in [true, false] {
            host.input(InputEvent::Key {
                index: 1,
                pressed: true,
            })
            .unwrap();
            host.input(InputEvent::Key {
                index: 1,
                pressed: false,
            })
            .unwrap();
            assert!(host.folder_exit.is_some());
            if disconnect {
                host.release_device_holds();
            } else {
                host.folder_back().unwrap();
            }
            assert!(host.folder_exit.is_none());
        }
    }
    #[test]
    fn edits_keep_unknown_fields_and_invalid_changes_are_atomic() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let mut action = host
            .slots
            .iter()
            .find(|s| s.uuid == "test.action")
            .unwrap()
            .clone();
        let id = action.context.clone();
        action.raw["Future"] = json!(42);
        action.raw["AutoExit"] = json!({"Enabled":false,"Delay":7,"FutureOption":8});
        host.store_slot(action);
        host.save_folder_exit(&json!({"context":id,"auto_exit":{"Enabled":true,"Delay":0}}))
            .unwrap();
        let raw = host.visible_slot(&id).unwrap().raw.clone();
        assert_eq!(raw["AutoExit"]["FutureOption"], 8);
        assert_eq!(raw["Future"], 42);
        assert!(
            host.save_folder_exit(&json!({"context":id,"auto_exit":{"Enabled":true,"Delay":61}}))
                .is_err()
        );
        assert_eq!(host.visible_slot(&id).unwrap().raw, raw);
        host.input(InputEvent::Key {
            index: 1,
            pressed: true,
        })
        .unwrap();
        assert_eq!(host.library.folders.len(), 1);
        host.input(InputEvent::Key {
            index: 1,
            pressed: false,
        })
        .unwrap();
        assert!(host.library.folders.is_empty());
    }
    #[test]
    fn deadlines_stay_with_their_device_and_nested_folder() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let device = host.device.clone();
        let parent = host.library.active_page.clone();
        host.library_command(&json!({"command":"folder_create","name":"Deeper","index":2}))
            .unwrap();
        host.input(InputEvent::Key {
            index: 2,
            pressed: true,
        })
        .unwrap();
        host.input(InputEvent::Key {
            index: 2,
            pressed: false,
        })
        .unwrap();
        assert!(
            host.folder_exit.is_none(),
            "Opening a child cannot arm the parent's timer"
        );
        host.assign("Keypad", 1, "test.action").unwrap();
        let action = host
            .slots
            .iter()
            .find(|s| s.uuid == "test.action")
            .unwrap()
            .context
            .clone();
        host.save_folder_exit(&json!({"context":action,"auto_exit":{"Enabled":true,"Delay":1}}))
            .unwrap();
        host.input(InputEvent::Key {
            index: 1,
            pressed: true,
        })
        .unwrap();
        host.input(InputEvent::Key {
            index: 1,
            pressed: false,
        })
        .unwrap();
        let due = host.folder_exit.as_ref().unwrap().due;
        host.add_simulated_device("second", Model::Plus).unwrap();
        host.with_device("second", |host| {
            assert!(host.folder_exit.is_none());
            host.input(InputEvent::Key {
                index: 1,
                pressed: true,
            })?;
            host.poll_folder_exit();
            assert!(host.library.folders.is_empty());
            Ok(())
        })
        .unwrap();
        host.with_device(&device, |host| {
            assert_eq!(host.folder_exit.as_ref().unwrap().due, due);
            host.multi_clock = std::time::Instant::now() - due;
            host.poll_folder_exit();
            assert_eq!(host.library.active_page, parent);
            assert_eq!(host.library.folders.len(), 1);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn reset_preserves_folder_default_other_pages_pins_and_plugin_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let back = host
            .slots
            .iter()
            .find(|s| s.uuid == library::BACK_FOLDER)
            .unwrap()
            .context
            .clone();
        let local = host
            .slots
            .iter()
            .find(|s| s.uuid == "test.action")
            .unwrap()
            .context
            .clone();
        host.save_folder_exit(&json!({"context":local,"auto_exit":{"Enabled":false,"Delay":8}}))
            .unwrap();
        host.assign("Keypad", 2, "test.action").unwrap();
        let pin = host
            .slots
            .iter()
            .find(|s| s.index == 2)
            .unwrap()
            .context
            .clone();
        host.save_folder_exit(&json!({"context":pin,"auto_exit":{"Enabled":true,"Delay":12}}))
            .unwrap();
        host.library_command(&json!({"command":"slot_pin","context":pin,"pinned":true}))
            .unwrap();
        let page = host.library.active_page.clone();
        let parent = host.library.folders[0].clone();
        let outside = host
            .library
            .current_mut()
            .unwrap()
            .pages
            .iter_mut()
            .find(|p| p.id == parent)
            .unwrap()
            .slots
            .iter_mut()
            .find(|s| s.uuid == "test.action")
            .unwrap();
        outside.raw["AutoExit"] = json!({"Enabled":true,"Delay":30});
        let outside_id = outside.context.clone();
        let before = host.visible_slot(&local).unwrap().settings.clone();
        let request = json!({"context":back,"page":page,"confirm":true});
        assert!(
            host.reset_folder_exits(&json!({"context":back,"page":"stale","confirm":true}))
                .is_err()
        );
        assert!(
            host.reset_folder_exits(&json!({"context":back,"page":page,"confirm":false}))
                .is_err()
        );
        host.reset_folder_exits(&request).unwrap();
        assert!(
            host.visible_slot(&local)
                .unwrap()
                .raw
                .get("AutoExit")
                .is_none()
        );
        assert_eq!(host.visible_slot(&local).unwrap().settings, before);
        assert_eq!(
            host.visible_slot(&back).unwrap().raw["AutoExit"]["Delay"],
            1
        );
        assert_eq!(
            host.visible_slot(&pin).unwrap().raw["AutoExit"]["Delay"],
            12
        );
        assert_eq!(
            host.library.slot(&outside_id).unwrap().raw["AutoExit"]["Delay"],
            30
        );
        host.save_folder_exit(&json!({"context":local,"auto_exit":{"Enabled":false,"Delay":8}}))
            .unwrap();
        let good_path = host.state_path.clone();
        host.state_path = root.path().join("missing").join("state.json");
        assert!(host.reset_folder_exits(&request).is_err());
        assert_eq!(
            host.visible_slot(&local).unwrap().raw["AutoExit"]["Delay"],
            8
        );
        host.state_path = good_path;
        host.library.current_mut().unwrap().readonly = true;
        assert!(host.reset_folder_exits(&request).is_err());
    }
    #[test]
    fn countdown_updates_hardware_and_editor_then_restores_on_cancel_and_save() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        let frame = |receiver: &std::sync::mpsc::Receiver<Draw>| {
            receiver
                .try_iter()
                .filter_map(|draw| {
                    if let Draw::Key(0, image) = draw {
                        Some(image.to_rgba8())
                    } else {
                        None
                    }
                })
                .last()
                .unwrap()
        };
        let back_id = host
            .slots
            .iter()
            .find(|s| s.uuid == library::BACK_FOLDER)
            .unwrap()
            .context
            .clone();
        host.render().unwrap();
        let original = frame(&receiver);
        let preview = host.visible_slot(&back_id).unwrap().preview.clone();
        let raw = host.visible_slot(&back_id).unwrap().raw.clone();
        host.input(InputEvent::Key {
            index: 1,
            pressed: true,
        })
        .unwrap();
        host.input(InputEvent::Key {
            index: 1,
            pressed: false,
        })
        .unwrap();
        let start = frame(&receiver);
        assert_ne!(start, original);
        let halfway = host.folder_exit.as_ref().unwrap().started + Duration::from_millis(500);
        host.multi_clock = std::time::Instant::now() - halfway;
        host.poll_folder_exit();
        let middle = frame(&receiver);
        assert_ne!(start, middle);
        assert_ne!(host.visible_slot(&back_id).unwrap().preview, preview);
        assert_eq!(host.visible_slot(&back_id).unwrap().raw, raw);
        assert!(host.next_folder_exit().unwrap() > halfway);
        host.input(InputEvent::Key {
            index: 1,
            pressed: true,
        })
        .unwrap();
        assert_eq!(frame(&receiver), original);
        assert_eq!(host.visible_slot(&back_id).unwrap().preview, preview);
        assert!(host.next_folder_exit().is_none());
        host.input(InputEvent::Key {
            index: 1,
            pressed: false,
        })
        .unwrap();
        assert!(host.folder_exit.is_some());
        host.save_folder_exit(&json!({"context":back_id,"auto_exit":{"Enabled":false,"Delay":1}}))
            .unwrap();
        assert_eq!(frame(&receiver), original);
        assert!(host.next_folder_exit().is_none());
    }
    fn enter_child(host: &mut Host) {
        host.library_command(&json!({"command":"folder_create","name":"Inner","index":2}))
            .unwrap();
        host.input(InputEvent::Key {
            index: 2,
            pressed: true,
        })
        .unwrap();
        host.input(InputEvent::Key {
            index: 2,
            pressed: false,
        })
        .unwrap();
    }
    #[test]
    fn back_threshold_is_strict_and_repeated_down_keeps_the_original_start() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        let id = host
            .slots
            .iter()
            .find(|s| s.uuid == library::BACK_FOLDER)
            .unwrap()
            .context
            .clone();
        let started = host.folder_back_holds[&id].started;
        assert!(!host.folder_back_holds[&id].held_long(started + Duration::from_millis(500)));
        assert!(host.folder_back_holds[&id].held_long(started + Duration::from_nanos(500_000_001)));
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(host.folder_back_holds[&id].started, started);
    }
    #[test]
    fn short_back_returns_one_level_and_long_back_returns_the_actual_root_page() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let outer = host.library.active_page.clone();
        host.library_command(&json!({"command":"page_create","name":"Second top page"}))
            .unwrap();
        let actual_root = host.library.active_page.clone();
        assert_ne!(actual_root, host.library.current().unwrap().page_order[0]);
        host.select_page(&outer, true).unwrap();
        enter_child(&mut host);
        let inner = host.library.active_page.clone();
        enter_child(&mut host);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(host.library.active_page, inner);
        assert_eq!(host.library.folders.len(), 2);
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(
            host.library.active_page, inner,
            "Late duplicate release must do nothing"
        );
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        let started = host.folder_back_holds.values().next().unwrap().started;
        host.multi_clock = std::time::Instant::now() - (started + Duration::from_millis(600));
        assert_eq!(
            host.library.active_page, inner,
            "Holding alone must not navigate"
        );
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(host.library.active_page, actual_root);
        assert!(host.library.folders.is_empty());
        assert!(host.folder_back_holds.is_empty());
    }
    #[test]
    fn back_holds_do_not_cross_navigation_paths_devices_or_disconnects() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let device = host.device.clone();
        let folder = host.library.active_page.clone();
        let back = host
            .slots
            .iter()
            .find(|s| s.uuid == library::BACK_FOLDER)
            .unwrap()
            .context
            .clone();
        host.library_command(&json!({"command":"slot_pin","context":back,"pinned":true}))
            .unwrap();
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.select_page(&folder, true).unwrap();
        assert!(
            host.folder_back_holds.is_empty(),
            "Even a retained pinned key loses a hold when its folder path changes"
        );
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(host.library.folders.len(), 2);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.add_simulated_device("other", Model::Plus).unwrap();
        host.with_device("other", |h| {
            assert!(h.folder_back_holds.is_empty());
            h.release_device_holds();
            Ok(())
        })
        .unwrap();
        host.with_device(&device, |h| {
            assert_eq!(h.folder_back_holds.len(), 1);
            h.release_device_holds();
            assert!(h.folder_back_holds.is_empty());
            h.input(InputEvent::Key {
                index: 0,
                pressed: false,
            })?;
            assert_eq!(h.library.folders.len(), 2);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn action_trigger_back_uses_the_same_hold_and_cancel_contract() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let target = host.library.folders[0].clone();
        enter_child(&mut host);
        host.actions.push(trigger::action());
        host.assign("Encoder", 0, crate::action_trigger::UUID)
            .unwrap();
        let id = host
            .slots
            .iter()
            .find(|s| s.uuid == crate::action_trigger::UUID)
            .unwrap()
            .context
            .clone();
        host.save_trigger(&id,json!({"Actions":[{}, {"UUID":library::BACK_FOLDER,"ActionID":"back","Settings":{},"States":[{}]}, {}]})).unwrap();
        host.input(InputEvent::DialPress {
            index: 0,
            pressed: true,
        })
        .unwrap();
        host.poll_folder_exit();
        assert_eq!(
            host.folder_back_holds.len(),
            1,
            "Visible nested Back must keep its ownership"
        );
        host.release_device_holds();
        assert!(host.folder_back_holds.is_empty());
        host.input(InputEvent::DialPress {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(host.library.folders.len(), 2);
        host.input(InputEvent::DialPress {
            index: 0,
            pressed: true,
        })
        .unwrap();
        let started = host.folder_back_holds.values().next().unwrap().started;
        host.multi_clock = std::time::Instant::now() - (started + Duration::from_millis(600));
        host.input(InputEvent::DialPress {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert_eq!(host.library.active_page, target);
        assert!(host.folder_back_holds.is_empty());
        assert!(host.library.folders.is_empty());
    }
    #[test]
    fn original_record_inheritance_and_disabled_overrides() {
        let folder = json!({"AutoExit":{"Delay":10,"Enabled":true}});
        assert_eq!(delay(&json!({}), &folder), Some(Duration::from_secs(10)));
        assert_eq!(
            delay(&json!({"AutoExit":{"Enabled":false,"Delay":0}}), &folder),
            None
        );
        assert_eq!(
            delay(&json!({"AutoExit":{"Enabled":true,"Delay":0}}), &folder),
            Some(Duration::ZERO)
        );
        for bad in [json!(-1), json!(61), json!(1.5), json!("1")] {
            assert_eq!(
                delay(&json!({"AutoExit":{"Enabled":true,"Delay":bad}}), &folder),
                None
            );
        }
        assert_eq!(delay(&json!({}), &json!({})), None);
    }
}
