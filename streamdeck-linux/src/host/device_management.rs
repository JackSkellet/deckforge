//! Destructive device edits commit the saved graph before releasing runtime state.
//! USB worker joins remain in the registry thread, outside the Host mutex.
use super::*;
use anyhow::ensure;
use device_pool::{DeviceRuntime, Navigation, Record};
use std::io::Write;

impl Host {
    pub(super) fn forget_device(&mut self, device: &str) -> Result<()> {
        let scope = self.device_scope(device).context("Unknown device")?;
        let online = if scope == self.scope {
            self.connected
        } else {
            self.other_devices[&scope].connected
        };
        ensure!(!online, "Disconnect this device before forgetting it");

        // Capture the latest shared graph, including background plugin updates.
        // No lifecycle or transport operation occurs until the final save succeeds.
        self.persist()?;
        let backup = self.backup_before_forget()?;
        self.forget_saved_device(device, scope, backup)
    }

    fn forget_saved_device(&mut self, device: &str, scope: String, backup: PathBuf) -> Result<()> {
        let previous_library = self.library.clone();
        let previous_scope = self.scope.clone();
        let previous_selected = self.selected_scope.clone();
        let empty_workspace = self.other_devices.is_empty();
        let mut removed = if empty_workspace {
            let device = format!("offline-streamdeck-plus-{}", uuid::Uuid::new_v4());
            let library = Library::for_model(vec![], &device, self.model);
            let mut replacement = DeviceRuntime::new(Record {
                scope: uuid::Uuid::new_v4().to_string(),
                device: device.clone(),
                name: None,
                model: self.model,
                firmware: "unknown".into(),
                config: Some(device::Config {
                    model: self.model,
                    id: device,
                    serial: None,
                    firmware: "unknown".into(),
                }),
                navigation: Navigation {
                    active_profile: library.active_profile.clone(),
                    active_page: library.active_page.clone(),
                    ..Default::default()
                },
            });
            self.library.profiles.extend(library.profiles);
            replacement.swap(self);
            replacement
        } else {
            if self.scope == scope {
                let next = self.other_devices.keys().next().unwrap().clone();
                self.enter_scope(&next)?;
            }
            self.other_devices.remove(&scope).unwrap()
        };
        if self.selected_scope == scope {
            self.selected_scope = self.scope.clone();
        }
        let controls = self.device_controls.remove(device);
        let saver = self.screensavers.remove(device);
        let profile_ids: HashSet<_> = self
            .library
            .profiles
            .iter()
            .filter(|profile| profile.device == device)
            .map(|profile| profile.id.clone())
            .collect();
        self.library
            .profiles
            .retain(|profile| profile.device != device);
        self.library
            .cursors
            .retain(|profile, _| !profile_ids.contains(profile));
        self.library
            .folder_paths
            .retain(|profile, _| !profile_ids.contains(profile));
        let save = self.library.validate().and_then(|()| self.persist());
        if let Err(error) = save {
            if empty_workspace {
                removed.swap(self);
            } else {
                self.other_devices.insert(scope, removed);
                self.enter_scope(&previous_scope)?;
            }
            self.selected_scope = previous_selected;
            self.library = previous_library;
            if let Some(controls) = controls {
                self.device_controls.insert(device.into(), controls);
            }
            if let Some(saver) = saver {
                self.screensavers.insert(device.into(), saver);
            }
            return Err(error);
        }

        // Bind the departed controls briefly for balanced releases, then discard
        // them. These cancellation methods do not persist or execute click actions.
        removed.swap(self);
        self.cancel_departing_triggers(&HashSet::new());
        self.cancel_all_multi();
        self.release_device_holds();
        self.close_inspectors(None);
        for slot in self.slots.iter().flat_map(Slot::visible_flatten) {
            self.emit(slot, "willDisappear", json!({}));
        }
        for slot in previous_library
            .profiles
            .iter()
            .filter(|profile| profile.device == device)
            .flat_map(|profile| &profile.pages)
            .flat_map(|page| &page.slots)
            .flat_map(Slot::flatten)
        {
            self.remember_removed(&slot.context, &slot.uuid);
        }
        for sender in [self.hardware.take(), self.pending_hardware.take()]
            .into_iter()
            .flatten()
        {
            let _ = sender.send(Draw::Stop);
        }
        removed.swap(self);
        self.clear_device_clipboard(device);

        // Block both already-queued Added events and all old Tagged events until
        // the registry confirms the old worker is joined. Rediscovery after that
        // acknowledgement deliberately starts with a fresh profile and generation.
        let key = crate::device_registry::Key {
            serial: device.into(),
            model: removed.model,
        };
        self.registry_routes.retain(|_, route| route != &scope);
        if let Some(sender) = &self.registry_commands {
            self.forgetting_devices.insert(key.clone());
            if sender
                .send(crate::device_registry::Command::Forget(key))
                .is_err()
            {
                self.log("Device registry is unavailable; restart Deckforge to resume discovery");
            }
        }
        self.update_volume_feedback();
        self.system_status = format!("Device forgotten. Recovery session: {}", backup.display());
        self.log(self.system_status.clone());
        Ok(())
    }

