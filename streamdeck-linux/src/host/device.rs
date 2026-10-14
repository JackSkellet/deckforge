//! Connection state is separate from the editor's visible action instances.
use super::*;
use crate::device_session::Event;
use anyhow::ensure;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Config {
    #[serde(default)]
    pub model: Model,
    pub id: String,
    pub serial: Option<String>,
    pub firmware: String,
}
impl Config {
    pub(super) fn remembered(saved: &Saved) -> Option<Self> {
        saved.device_config.clone().or_else(|| {
            let library = saved.library.as_ref()?;
            let profile = library
                .profiles
                .iter()
                .find(|p| p.id == library.active_profile)?;
            let id = &profile.device;
            (!id.is_empty() && id != "simulated-streamdeck-plus").then(|| Self {
                model: profile.model,
                id: id.clone(),
                serial: (!id.starts_with("offline-streamdeck-plus-")).then(|| id.clone()),
                firmware: "unknown".into(),
            })
        })
    }
    pub(super) fn select(remembered: Option<Self>, serial: Option<String>) -> Self {
        if let Some(serial) = serial {
            let firmware = remembered
                .as_ref()
                .filter(|c| c.id == serial)
                .map(|c| c.firmware.clone())
                .unwrap_or_else(|| "unknown".into());
            Self {
                model: remembered.as_ref().map(|c| c.model).unwrap_or_default(),
                id: serial.clone(),
                serial: Some(serial),
                firmware,
            }
        } else {
            remembered.unwrap_or_else(|| Self {
                model: Model::Plus,
                id: format!("offline-streamdeck-plus-{}", uuid::Uuid::new_v4()),
                serial: None,
                firmware: "unknown".into(),
            })
        }
    }
    pub(super) fn validate(&self) -> Result<()> {
        for value in [Some(self.id.as_str()), self.serial.as_deref()]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 4096 || value.contains('\0') {
                bail!("Invalid configured device identity");
            }
        }
        Ok(())
    }
}
pub(super) fn selected_model(
    remembered: Option<&Config>,
    serial: Option<&str>,
    requested: Option<Model>,
) -> Option<Model> {
    requested.or_else(|| {
        remembered
            .filter(|saved| {
                serial.is_none_or(|serial| saved.serial.as_deref().unwrap_or(&saved.id) == serial)
            })
            .map(|saved| saved.model)
    })
}
pub(super) fn info(model: Model) -> Value {
    json!({"name":model.name(),"type":model.sdk_type(),"size":{"columns":model.columns(),"rows":model.rows()}})
}
pub(super) fn valid_slot(model: Model, controller: &str, index: u8) -> bool {
    match controller {
        "Keypad" => index < model.key_count(),
        "Encoder" => index < model.dial_count(),
        "Neo" => model == Model::Neo && index == 1,
        _ => false,
    }
}
pub(super) fn validate_controls(controls: device_session::Controls) -> Result<()> {
    ensure!(
        controls.brightness.is_none_or(|value| value <= 100),
        "Brightness must be between 0 and 100"
    );
    ensure!(
        controls
            .sleep_seconds
            .is_none_or(|value| value <= i32::MAX as u32),
        "Device idle timer is out of range"
    );
    Ok(())
}
impl Host {
    pub(super) fn configured_controls(&self) -> device_session::Controls {
        self.device_controls
            .get(&self.device)
            .copied()
            .unwrap_or_default()
    }
    pub(super) fn send_device_controls(&mut self) {
        if self
            .device_config
            .as_ref()
            .is_none_or(|config| config.serial.is_none())
        {
            return;
        }
        if let Some(hardware) = &self.hardware {
            // Timer ownership and restoration are ordered with HID writes in
            // the worker; old asynchronous reports cannot release that ownership.
            let result = hardware
                .send(Draw::SleepPolicy(
                    self.screensavers.contains_key(&self.device),
                ))
                .and_then(|()| hardware.send(Draw::Controls(self.configured_controls())));
            if let Err(error) = result {
                self.controls_error = Some(format!("Device settings delivery failed: {error}"));
            }
        }
    }
    pub(super) fn set_device_controls(&mut self, controls: device_session::Controls) -> Result<()> {
        validate_controls(controls)?;
        ensure!(
            controls.brightness.is_none() || self.model.capabilities().brightness,
            "This device has no brightness control"
        );
        ensure!(
            controls.sleep_seconds.is_none() || self.model.capabilities().sleep,
            "This device has no sleep timer"
        );
        let previous = self.device_controls.insert(self.device.clone(), controls);
        if let Err(error) = self.persist() {
            match previous {
                Some(value) => {
                    self.device_controls.insert(self.device.clone(), value);
                }
                None => {
                    self.device_controls.remove(&self.device);
                }
            }
            return Err(error);
        }
        self.controls_error = None;
        self.send_device_controls();
        self.render()?;
        if let Some(redraw) = &self.redraw {
            redraw();
        }
        Ok(())
    }
    pub(super) fn device_broadcast(&self, event: Value) {
        for plugin in &self.plugins {
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(event.clone());
            }
        }
    }
    pub(super) fn release_device_holds(&mut self) {
        self.encoder_routing.disconnected();
        // Completed clicks and their asynchronous actions remain active offline.
        // Only physical ownership is lost when the USB transport disappears.
        self.disconnect_trigger_buttons();
        self.wheels.stop_turns();
        self.folder_exit = None;
        self.cancel_departing_inputs(&HashSet::new());
        self.cancel_departing_stacks(&HashSet::new());
        self.dial_pressed.fill(false);
        self.sensor_pressed.fill(false);
    }
    pub(super) fn device_event(&mut self, event: Event) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        match event {
            Event::Activity => {
                if !self.saver.active {
                    self.saver.activity = self.multi_clock.elapsed();
                    self.multi_notify.notify_one();
                }
                return Ok(());
            }
            Event::Wake { generation } => {
                if generation == self.saver.generation {
                    self.wake_device()?;
                }
            }
            Event::Controls { controls, error } => {
                self.controls_observed = controls;
                if let Some(error) = &error {
                    self.system_status = format!("Device settings failed: {error}");
                    self.log(self.system_status.clone());
                } else if self.system_status.starts_with("Device settings failed:") {
                    self.system_status.clear();
                }
                self.controls_error = error;
            }
            Event::Input(input) => {
                if self.connected {
                    self.physical_input(input)?;
                }
            }
            Event::Retry { reason } => {
                if self.connection_error.as_ref() != Some(&reason) {
                    self.log(format!("Waiting for Stream Deck +: {reason}"));
                    self.connection_error = Some(reason);
                }
            }
            Event::Disconnected { reason } => {
                if self.connected {
                    self.connected = false;
                    self.controls_observed = device_session::Controls::default();
                    self.device_broadcast(
                        json!({"event":"deviceDidDisconnect","device":self.device}),
                    );
                    self.release_device_holds();
                    self.encoder_marker_layer()?;
                    self.log(format!(
                        "USB disconnected: {reason}. Reconnecting automatically."
                    ));
                }
                self.connection_error = Some(reason);
            }
            Event::Connected {
                serial,
                firmware,
                model,
            } => {
                if model != self.model || serial != self.device {
                    self.history.clear("Device identity or layout changed");
                }

                ensure!(
                    self.device_scope(&serial)
                        .is_none_or(|scope| scope == self.scope),
                    "Device identity is already owned by another runtime"
                );
                let config = Config {
                    model,
                    id: serial.clone(),
                    serial: Some(serial.clone()),
                    firmware: firmware.clone(),
                };
                config.validate()?;
                let changed = self.device != serial || self.model != model;
                let announce = changed || !self.connected;
                if changed {
                    super::screensaver::adopt_preferences(
                        &mut self.screensavers,
                        Some(&self.device),
                        &serial,
                    );
                    let generation = self.saver.generation.saturating_add(1);
                    let locked = self.saver.locked;
                    self.saver = super::screensaver::Runtime {
                        generation,
                        locked,
                        activity: self.multi_clock.elapsed(),
                        ..Default::default()
                    };
                    if let Some(hardware) = &self.hardware {
                        let _ = hardware.send(Draw::Screensaver {
                            generation,
                            image: None,
                            wake_on_input: true,
                        });
                    }
                    if self.device.starts_with("offline-streamdeck-plus-")
                        && !self.device_controls.contains_key(&serial)
                        && let Some(controls) = self.device_controls.remove(&self.device)
                    {
                        self.device_controls.insert(serial.clone(), controls);
                    }
                    self.controls_observed = device_session::Controls::default();
                    if self.connected {
                        self.device_broadcast(
                            json!({"event":"deviceDidDisconnect","device":self.device}),
                        );
                    }
                    self.release_device_holds();
                    self.close_inspectors(None);
                    self.cancel_departing_triggers(&HashSet::new());
                    self.cancel_all_multi();
                    for slot in self.slots.iter().flat_map(Slot::visible_flatten) {
                        self.emit(slot, "willDisappear", json!({}));
                    }
                    // The first real device adopts the offline editor's existing configuration.
                    for profile in &mut self.library.profiles {
                        if profile.device == self.device {
                            profile.device = serial.clone();
                        }
                    }
                    self.sync_visible();
                    self.device = serial;
                    self.model = model;
                    self.dial_pressed = vec![false; model.dial_count() as usize];
                    self.sensor_pressed = vec![false; model.sensor_count() as usize];
                    if !self.library.current()?.model.profile_compatible(model) {
                        self.library.activate_model(model, &self.device)?;
                    }
                    self.slots = self.library.visible()?;
                    self.badges.clear();
                }
                self.connected = true;
                if announce && !self.saver.active {
                    self.saver.activity = self.multi_clock.elapsed();
                }
                self.connection_error = None;
                self.firmware = firmware;
                if !self.simulate {
                    self.device_config = Some(config);
                }
                if announce {
                    self.device_broadcast(json!({"event":"deviceDidConnect","device":self.device,"deviceInfo":self.device_info()}));
                }
                if changed {
                    for slot in self.slots.iter().flat_map(Slot::visible_flatten) {
                        self.emit(slot, "willAppear", json!({}));
                    }
                }
                if changed {
                    self.send_device_controls();
                    if self.saver.locked && !self.saver_preferences().disable_on_lock {
                        self.sleep_device("lock")?;
                    }
                }
                self.render()?;
                self.persist()?;
                if announce {
                    self.log(format!(
                        "USB connected: {} | firmware {}",
                        self.device, self.firmware
                    ));
                }
            }
        }
        if let Some(redraw) = &self.redraw {
            redraw();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn studio_encoders_keep_sdk_metadata_without_loading_or_drawing_lcd_layouts() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.model = Model::Studio;
        host.slots[0].model = Model::Studio;
        host.library = Library::for_model(host.slots.clone(), &host.device, host.model);
        host.dial_pressed = vec![false; 2];
        host.actions[0].controllers.push("Encoder".into());
        host.actions[0].raw["Encoder"] = json!({"layout":"absent-lcd-layout.json"});
        host.assign("Encoder", 1, "test.action").unwrap();
        let context = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .context
            .clone();
        host.plugin_message(0, json!({"event":"setFeedbackLayout","context":context,"payload":{"layout":"still-no-lcd.json"}})).unwrap();
        host.plugin_message(0, json!({"event":"setFeedback","context":context,"payload":{"value":42,"title":"Software metadata"}})).unwrap();
        let dial = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap();
        assert_eq!(dial.feedback["value"], 42);
        assert_eq!(dial.layout, "still-no-lcd.json");
        assert!(dial.layout_cache.is_none() && dial.preview.is_empty());

        let (draw, frames) = std::sync::mpsc::channel();
        host.hardware = Some(draw);
        host.render().unwrap();
        let frames: Vec<_> = frames.try_iter().collect();
        assert_eq!(frames.len(), 32);
        assert!(frames.iter().all(|frame| matches!(frame, Draw::Key(_, image) if image.width() == 144 && image.height() == 112)));
        let (send, mut events) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(send);
        for command in [
            json!({"command":"input","event":"dialDown","index":1}),
            json!({"command":"input","event":"dialRotate","index":1,"ticks":-3}),
            json!({"command":"input","event":"dialUp","index":1}),
        ] {
            super::super::apply_control(&mut host, command).unwrap();
        }
        let events: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert_eq!(
            events
                .iter()
                .map(|e| e["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["dialDown", "dialRotate", "dialUp"]
        );
        assert!(
            events
                .iter()
                .all(|e| e["payload"]["coordinates"] == json!({"column":1,"row":0}))
        );
        assert_eq!(events[1]["payload"]["ticks"], -3);
        assert_eq!(events[1]["payload"]["pressed"], true);
        assert!(
            super::super::apply_control(
                &mut host,
                json!({"command":"input","event":"touchTap","index":1})
            )
            .is_err()
        );
    }
    #[test]
    fn unidentified_offline_overrides_wait_for_serial_specific_selection() {
        let root = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(root.path());
        host.simulate = false;
        let offline = Config::select(None, None);
        host.device = offline.id.clone();
        host.device_config = Some(offline);
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        host.set_device_controls(device_session::Controls {
            brightness: Some(20),
            sleep_seconds: None,
        })
        .unwrap();
        assert!(
            !receiver
                .try_iter()
                .any(|event| matches!(event, Draw::Controls(_)))
        );
        host.device_controls
            .insert("real-a".into(), device_session::Controls::default());
        host.device_event(Event::Connected {
            model: Model::Plus,
            serial: "real-a".into(),
            firmware: "test".into(),
        })
        .unwrap();
        let commands: Vec<_> = receiver
            .try_iter()
            .filter_map(|event| match event {
                Draw::Controls(controls) => Some(controls),
                _ => None,
            })
            .collect();
        assert_eq!(commands, vec![device_session::Controls::default()]);
        assert_eq!(host.configured_controls().brightness, None);
        host.device_event(Event::Connected {
            model: Model::Plus,
            serial: "real-a".into(),
            firmware: "test".into(),
        })
        .unwrap();
        assert!(
            !receiver
                .try_iter()
                .any(|event| matches!(event, Draw::Controls(_)))
        );
    }

    #[test]
    fn offline_identity_is_reused_and_explicit_serial_does_not_inherit_another_firmware() {
        let first = Config::select(None, None);
        assert!(first.serial.is_none());
        first.validate().unwrap();
        let restored: Config =
            serde_json::from_value(serde_json::to_value(&first).unwrap()).unwrap();
        assert_eq!(Config::select(Some(restored), None).id, first.id);
        let original = Config {
            model: Model::Plus,
            id: "device-a".into(),
            serial: Some("device-a".into()),
            firmware: "2.0.3.4".into(),
        };
        assert_eq!(
            Config::select(Some(original.clone()), Some("device-a".into())).firmware,
            "2.0.3.4"
        );
        let changed = Config::select(Some(original), Some("device-b".into()));
        assert_eq!(changed.firmware, "unknown");
        assert_eq!(changed.serial.as_deref(), Some("device-b"));
    }
    #[test]
    fn disconnected_device_keeps_visible_actions_and_sends_balanced_release_once() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        let context = host.slots[0].context.clone();
        host.device_event(Event::Disconnected {
            reason: "test loss".into(),
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "deviceDidDisconnect");
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        assert!(rx.try_recv().is_err());
        assert_eq!(host.slots[0].context, context);
        host.device_event(Event::Disconnected {
            reason: "same loss".into(),
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
        host.device_event(Event::Connected {
            model: Model::Plus,
            serial: host.device.clone(),
            firmware: "test".into(),
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "deviceDidConnect");
        assert!(rx.try_recv().is_err());
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
        assert!(host.connected);
    }
    #[test]
    fn identity_change_closes_inspectors_with_stale_device_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        let (pi, _) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "old-inspector".into(),
            Inspector {
                context: host.slots[0].context.clone(),
                plugin: 0,
                child: None,
                sender: Some(pi),
            },
        );
        host.device_event(Event::Connected {
            model: Model::Plus,
            serial: "new-device".into(),
            firmware: "test".into(),
        })
        .unwrap();
        assert!(host.inspectors.is_empty());
        let rows: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let closed = rows
            .iter()
            .position(|row| row["event"] == "propertyInspectorDidDisappear")
            .unwrap();
        let changed = rows
            .iter()
            .position(|row| row["event"] == "deviceDidConnect")
            .unwrap();
        assert!(closed < changed);
        assert_eq!(rows[closed]["device"], "test-device");
    }
    #[test]
    fn new_device_is_known_before_action_appearance_and_config_is_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.simulate = false;
        host.connected = false;
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.device_event(Event::Connected {
            model: Model::Plus,
            serial: "new-device".into(),
            firmware: "2.0.3.4".into(),
        })
        .unwrap();
        let rows: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            rows.iter()
                .map(|row| row["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["willDisappear", "deviceDidConnect", "willAppear"]
        );
        assert_eq!(rows[0]["device"], "test-device");
        assert_eq!(rows[2]["device"], "new-device");
        assert_eq!(host.library.profiles[0].device, "new-device");
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        let config = Config::remembered(&saved).unwrap();
        assert_eq!(config.serial.as_deref(), Some("new-device"));
        assert_eq!(config.firmware, "2.0.3.4");
    }
    #[test]
    fn explicit_new_serial_does_not_inherit_other_devices_model_filter() {
        let saved = Config {
            model: Model::Plus,
            id: "plus-a".into(),
            serial: Some("plus-a".into()),
            firmware: "test".into(),
        };
        assert_eq!(selected_model(Some(&saved), None, None), Some(Model::Plus));
        assert_eq!(
            selected_model(Some(&saved), Some("plus-a"), None),
            Some(Model::Plus)
        );
        assert_eq!(selected_model(Some(&saved), Some("mini-b"), None), None);
        assert_eq!(
            selected_model(Some(&saved), Some("mini-b"), Some(Model::Mini)),
            Some(Model::Mini)
        );
    }
    #[test]
    fn empty_neo_page_clears_infobar_without_fake_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.model = Model::Neo;
        host.slots.clear();
        host.library = Library::for_model(vec![], "neo", Model::Neo);
        host.sensor_pressed = vec![false; 2];
        let (tx, rx) = std::sync::mpsc::channel();
        host.hardware = Some(tx);
        host.render().unwrap();
        let frames: Vec<_> = rx.try_iter().collect();
        let frame = frames
            .into_iter()
            .find_map(|draw| match draw {
                Draw::Infobar(image) => Some(image),
                _ => None,
            })
            .unwrap();
        assert_eq!((frame.width(), frame.height()), (232, 50));
        assert!(frame.to_rgb8().pixels().all(|pixel| pixel.0 == [0, 0, 0]));
        assert!(
            super::super::apply_control(
                &mut host,
                json!({"command":"input","event":"touchTap","index":0})
            )
            .is_err()
        );
        assert!(
            super::super::apply_control(
                &mut host,
                json!({"command":"input","event":"swipe","index":0,"start":[1,1],"end":[20,1]})
            )
            .is_err()
        );
    }
    #[test]
    fn neo_sensor_lights_follow_page_boundaries_and_press_edges() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.model = Model::Neo;
        host.slots.clear();
        host.library = Library::for_model(vec![], "neo", Model::Neo);
        host.sensor_pressed = vec![false; 2];
        let (tx, rx) = std::sync::mpsc::channel();
        host.hardware = Some(tx);
        let lights = |rx: &std::sync::mpsc::Receiver<Draw>| {
            let mut result = [None, None];
            for draw in rx.try_iter() {
                if let Draw::Sensor(index, image) = draw {
                    assert_eq!((image.width(), image.height()), (96, 16));
                    result[index as usize] =
                        Some(image.to_rgb8().pixels().any(|p| p.0 != [0, 0, 0]));
                }
            }
            result
        };
        host.render().unwrap();
        assert_eq!(lights(&rx), [Some(false), Some(false)]);
        let first = host.library.active_page.clone();
        super::super::apply_control(&mut host, json!({"command":"page_create","name":"Second"}))
            .unwrap();
        let second = host.library.active_page.clone();
        assert_eq!(lights(&rx), [Some(true), Some(false)]);
        host.input(InputEvent::Sensor {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(host.library.active_page, first);
        assert_eq!(lights(&rx), [Some(false), Some(true)]);
        host.input(InputEvent::Sensor {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(lights(&rx), [None, None]);
        host.input(InputEvent::Sensor {
            index: 0,
            pressed: false,
        })
        .unwrap();
        host.input(InputEvent::Sensor {
            index: 1,
            pressed: true,
        })
        .unwrap();
        assert_eq!(host.library.active_page, second);
        assert_eq!(lights(&rx), [Some(true), Some(false)]);
        host.input(InputEvent::Sensor {
            index: 1,
            pressed: false,
        })
        .unwrap();
        host.input(InputEvent::Sensor {
            index: 1,
            pressed: true,
        })
        .unwrap();
        assert_eq!(lights(&rx), [None, None]); // Disabled sensor does not reload the page.
        let mut folder = host.library.current().unwrap().pages[0].clone();
        folder.id = "folder-fixture".into();
        folder.slots.clear();
        host.library.current_mut().unwrap().pages.push(folder);
        host.library.folders = vec![first.clone()];
        host.library.active_page = "folder-fixture".into();
        let navigation = host.page_navigation();
        assert!(navigation.previous && navigation.next);
        host.input(InputEvent::Sensor {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(host.library.active_page, first);
        assert!(host.library.folders.is_empty());
        assert_eq!(lights(&rx), [Some(false), Some(true)]);
    }

    #[test]
    fn touch_input_requires_matching_dial_segment_and_bounds() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        for (x, y) in [(800, 20), (2, 100), (220, 30)] {
            assert!(
                super::super::apply_control(
                    &mut host,
                    json!({"command":"input","event":"touchTap","index":0,"x":x,"y":y})
                )
                .is_err()
            );
        }
    }
}
