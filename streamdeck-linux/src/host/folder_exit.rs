//! Original action-level AutoExit settings and device-owned folder deadlines.
use super::*;

#[derive(Clone)]
pub(super) struct Pending {
    profile: String,
    page: String,
    folders: Vec<String>,
    back: String,
    pub due: Duration,
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
        if pressed || owned {
            self.folder_exit = None;
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
        self.persist()
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
