//! Device-owned execution state. The host mutex serializes scoped operations;
//! switching the editor does not change any device's visible actions or transport.
use super::*;
use anyhow::ensure;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Navigation {
    pub active_profile: String,
    pub active_page: String,
    pub folders: Vec<String>,
    pub previous: Option<(String, String)>,
    pub bundles: HashMap<String, HashMap<String, Option<String>>>,
}
impl Navigation {
    fn from_library(library: &Library) -> Self {
        Self {
            active_profile: library.active_profile.clone(),
            active_page: library.active_page.clone(),
            folders: library.folders.clone(),
            previous: library.previous.clone(),
            bundles: library.bundles.clone(),
        }
    }
    fn swap(&mut self, library: &mut Library) {
        std::mem::swap(&mut self.active_profile, &mut library.active_profile);
        std::mem::swap(&mut self.active_page, &mut library.active_page);
        std::mem::swap(&mut self.folders, &mut library.folders);
        std::mem::swap(&mut self.previous, &mut library.previous);
        std::mem::swap(&mut self.bundles, &mut library.bundles);
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    pub scope: String,
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub model: Model,
    pub firmware: String,
    pub config: Option<device::Config>,
    pub navigation: Navigation,
}

pub(super) struct DeviceRuntime {
    pub scope: String,
    pub device: String,
    pub device_name: Option<String>,
    pub model: Model,
    pub firmware: String,
    pub device_config: Option<device::Config>,
    pub navigation: Navigation,
    pub slots: Vec<Slot>,
    pub connected: bool,
    pub connection_error: Option<String>,
    pub hardware: Option<std::sync::mpsc::Sender<Draw>>,
    pub transport_generation: Option<u64>,
    pending_hardware: Option<std::sync::mpsc::Sender<Draw>>,
    pub controls_observed: device_session::Controls,
    pub controls_error: Option<String>,
    saver: screensaver::Runtime,
    brightness_repeats: HashMap<String, (u8, Duration)>,
    badges: HashMap<String, feedback::Badge>,
    trigger_descriptions: HashMap<String, Value>,
    triggers: trigger::Runtime,
    virtual_suppressed: HashSet<String>,
    stack_gestures: HashMap<String, crate::dial_stack::Gesture>,
    stack_cooldown: HashMap<String, std::time::Instant>,
    multi_owned: HashSet<String>,
    multi_held: HashMap<String, crate::multi::Entry>,
    multi_pending: HashMap<String, Vec<crate::multi::Entry>>,
    multi_preparing: bool,
    multi: crate::multi::Scheduler,
    pub inspectors: HashMap<String, Inspector>,
    dial_pressed: Vec<bool>,
    sensor_pressed: Vec<bool>,
    held: HashMap<HeldControl, String>,
    released: HashSet<HeldControl>,
}
impl DeviceRuntime {
    pub fn new(record: Record) -> Self {
        let model = record.model;
        Self {
            scope: record.scope,
            device: record.device,
            device_name: record.name,
            model,
            firmware: record.firmware,
            device_config: record.config,
            navigation: record.navigation,
            slots: vec![],
            connected: false,
            connection_error: None,
            hardware: None,
            transport_generation: None,
            pending_hardware: None,
            controls_observed: Default::default(),
            controls_error: None,
            saver: Default::default(),
            brightness_repeats: HashMap::new(),
            badges: HashMap::new(),
            trigger_descriptions: HashMap::new(),
            triggers: Default::default(),
            virtual_suppressed: HashSet::new(),
            stack_gestures: HashMap::new(),
            stack_cooldown: HashMap::new(),
            multi_owned: HashSet::new(),
            multi_held: HashMap::new(),
            multi_pending: HashMap::new(),
            multi_preparing: false,
            multi: crate::multi::Scheduler::new(),
            inspectors: HashMap::new(),
            dial_pressed: vec![false; model.dial_count() as usize],
            sensor_pressed: vec![false; model.sensor_count() as usize],
            held: HashMap::new(),
            released: HashSet::new(),
        }
    }
    pub fn record(&self) -> Record {
        Record {
            scope: self.scope.clone(),
            device: self.device.clone(),
            name: self.device_name.clone(),
            model: self.model,
            firmware: self.firmware.clone(),
            config: self.device_config.clone(),
            navigation: self.navigation.clone(),
        }
    }
    fn swap(&mut self, host: &mut Host) {
        macro_rules! swap {($($field:ident),+)=>{$(std::mem::swap(&mut self.$field,&mut host.$field);)+};}
        swap!(
            scope,
            device,
            device_name,
            model,
            firmware,
            device_config,
            slots,
            connected,
            connection_error,
            hardware,
            transport_generation,
            pending_hardware,
            controls_observed,
            controls_error,
            saver,
            brightness_repeats,
            badges,
            trigger_descriptions,
            triggers,
            virtual_suppressed,
            stack_gestures,
            stack_cooldown,
            multi_owned,
            multi_held,
            multi_pending,
            multi_preparing,
            multi,
            inspectors,
            dial_pressed,
            sensor_pressed,
            held,
            released
        );
        self.navigation.swap(&mut host.library);
    }
}
impl Host {
    pub(super) fn device_display_name(&self) -> &str {
        self.device_name.as_deref().unwrap_or(self.model.name())
    }
    pub(super) fn device_info(&self) -> Value {
        let mut info = device::info(self.model);
        info["name"] = json!(self.device_display_name());
        info
    }
    pub(super) fn rename_device(&mut self, name: &str) -> Result<()> {
        ensure!(
            name.chars().count() <= 64 && !name.chars().any(char::is_control),
            "Device name must contain at most 64 characters and no control characters"
        );
        let name = name.trim();
        // An empty name resets to the model's default name, including after upgrades.
        let next = (!name.is_empty()).then(|| name.to_owned());
        if self.device_name == next {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.device_name, next);
        if let Err(error) = self.persist() {
            self.device_name = previous;
            return Err(error);
        }
        self.device_broadcast(
            json!({"event":"deviceDidChange","device":self.device,"deviceInfo":self.device_info()}),
        );
        Ok(())
    }
    pub(super) fn current_record(&self) -> Record {
        Record {
            scope: self.scope.clone(),
            device: self.device.clone(),
            name: self.device_name.clone(),
            model: self.model,
            firmware: self.firmware.clone(),
            config: self.device_config.clone(),
            navigation: Navigation::from_library(&self.library),
        }
    }
    pub(super) fn device_records(&self) -> Vec<Record> {
        let mut records: Vec<_> = self
            .other_devices
            .values()
            .map(DeviceRuntime::record)
            .collect();
        records.push(self.current_record());
        records.sort_by(|a, b| a.scope.cmp(&b.scope));
        records
    }
    fn enter_scope(&mut self, scope: &str) -> Result<()> {
        if self.scope == scope {
            return Ok(());
        }
        let mut target = self
            .other_devices
            .remove(scope)
            .context("Unknown device scope")?;
        target.swap(self);
        self.other_devices.insert(target.scope.clone(), target);
        Ok(())
    }
    pub(super) fn with_scope<T>(
        &mut self,
        scope: &str,
        operation: impl FnOnce(&mut Host) -> Result<T>,
    ) -> Result<T> {
        let previous = self.scope.clone();
        self.enter_scope(scope)?;
        let result = operation(self);
        // Restore on both success and error; unrelated device state stays parked.
        self.enter_scope(&previous)?;
        result
    }
    pub(super) fn device_scope(&self, device: &str) -> Option<String> {
        if self.device == device {
            Some(self.scope.clone())
        } else {
            self.other_devices
                .values()
                .find(|state| state.device == device)
                .map(|state| state.scope.clone())
        }
    }
    pub(super) fn with_device<T>(
        &mut self,
        device: &str,
        operation: impl FnOnce(&mut Host) -> Result<T>,
    ) -> Result<T> {
        let scope = self.device_scope(device).context("Unknown device")?;
        self.with_scope(&scope, operation)
    }
    pub(super) fn device_ids(&self) -> Vec<String> {
        self.device_records()
            .into_iter()
            .map(|record| record.device)
            .collect()
    }
    pub(super) fn registration_devices(&self) -> Vec<Value> {
        self.device_records()
            .into_iter()
            .map(|record| {
                let mut info = device::info(record.model);
                if let Some(name) = record.name {
                    info["name"] = json!(name);
                }
                info["id"] = json!(record.device);
                info
            })
            .collect()
    }
    pub(super) fn context_device(&self, context: &str) -> Option<String> {
        if self
            .slots
            .iter()
            .any(|slot| slot.flatten().iter().any(|slot| slot.context == context))
        {
            return Some(self.device.clone());
        }
        if let Some(state) = self.other_devices.values().find(|state| {
            state
                .slots
                .iter()
                .any(|slot| slot.flatten().iter().any(|slot| slot.context == context))
        }) {
            return Some(state.device.clone());
        }
        self.library
            .profiles
            .iter()
            .find(|profile| {
                profile.pages.iter().any(|page| {
                    page.slots
                        .iter()
                        .any(|slot| slot.flatten().iter().any(|slot| slot.context == context))
                })
            })
            .map(|profile| profile.device.clone())
    }
    pub(super) fn editable_context(&self, context: &str) -> Result<()> {
        let profile = self
            .library
            .profiles
            .iter()
            .find(|profile| {
                profile.pages.iter().any(|page| {
                    page.slots
                        .iter()
                        .any(|slot| slot.flatten().iter().any(|slot| slot.context == context))
                })
            })
            .context("Action profile no longer exists")?;
        ensure!(
            profile.device == self.device,
            "Action belongs to another device"
        );
        ensure!(
            !profile.readonly,
            "This bundled profile is read-only; duplicate it to edit"
        );
        Ok(())
    }
    pub(super) fn inspector_device(&self, registration: &str) -> Option<String> {
        if self.inspectors.contains_key(registration) {
            Some(self.device.clone())
        } else {
            self.other_devices
                .values()
                .find(|state| state.inspectors.contains_key(registration))
                .map(|state| state.device.clone())
        }
    }
    pub(super) fn sync_all_devices(&mut self) {
        self.sync_visible();
        for state in self.other_devices.values() {
            for slot in &state.slots {
                self.library.update(slot);
            }
        }
    }
    pub(super) fn render_all_devices(&mut self) -> Result<()> {
        for id in self.device_ids() {
            self.with_device(&id, Host::render)?;
        }
        Ok(())
    }
    pub(super) fn select_device(&mut self, device: &str) -> Result<()> {
        let scope = self.device_scope(device).context("Unknown device")?;
        self.enter_scope(&scope)?;
        self.selected_scope = scope;
        self.persist()
    }
    pub(super) fn add_device_runtime(
        &mut self,
        device: &str,
        model: Model,
        connected: bool,
        config: Option<device::Config>,
    ) -> Result<()> {
        ensure!(self.device_scope(device).is_none(), "Device already exists");
        ensure!(
            !device.is_empty() && device.len() <= 4096 && !device.contains('\0'),
            "Invalid device identity"
        );
        ensure!(
            self.other_devices.len() + 1 < crate::device_registry::MAX_WORKERS,
            "At most32 devices can be associated"
        );
        let library = Library::for_model(vec![], device, model);
        let record = Record {
            scope: uuid::Uuid::new_v4().to_string(),
            device: device.into(),
            name: None,
            model,
            firmware: if self.simulate {
                "simulation".into()
            } else {
                "unknown".into()
            },
            config,
            navigation: Navigation::from_library(&library),
        };
        let mut state = DeviceRuntime::new(record);
        state.connected = connected;
        state.slots = library.visible()?;
        self.library.profiles.extend(library.profiles);
        self.other_devices.insert(state.scope.clone(), state);
        self.with_device(device,|host|{
            if connected {host.device_broadcast(json!({"event":"deviceDidConnect","device":host.device,"deviceInfo":host.device_info()}));host.install_bundled_profiles(true)?;}
            host.hydrate_library_children();host.slots=host.library.visible()?;host.render()?;
            host.persist()
        })
    }
    pub(super) fn copy_profile_to_device(&mut self, request: &Value) -> Result<()> {
        self.sync_all_devices();
        let profile = request["profile"]
            .as_str()
            .context("Source profile missing")?;
        let target = request["target_device"]
            .as_str()
            .context("Target device missing")?;
        let record = self
            .device_records()
            .into_iter()
            .find(|record| record.device == target)
            .context("Unknown destination device")?;
        let source = self
            .library
            .profiles
            .iter()
            .find(|p| p.id == profile)
            .context("Profile missing")?;
        ensure!(
            source.device == self.device,
            "Source profile belongs to another device"
        );
        ensure!(
            source.model.profile_compatible(record.model),
            "Profile geometry is incompatible with the destination device"
        );
        let name = request["name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} copy", source.name));
        ensure!(
            !name.trim().is_empty() && name.len() <= 256,
            "Invalid profile name"
        );
        let mut copy = library::copied_profile(source, &name, &self.actions)?;
        copy.device = target.into();
        let id = copy.id.clone();
        self.library.profiles.push(copy);
        self.with_device(target, |host| host.select_profile(&id, None))
    }
    pub(super) fn startup_device_selection(
        &mut self,
        serial: Option<&str>,
        model: Option<Model>,
    ) -> Result<()> {
        if serial.is_none() && model.is_none() {
            return Ok(());
        }
        if let Some(record) = self.device_records().into_iter().find(|record| {
            serial.is_none_or(|serial| record.device == serial)
                && model.is_none_or(|model| record.model == model)
        }) {
            return self.select_device(&record.device);
        }
        let model = model.unwrap_or_default();
        let device = serial
            .map(str::to_owned)
            .unwrap_or_else(|| format!("offline-streamdeck-plus-{}", uuid::Uuid::new_v4()));
        ensure!(
            self.device_scope(&device).is_none(),
            "The selected serial belongs to a different device model"
        );
        self.add_device_runtime(
            &device,
            model,
            self.simulate,
            Some(device::Config {
                model,
                id: device.clone(),
                serial: serial.map(str::to_owned),
                firmware: "unknown".into(),
            }),
        )?;
        self.select_device(&device)
    }
    pub(super) fn restore_device_records(
        &mut self,
        records: Vec<Record>,
        selected: Option<String>,
    ) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        ensure!(
            records.len() <= crate::device_registry::MAX_WORKERS,
            "Saved session exceeds32 devices"
        );
        let mut scopes = HashSet::new();
        let mut ids = HashSet::new();
        for record in &records {
            ensure!(
                scopes.insert(record.scope.clone()) && ids.insert(record.device.clone()),
                "Duplicate saved device identity"
            );
            ensure!(
                !record.scope.is_empty() && !record.device.is_empty(),
                "Empty saved device identity"
            );
        }
        ensure!(
            scopes.contains(&self.scope),
            "Active saved device is missing"
        );
        let current = records.iter().find(|r| r.scope == self.scope).unwrap();
        self.device_name = current.name.clone();
        ensure!(
            current.device == self.device && current.model == self.model,
            "Saved active device metadata does not match"
        );
        let profile = self.library.current()?;
        ensure!(
            profile.device == self.device && profile.model.profile_compatible(self.model),
            "Saved active profile belongs to another device"
        );
        for record in records {
            if record.scope == self.scope {
                continue;
            }
            let key = record.scope.clone();
            self.other_devices
                .insert(key.clone(), DeviceRuntime::new(record));
            self.with_scope(&key, |host| {
                let profile = host.library.current()?;
                ensure!(
                    profile.device == host.device && profile.model.profile_compatible(host.model),
                    "Saved device profile does not match its owner"
                );
                host.slots = host.library.visible()?;
                host.connected = host.simulate;
                host.hydrate_library_children();
                host.render()
            })?;
        }
        if let Some(selected) = selected {
            self.enter_scope(&selected)?;
            self.selected_scope = selected;
        }
        Ok(())
    }
    pub(super) fn add_simulated_device(&mut self, device: &str, model: Model) -> Result<()> {
        ensure!(self.simulate, "Simulated devices require simulation mode");
        self.add_device_runtime(device, model, true, None)
    }
    pub(super) fn registry_event(&mut self, event: crate::device_registry::Event) -> Result<()> {
        use crate::device_registry::Event;
        if !self.running {
            return Ok(());
        }
        match event {
            Event::Diagnostic { error } => {
                if !self.logs.contains(&error) {
                    self.log(error);
                }
                Ok(())
            }
            Event::Added { handle } => {
                let scope = self.device_scope(&handle.key.serial).or_else(|| {
                    self.device_records()
                        .into_iter()
                        .find(|record| {
                            record.device.starts_with("offline-streamdeck-plus-")
                                && record.model == handle.key.model
                                && record
                                    .config
                                    .as_ref()
                                    .is_none_or(|config| config.serial.is_none())
                        })
                        .map(|record| record.scope)
                });
                let scope = match scope {
                    Some(scope) => scope,
                    None => {
                        self.add_device_runtime(
                            &handle.key.serial,
                            handle.key.model,
                            false,
                            Some(device::Config {
                                model: handle.key.model,
                                id: handle.key.serial.clone(),
                                serial: Some(handle.key.serial.clone()),
                                firmware: "unknown".into(),
                            }),
                        )?;
                        self.device_scope(&handle.key.serial).unwrap()
                    }
                };
                self.registry_routes.insert(handle.key, scope.clone());
                self.with_scope(&scope, |host| {
                    host.hardware = None;
                    host.pending_hardware = Some(handle.draw);
                    host.transport_generation = Some(handle.generation);
                    Ok(())
                })
            }
            Event::Tagged {
                key,
                generation,
                event,
            } => {
                let Some(scope) = self.registry_routes.get(&key).cloned() else {
                    return Ok(());
                };
                self.with_scope(&scope, |host| {
                    if host.transport_generation != Some(generation) {
                        return Ok(());
                    }
                    let connected = matches!(event, device_session::Event::Connected { .. });
                    host.device_event(event)?;
                    if connected {
                        if let Some(draw) = host.pending_hardware.take() {
                            host.hardware = Some(draw);
                            host.send_device_controls();
                            host.render()?;
                        }
                        host.install_bundled_profiles(true)?;
                    }
                    Ok(())
                })
            }
        }
    }
    pub(super) fn devices_snapshot(&self) -> Value {
        let profile_model = self
            .library
            .current()
            .map_or(self.model, |profile| profile.model);
        let mut records = vec![
            json!({"id":self.device,"name":self.device_display_name(),"model":self.model,"connected":self.connected&&!self.simulate,"online":self.connected,"firmware":self.firmware,"selected":self.scope==self.selected_scope,"profile_compatible":profile_model.profile_compatible(self.model)}),
        ];
        records.extend(self.other_devices.values().map(|state|json!({"id":state.device,"name":state.device_name.as_deref().unwrap_or(state.model.name()),"model":state.model,"connected":state.connected&&!self.simulate,"online":state.connected,"firmware":state.firmware,"selected":state.scope==self.selected_scope,"profile_compatible":profile_model.profile_compatible(state.model)})));
        json!(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Host {
        super::super::tests::fixture(root)
    }
    #[test]
    fn names_are_scoped_persisted_and_announced_without_action_lifecycle() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let a = host.device.clone();
        host.add_simulated_device("B", Model::Mini).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        let context = host.slots[0].context.clone();
        super::super::apply_control(
            &mut host,
            json!({"command":"device_rename","device":"B","name":"  Studio 🎛  "}),
        )
        .unwrap();
        assert_eq!(host.device, a);
        assert_eq!(host.slots[0].context, context);
        assert_eq!(host.device_display_name(), Model::Plus.name());
        let changed = rx.try_recv().unwrap();
        assert_eq!(
            changed,
            json!({"event":"deviceDidChange","device":"B","deviceInfo":{"name":"Studio 🎛","type":Model::Mini.sdk_type(),"size":{"columns":3,"rows":2}}})
        );
        assert!(rx.try_recv().is_err());
        assert_eq!(
            host.registration_devices()
                .iter()
                .find(|v| v["id"] == "B")
                .unwrap()["name"],
            "Studio 🎛"
        );
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(
            saved
                .devices
                .iter()
                .find(|r| r.device == "B")
                .unwrap()
                .name
                .as_deref(),
            Some("Studio 🎛")
        );
        host.with_device("B", |h| h.rename_device("Studio 🎛"))
            .unwrap();
        assert!(rx.try_recv().is_err());
        host.with_device("B", |h| h.rename_device("")).unwrap();
        assert_eq!(
            rx.try_recv().unwrap()["deviceInfo"]["name"],
            Model::Mini.name()
        );
        assert!(host.device_records().iter().all(|r| r.name.is_none()));
    }
    #[test]
    fn name_validation_and_failed_save_preserve_current_metadata() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.rename_device("Desk").unwrap();
        rx.try_recv().unwrap();
        for name in ["a".repeat(65), "bad\nname".into(), "bad\0name".into()] {
            assert!(host.rename_device(&name).is_err());
            assert_eq!(host.device_display_name(), "Desk");
        }
        host.state_path = root.path().join("absent").join("session.json");
        assert!(host.rename_device("Lost").is_err());
        assert_eq!(host.device_display_name(), "Desk");
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn transport_is_bound_only_after_model_identity_and_stale_generations_are_ignored() {
        use crate::device_registry::{Event, Handle, Key};
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let key = Key {
            serial: host.device.clone(),
            model: Model::Mini,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        host.registry_event(Event::Added {
            handle: Handle {
                key: key.clone(),
                generation: 7,
                draw: tx,
            },
        })
        .unwrap();
        host.render().unwrap();
        assert!(rx.try_recv().is_err());
        host.registry_event(Event::Tagged {
            key: key.clone(),
            generation: 6,
            event: DeviceEvent::Disconnected {
                reason: "stale".into(),
            },
        })
        .unwrap();
        assert!(host.connected);
        host.registry_event(Event::Tagged {
            key: key.clone(),
            generation: 7,
            event: DeviceEvent::Connected {
                serial: key.serial.clone(),
                model: Model::Mini,
                firmware: "test".into(),
            },
        })
        .unwrap();
        assert_eq!(host.model, Model::Mini);
        let frames: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            frames
                .iter()
                .filter(|frame| matches!(frame, Draw::Key(..)))
                .count(),
            6
        );
        assert!(
            !frames
                .iter()
                .any(|frame| matches!(frame, Draw::Strip(_) | Draw::Infobar(_) | Draw::Sensor(..)))
        );
        host.registry_event(Event::Tagged {
            key,
            generation: 6,
            event: DeviceEvent::Connected {
                serial: "wrong".into(),
                model: Model::Xl,
                firmware: "stale".into(),
            },
        })
        .unwrap();
        assert_eq!(host.model, Model::Mini);
        assert_ne!(host.device, "wrong");
    }
    #[test]
    fn another_devices_profile_cannot_be_selected_renamed_or_deleted() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let a = host.library.active_profile.clone();
        host.add_simulated_device("B", Model::Plus).unwrap();
        host.select_device("B").unwrap();
        for command in ["profile_select", "profile_rename", "profile_delete"] {
            assert!(
                host.library_command(&json!({"command":command,"id":a,"name":"wrong"}))
                    .is_err()
            );
        }
        let b = host.library.active_profile.clone();
        assert!(
            host.library_command(&json!({"command":"profile_delete","id":b}))
                .is_err()
        );
        assert_eq!(host.library.profiles.len(), 2);
    }
    #[test]
    fn restarting_after_background_save_restores_large_selected_layout() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        host.state_path = root.path().join("session.json");
        let a = host.device.clone();
        host.add_simulated_device("XL-B", Model::Xl).unwrap();
        host.with_device("XL-B", |host| host.assign("Keypad", 31, "test.action"))
            .unwrap();
        host.select_device("XL-B").unwrap();
        host.rename_device("Main XL").unwrap();
        host.with_device(&a, |host| {
            host.slots[0].settings = json!({"latest":42});
            host.rename_device("Small desk")
        })
        .unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_ne!(saved["active_scope"], saved["selected_scope"]);
        let running = super::super::start(HostOptions {
            model: None,
            serial: None,
            plugins: vec![],
            profile: None,
            state_dir: root.path().to_owned(),
            port: 0,
            simulate: true,
            seconds: None,
        })
        .unwrap();
        let snapshot = running.controller.snapshot();
        assert_eq!(snapshot["device_name"], "Main XL");
        assert_eq!(
            snapshot["devices"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["id"] == a)
                .unwrap()["name"],
            "Small desk"
        );
        assert_eq!(snapshot["device"], "XL-B");
        assert_eq!(snapshot["device_model"], "xl");
        assert_eq!(snapshot["slots"][0]["index"], 31);
        assert_eq!(snapshot["devices"].as_array().unwrap().len(), 2);
    }
}
