//! Plugin system events are application-wide; physical reconnect and lock/idle
//! changes remain separate device/session events.
use super::*;

impl Host {
    pub(super) fn system_wake(&mut self) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        // Rebuild SDK action stores first, including each visible container child.
        // Selection, contexts, settings and held controls are not recreated.
        for device in self.device_ids() {
            self.with_device(&device, |host| {
                for slot in host.slots.iter().flat_map(Slot::visible_flatten) {
                    host.emit(slot, "willAppear", json!({}));
                }
                Ok(())
            })?;
        }
        for plugin in &self.plugins {
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(json!({"event":"systemDidWakeUp"}));
            }
        }
        self.log("System resume notification sent to plugins");
        Ok(())
    }
    pub(super) fn poll_system_resume(&mut self) {
        let events = self
            .resume_monitor
            .as_ref()
            .map(|worker| worker.drain())
            .unwrap_or_default();
        for event in events {
            match event {
                crate::system_resume::Event::Wake => {
                    if let Err(error) = self.system_wake() {
                        self.log(format!("Plugin resume replay failed: {error:#}"));
                    }
                }
                crate::system_resume::Event::Available => {
                    self.resume_available = true;
                    self.resume_error = None;
                }
                crate::system_resume::Event::Unavailable(error) => {
                    self.resume_available = false;
                    if self.resume_error.as_ref() != Some(&error) {
                        self.log(format!("System resume monitor: {error}"));
                    }
                    self.resume_error = Some(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_replays_each_device_then_one_global_event_without_pi_or_transport_effects() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let a = host.device.clone();
        host.add_simulated_device("B", Model::Mini).unwrap();
        host.with_device("B", |h| h.assign("Keypad", 5, "test.action"))
            .unwrap();
        host.select_device("B").unwrap();
        let before = serde_json::to_value(host.device_records()).unwrap();
        let (send, mut received) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(send);
        let (pi, mut pi_events) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "fixture-pi".into(),
            Inspector {
                setup: None,
                context: host.slots[0].context.clone(),
                plugin: 0,
                child: None,
                sender: Some(pi),
            },
        );
        let (draw, frames) = std::sync::mpsc::channel();
        host.hardware = Some(draw);
        host.system_wake().unwrap();
        let events: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event"] == "systemDidWakeUp")
                .count(),
            1
        );
        assert_eq!(events.last().unwrap(), &json!({"event":"systemDidWakeUp"}));
        assert_eq!(
            events.iter().filter(|e| e["event"] == "willAppear").count(),
            2
        );
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "willAppear" && e["device"] == a)
        );
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "willAppear" && e["device"] == "B")
        );
        assert_eq!(serde_json::to_value(host.device_records()).unwrap(), before);
        assert_eq!(host.device, "B");
        assert!(pi_events.try_recv().is_err());
        assert!(frames.try_recv().is_err());
        host.running = false;
        host.system_wake().unwrap();
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn lock_idle_and_usb_events_do_not_claim_a_computer_resume() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let (send, mut received) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(send);
        for locked in [true, false] {
            host.session_update(crate::session_activity::State {
                idle_ms: Some(0),
                locked: Some(locked),
                source: "fixture".into(),
                error: None,
            })
            .unwrap();
        }
        host.device_event(DeviceEvent::Disconnected {
            reason: "fixture".into(),
        })
        .unwrap();
        host.device_event(DeviceEvent::Connected {
            serial: host.device.clone(),
            model: host.model,
            firmware: "fixture".into(),
        })
        .unwrap();
        assert!(
            std::iter::from_fn(|| received.try_recv().ok())
                .all(|e| e["event"] != "systemDidWakeUp")
        );
        host.simulate = false;
        assert!(apply_control(&mut host, json!({"command":"system_wake"})).is_err());
    }
}
