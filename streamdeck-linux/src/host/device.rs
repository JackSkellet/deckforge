//! Connection state is separate from the editor's visible action instances.
use super::*;
use crate::device_session::Event;

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
impl Host {
    fn device_broadcast(&self, event: Value) {
        for plugin in &self.plugins {
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(event.clone());
            }
        }
    }
    fn release_device_holds(&mut self) {
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