    fn backup_before_forget(&self) -> Result<PathBuf> {
        let directory = self
            .state_path
            .parent()
            .context("State directory missing")?
            .join("forgotten-devices");
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.json", uuid::Uuid::new_v4()));
        // Never replace an earlier recovery copy. Permissions match session.json.
        use std::os::unix::fs::OpenOptionsExt;
        let mut backup = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        backup.write_all(&fs::read(&self.state_path)?)?;
        backup.sync_all()?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forget_requires_confirmation_and_disconnect_then_closes_only_its_contexts() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let a = host.device.clone();
        let context_a = host.slots[0].context.clone();
        host.add_simulated_device("B", Model::Plus).unwrap();
        host.with_device("B", |h| h.assign("Keypad", 0, "test.action"))
            .unwrap();
        assert!(
            apply_control(
                &mut host,
                json!({"command":"device_forget","device":a,"confirm":true})
            )
            .is_err()
        );
        host.device_event(DeviceEvent::Disconnected {
            reason: "test".into(),
        })
        .unwrap();
        assert!(apply_control(&mut host, json!({"command":"device_forget","device":a})).is_err());
        host.select_device("B").unwrap();
        let profile_b = host.library.active_profile.clone();
        let context_b = host.slots[0].context.clone();
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        let (pi, _pi_rx) = mpsc::unbounded_channel();
        host.with_device(&a, |h| {
            h.inspectors.insert(
                "inspector-a".into(),
                Inspector {
                    context: context_a.clone(),
                    plugin: 0,
                    child: None,
                    sender: Some(pi),
                },
            );
            Ok(())
        })
        .unwrap();
        host.forget_device(&a).unwrap();
        assert_eq!(host.device, "B");
        assert_eq!(host.library.active_profile, profile_b);
        assert_eq!(host.slots[0].context, context_b);
        assert!(!host.held.is_empty());
        assert!(host.context_device(&context_a).is_none());
        assert!(host.inspector_device("inspector-a").is_none());
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            events
                .iter()
                .map(|e| e["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["propertyInspectorDidDisappear", "willDisappear"]
        );
        assert!(events.iter().all(|e| e["device"] == a));
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.devices.len(), 1);
        assert_eq!(saved.devices[0].device, "B");
        let backup = fs::read_dir(root.path().join("forgotten-devices"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let original: Saved = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
        assert_eq!(original.devices.len(), 2);
        assert!(original.library.unwrap().slot(&context_a).is_some());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn failed_final_save_restores_selected_and_parked_devices_without_lifecycle() {
        for (target, second_device) in [("test-device", true), ("B", true), ("test-device", false)]
        {
            let root = tempfile::tempdir().unwrap();
            let mut host = super::super::tests::fixture(root.path());
            if second_device {
                host.add_simulated_device("B", Model::Mini).unwrap();
            }
            let before = snapshot(&host);
            let library = serde_json::to_value(&host.library).unwrap();
            let records = serde_json::to_value(host.device_records()).unwrap();
            let (tx, mut rx) = mpsc::unbounded_channel();
            host.plugins[0].sender = Some(tx);
            let scope = host.device_scope(target).unwrap();
            host.state_path = root.path().join("missing").join("session.json");
            assert!(
                host.forget_saved_device(target, scope, root.path().join("backup.json"))
                    .is_err()
            );
            assert_eq!(snapshot(&host)["device"], before["device"]);
            assert_eq!(serde_json::to_value(&host.library).unwrap(), library);
            assert_eq!(
                serde_json::to_value(host.device_records()).unwrap(),
                records
            );
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn failed_recovery_copy_leaves_device_and_profiles_available() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.connected = false;
        fs::write(root.path().join("forgotten-devices"), b"occupied").unwrap();
        let before = serde_json::to_value(host.device_records()).unwrap();
        let device = host.device.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        assert!(host.forget_device(&device).is_err());
        assert_eq!(serde_json::to_value(host.device_records()).unwrap(), before);
        assert!(!host.slots.is_empty());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn forgetting_last_device_opens_empty_workspace_and_stale_registry_events_are_ignored() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let a = host.device.clone();
        let key = crate::device_registry::Key {
            serial: a.clone(),
            model: Model::Plus,
        };
        let (commands, rx) = std::sync::mpsc::channel();
        host.registry_commands = Some(commands);
        host.registry_routes.insert(key.clone(), host.scope.clone());
        host.device_event(DeviceEvent::Disconnected {
            reason: "test".into(),
        })
        .unwrap();
        host.forget_device(&a).unwrap();
        assert!(host.device.starts_with("offline-streamdeck-plus-"));
        assert!(host.slots.is_empty());
        assert_eq!(host.device_records().len(), 1);
        assert!(
            matches!(rx.try_recv().unwrap(), crate::device_registry::Command::Forget(k) if k == key)
        );
        let (draw, received) = std::sync::mpsc::channel();
        host.registry_event(crate::device_registry::Event::Added {
            handle: crate::device_registry::Handle {
                key: key.clone(),
                generation: 4,
                draw,
            },
        })
        .unwrap();
        assert!(matches!(received.try_recv().unwrap(), Draw::Stop));
        host.registry_event(crate::device_registry::Event::Tagged {
            key: key.clone(),
            generation: 4,
            event: DeviceEvent::Connected {
                serial: a.clone(),
                model: Model::Plus,
                firmware: "late".into(),
            },
        })
        .unwrap();
        assert!(!host.connected);
        assert!(host.device_scope(&a).is_none());
        host.registry_event(crate::device_registry::Event::Forgotten { key: key.clone() })
            .unwrap();
        let (draw, _received) = std::sync::mpsc::channel();
        host.registry_event(crate::device_registry::Event::Added {
            handle: crate::device_registry::Handle {
                key: key.clone(),
                generation: 5,
                draw,
            },
        })
        .unwrap();
        host.registry_event(crate::device_registry::Event::Tagged {
            key,
            generation: 5,
            event: DeviceEvent::Connected {
                serial: a.clone(),
                model: Model::Plus,
                firmware: "fresh".into(),
            },
        })
        .unwrap();
        assert_eq!(host.device, a);
        assert!(host.connected);
        assert!(host.slots.is_empty());
    }
}
