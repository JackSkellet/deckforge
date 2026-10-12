//! Connection state is separate from the editor's visible action instances.
use super::*;
use crate::device_session::Event;
use anyhow::ensure;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Config {
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
                id: serial.clone(),
                serial: Some(serial),
                firmware,
            }
        } else {
            remembered.unwrap_or_else(|| Self {
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
pub(super) fn info() -> Value {
    json!({"name":"Stream Deck +","type":7,"size":{"columns":4,"rows":2}})
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
    fn device_broadcast(&self, event: Value) {
        for plugin in &self.plugins {
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(event.clone());
            }
        }
    }
    pub(super) fn release_device_holds(&mut self) {
        // Completed clicks and their asynchronous actions remain active offline.
        // Only physical ownership is lost when the USB transport disappears.
        self.disconnect_trigger_buttons();
        self.cancel_departing_inputs(&HashSet::new());
        self.cancel_departing_stacks(&HashSet::new());
        self.dial_pressed = [false; 4];
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
                    self.input(input)?;
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
                    self.log(format!(
                        "USB disconnected: {reason}. Reconnecting automatically."
                    ));
                }
                self.connection_error = Some(reason);
            }
            Event::Connected { serial, firmware } => {
                let config = Config {
                    id: serial.clone(),
                    serial: Some(serial.clone()),
                    firmware: firmware.clone(),
                };
                config.validate()?;
                let changed = self.device != serial;
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
                    self.device = serial;
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
                    self.device_broadcast(json!({"event":"deviceDidConnect","device":self.device,"deviceInfo":info()}));
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
}
