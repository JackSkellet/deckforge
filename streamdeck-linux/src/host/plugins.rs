//! Owned plugin processes and WebSocket generations. Failures stay local to one
//! plugin; retries are explicit and never replace saved action/global settings.
use super::*;
use sha2::{Digest, Sha256};
use std::{os::unix::ffi::OsStrExt, time::Instant};

pub(super) fn new_asset_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(super) struct Runtime {
    pub status: &'static str,
    pub error: Option<String>,
    pub session: Option<String>,
    pub output_drain: Option<super::transport::Drain>,
    pub started: Option<Instant>,
    pub log_path: Option<PathBuf>,
    pub entry_path: Option<PathBuf>,
    pub entry_field: Option<&'static str>,
    pub runtime_path: Option<PathBuf>,
    pub load_failed: bool,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            status: "stopped",
            error: None,
            session: None,
            output_drain: None,
            started: None,
            log_path: None,
            entry_path: None,
            entry_field: None,
            runtime_path: None,
            load_failed: false,
        }
    }
}
impl Runtime {
    pub fn starting() -> Self {
        Self {
            status: "starting",
            started: Some(Instant::now()),
            ..Self::default()
        }
    }
    pub fn fail(&mut self, error: String) {
        self.status = "failed";
        self.error = Some(error);
        self.session = None;
    }
}
impl Plugin {
    pub(super) fn snapshot(&self) -> Value {
        json!({"id":self.id,"name":self.name,"connected":self.sender.is_some(),
            "status":self.runtime.status,"error":self.runtime.error,"path":self.root,
            "version":self.manifest["Version"],"log_path":self.runtime.log_path,
            "entry_path":self.runtime.entry_path,"entry_field":self.runtime.entry_field,"runtime_path":self.runtime.runtime_path,
            "pid":self.child.as_ref().map(Child::id),
            "started_seconds":self.runtime.started.map(|start|start.elapsed().as_secs())})
    }
}

pub(super) fn parse(path: &Path, reference: Option<&Path>) -> Result<(Plugin, Vec<Action>)> {
    let root = path
        .canonicalize()
        .with_context(|| format!("Plugin directory unavailable: {}", path.display()))?;
    let manifest = crate::package::read_installed_manifest(&root, reference)?;
    let id = manifest["UUID"]
        .as_str()
        .filter(|value| !value.is_empty())
        .context("Plugin UUID missing")?
        .to_owned();
    let mut actions = Vec::new();
    for action in manifest["Actions"]
        .as_array()
        .context("Plugin Actions missing")?
    {
        let uuid = action["UUID"]
            .as_str()
            .filter(|value| !value.is_empty())
            .context("Action UUID missing")?
            .to_owned();
        if mixer_actions::supported(&uuid)
            || crate::system_actions::supported(&uuid)
            || crate::audio_output::supported(&uuid)
            || library::navigation_actions().iter().any(|a| a.uuid == uuid)
            || multiaction::container(&uuid)
            || uuid == multiaction::DELAY
            || uuid == crate::dial_stack::UUID
            || uuid == crate::action_trigger::UUID
            || uuid == crate::key_logic::UUID
            || uuid == crate::action_wheel::UUID
        {
            bail!("Plugin action UUID is already owned by a built-in action: {uuid}");
        }
        if actions.iter().any(|a: &Action| a.uuid == uuid) {
            bail!("Duplicate action {uuid}");
        }
        actions.push(Action {
            uuid,
            name: action["Name"].as_str().unwrap_or("Action").into(),
            controllers: action["Controllers"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_else(|| vec!["Keypad".into()]),
            plugin: id.clone(),
            raw: action.clone(),
        });
    }
    Ok((
        Plugin {
            asset_generation: new_asset_generation(),
            id,
            name: manifest["Name"].as_str().unwrap_or("Plugin").into(),
            root,
            manifest,
            registration: uuid::Uuid::new_v4().to_string(),
            runtime: Runtime::default(),
            child: None,
            sender: None,
        },
        actions,
    ))
}

fn unavailable(path: &Path, error: anyhow::Error) -> Result<Plugin> {
    let root = path.canonicalize().unwrap_or(std::path::absolute(path)?);
    let id = format!(
        "unavailable.{:x}",
        Sha256::digest(root.as_os_str().as_bytes())
    );
    let mut runtime = Runtime {
        load_failed: true,
        ..Runtime::default()
    };
    runtime.fail(format!("{error:#}"));
    Ok(Plugin {
        asset_generation: new_asset_generation(),
        id,
        name: root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        root,
        manifest: Value::Null,
        registration: uuid::Uuid::new_v4().to_string(),
        runtime,
        child: None,
        sender: None,
    })
}

pub(super) fn load(
    paths: Vec<PathBuf>,
    reference: Option<&Path>,
) -> Result<(Vec<Plugin>, Vec<Action>)> {
    let mut plugins: Vec<Plugin> = Vec::new();
    let mut actions: Vec<Action> = Vec::new();
    for path in paths {
        let absolute = path.canonicalize().unwrap_or(std::path::absolute(&path)?);
        if plugins.iter().any(|plugin| plugin.root == absolute) {
            continue;
        }
        let parsed = parse(&absolute, reference).and_then(|(plugin, incoming)| {
            if plugins.iter().any(|p| p.id == plugin.id) {
                bail!("Duplicate plugin UUID {}", plugin.id);
            }
            if incoming
                .iter()
                .any(|a| actions.iter().any(|old| old.uuid == a.uuid))
            {
                bail!("Plugin declares an action UUID already owned by another installed plugin");
            }
            Ok((plugin, incoming))
        });
        match parsed {
            Ok((plugin, incoming)) => {
                plugins.push(plugin);
                actions.extend(incoming);
            }
            Err(error) => plugins.push(unavailable(&absolute, error)?),
        }
    }
    Ok((plugins, actions))
}

impl Host {
    pub(super) fn plugin_enabled(&self, plugin: &Plugin) -> bool {
        !self.disabled_plugins.contains(&plugin.root)
    }
    pub(super) fn action_plugin_disabled(&self, uuid: &str) -> bool {
        self.actions
            .iter()
            .find(|a| a.uuid == uuid)
            .and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin))
            .is_some_and(|p| !self.plugin_enabled(p))
    }
    pub(super) fn set_plugin_enabled(&mut self, request: &Value) -> Result<()> {
        let id = request["id"].as_str().context("Plugin id missing")?;
        let enabled = request["enabled"]
            .as_bool()
            .context("Plugin enabled state missing")?;
        let expected = request["expected_enabled"]
            .as_bool()
            .context("Expected plugin state missing")?;
        let index = self
            .plugins
            .iter()
            .position(|p| p.id == id)
            .context("Plugin is no longer installed")?;
        let previous = self.plugin_enabled(&self.plugins[index]);
        anyhow::ensure!(
            previous == expected,
            "Plugin state changed; refresh before changing it again"
        );
        if previous == enabled {
            return Ok(());
        }
        let root = self.plugins[index].root.clone();
        if enabled {
            self.disabled_plugins.remove(&root);
        } else {
            self.disabled_plugins.insert(root.clone());
        }
        // Commit the preference before disrupting a working process. A failed
        // save leaves the previous process, inspectors and held controls intact.
        if let Err(error) = self.persist() {
            if previous {
                self.disabled_plugins.remove(&root);
            } else {
                self.disabled_plugins.insert(root);
            }
            return Err(error);
        }
        if enabled {
            // A launch failure stays visible as enabled-but-failed and can be
            // retried; it must not silently revert the user's saved preference.
            return self.restart_plugin(id);
        }
        self.stop_plugin_runtime(index, id);
        self.log(format!("Plugin disabled: {id}"));
        if let Some(redraw) = &self.redraw {
            redraw();
        }
        Ok(())
    }
    pub(super) fn stop_plugin_runtime(&mut self, index: usize, id: &str) {
        for device in self.device_ids() {
            if let Err(error) = self.with_device(&device, |host| {
                host.prepare_plugin_restart(index, id);
                for slot in host.slots.iter().flat_map(Slot::visible_flatten) {
                    if host
                        .actions
                        .iter()
                        .any(|a| a.uuid == slot.uuid && a.plugin == id)
                    {
                        host.emit(slot, "willDisappear", json!({}));
                    }
                }
                host.sync_visible();
                Ok(())
            }) {
                self.log(format!("Plugin stop cleanup: {error:#}"));
            }
        }
        let plugin = &mut self.plugins[index];
        plugin.runtime.session = None;
        plugin.registration = uuid::Uuid::new_v4().to_string();
        let drained = plugin.drain_output();
        stop_child(&mut plugin.child);
        plugin.runtime.status = "disabled";
        plugin.runtime.started = None;
        if !drained {
            self.log(format!(
                "Plugin output drain failed or timed out; stopped owned child: {id}"
            ));
        }
        self.discard_plugin_links(id);
        self.application_delivered.remove(id);
        self.poll_applications();
    }
    pub(super) fn plugin_session_index(&self, launch: &str, session: &str) -> Option<usize> {
        self.plugins.iter().enumerate().find_map(|(index, _)| {
            self.plugin_session_current(index, launch, session)
                .then_some(index)
        })
    }
    pub(super) fn plugin_session_current(&self, index: usize, launch: &str, session: &str) -> bool {
        self.plugins.get(index).is_some_and(|plugin| {
            self.plugin_enabled(plugin)
                && plugin.registration == launch
                && plugin.runtime.session.as_deref() == Some(session)
                && plugin.sender.is_some()
        })
    }
    pub(super) fn plugin_disconnected(&mut self, index: usize, launch: &str, session: &str) {
        if !self.plugin_session_current(index, launch, session) {
            return;
        }
        let plugin = &mut self.plugins[index];
        plugin.sender = None;
        plugin.runtime.session = None;
        plugin.runtime.status = "disconnected";
        plugin.runtime.error =
            Some("Plugin connection closed. Check its log or restart the plugin.".into());
        let id = plugin.id.clone();
        self.log(format!("Plugin disconnected: {id}"));
        if let Some(redraw) = &self.redraw {
            redraw();
        }
    }
    pub(super) fn restart_plugin(&mut self, id: &str) -> Result<()> {
        let index = self
            .plugins
            .iter()
            .position(|p| p.id == id)
            .context("Plugin is no longer installed")?;
        anyhow::ensure!(
            self.plugin_enabled(&self.plugins[index]),
            "Enable the plugin before restarting it"
        );
        let root = self.plugins[index].root.clone();
        let state = self
            .state_path
            .parent()
            .context("Missing plugin state directory")?
            .to_owned();
        for device in self.device_ids() {
            self.with_device(&device, |host| {
                host.prepare_plugin_restart(index, id);
                host.sync_visible();
                Ok(())
            })?;
        }
        self.plugins[index].runtime.session = None;
        self.plugins[index].registration = uuid::Uuid::new_v4().to_string();
        let drained = self.plugins[index].drain_output();
        stop_child(&mut self.plugins[index].child);
        if !drained {
            self.log(format!(
                "Plugin output drain failed or timed out; stopped owned child: {id}"
            ));
        }
        let result = (|| {
            let (mut plugin, incoming) = parse(&root, self.reference_app.as_deref())?;
            if self
                .plugins
                .iter()
                .enumerate()
                .any(|(other, p)| other != index && p.id == plugin.id)
            {
                bail!("Plugin UUID already belongs to another installed plugin");
            }
            if incoming.iter().any(|action| {
                self.actions
                    .iter()
                    .any(|old| old.plugin != id && old.uuid == action.uuid)
            }) {
                bail!("Plugin declares an action UUID already owned by another installed plugin");
            }
            self.sync_visible();
            let previous: Vec<_> = self
                .actions
                .iter()
                .filter(|action| action.plugin == id)
                .cloned()
                .collect();
            self.actions.retain(|action| action.plugin != id);
            self.actions.extend(incoming.clone());
            plugin.runtime.load_failed = false;
            self.plugins[index] = plugin;
            self.refresh_localizations();
            self.refresh_updated_plugin_assets(&previous, &incoming);
            self.refresh_previous_plugins();
            self.recover_updated_plugin_assets()?;
            self.record_plugin_asset_identity(index)?;
            for device in self.device_ids() {
                self.with_device(&device, |host| {
                    host.sync_visible();
                    host.hydrate_library_children();
                    host.slots = host.library.visible()?;
                    for index in 0..host.slots.len() {
                        if host.slots[index].background.is_none() {
                            let slot = host.slots[index].clone();
                            host.slots[index].background =
                                host.resolve_state_image(&slot, slot.state);
                        }
                    }
                    host.render()?;
                    host.persist()
                })?;
            }
            let devices = self.registration_devices();
            launch_plugin(
                &mut self.plugins[index],
                &devices,
                self.port,
                &state,
                self.language,
            )
        })();
        if let Err(error) = &result {
            self.plugins[index].runtime.fail(format!("{error:#}"));
        }
        if let Some(redraw) = &self.redraw {
            redraw();
        }
        result
    }
    fn prepare_plugin_restart(&mut self, index: usize, id: &str) {
        // Revoke only this plugin's action instances and container runs that
        // include them, while the old sender can still receive balanced releases.
        let owned: HashSet<_> = self
            .actions
            .iter()
            .filter(|action| action.plugin == id)
            .map(|action| action.uuid.as_str())
            .collect();
        let unaffected: HashSet<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|slot| {
                !slot
                    .flatten()
                    .iter()
                    .any(|child| owned.contains(child.uuid.as_str()))
            })
            .map(|slot| slot.context.clone())
            .collect();
        self.cancel_departing_triggers(&unaffected);
        self.cancel_departing_stacks(&unaffected);
        self.cancel_departing_multi(&unaffected);
        self.cancel_departing_inputs(&unaffected);
        let inspectors: Vec<_> = self
            .inspectors
            .iter()
            .filter(|(_, inspector)| inspector.plugin == index)
            .map(|(registration, _)| registration.clone())
            .collect();
        for inspector in inspectors {
            self.close_inspector(&inspector);
        }
    }
    pub(super) fn monitor_plugins(&mut self) {
        let mut exited = Vec::new();
        let mut waiting = Vec::new();
        for (index, plugin) in self.plugins.iter_mut().enumerate() {
            let Some(child) = &mut plugin.child else {
                continue;
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    exited.push((
                        index,
                        format!("Plugin exited ({status}). Check its log or restart."),
                    ));
                }
                Err(error) => {
                    let message = format!("Cannot check plugin process: {error}");
                    if plugin.runtime.error.as_ref() != Some(&message) {
                        waiting.push(message.clone());
                        plugin.runtime.error = Some(message);
                    }
                }
                Ok(None)
                    if plugin.runtime.status == "starting"
                        && plugin.runtime.error.is_none()
                        && plugin
                            .runtime
                            .started
                            .is_some_and(|start| start.elapsed() >= Duration::from_secs(15)) =>
                {
                    let message =
                        "Plugin has not connected after 15 seconds. Check its log or restart."
                            .to_owned();
                    plugin.runtime.error = Some(message.clone());
                    waiting.push(format!("{}: {message}", plugin.id));
                }
                Ok(None) => {}
            }
        }
        for (index, message) in exited {
            let plugin = &mut self.plugins[index];
            plugin.child = None; // try_wait already reaped this exact owned child.
            plugin.sender = None;
            plugin.registration = uuid::Uuid::new_v4().to_string();
            plugin.runtime.fail(message.clone());
            let id = plugin.id.clone();
            let log = plugin
                .runtime
                .log_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned());
            let tail = log.map(|name| self.child_log(&name)).unwrap_or_default();
            self.log(format!("Plugin {id}: {message} {tail}"));
            if let Some(redraw) = &self.redraw {
                redraw();
            }
        }
        for message in waiting {
            self.log(message);
            if let Some(redraw) = &self.redraw {
                redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_preference_is_durable_and_failed_save_does_not_disrupt_runtime() {
        let root = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(root.path());
        let id = host.plugins[0].id.clone();
        let original = serde_json::to_value(host.device_records()).unwrap();
        let (sender, mut events) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.plugins[0].registration = "old-launch".into();
        host.plugins[0].runtime.session = Some("old-session".into());
        host.plugins[0].runtime.status = "connected";
        let state_path = host.state_path.clone();
        host.state_path = root.path().join("missing/session.json");
        let disable = json!({"id":id,"enabled":false,"expected_enabled":true});
        assert!(host.set_plugin_enabled(&disable).is_err());
        assert!(host.plugin_session_current(0, "old-launch", "old-session"));
        assert!(host.disabled_plugins.is_empty());
        assert!(events.try_recv().is_err());
        host.state_path = state_path;
        host.set_plugin_enabled(&disable).unwrap();
        assert!(!host.plugin_session_current(0, "old-launch", "old-session"));
        assert_eq!(host.plugins[0].runtime.status, "disabled");
        assert!(host.plugins[0].sender.is_none() && host.plugins[0].child.is_none());
        assert_eq!(
            serde_json::to_value(host.device_records()).unwrap(),
            original
        );
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.disabled_plugins.contains(&host.plugins[0].root));
        assert_eq!(snapshot(&host)["plugins"][0]["enabled"], false);
        assert_eq!(snapshot(&host)["slots"][0]["plugin_disabled"], true);
        assert_eq!(snapshot(&host)["slots"][0]["available"], false);
        assert!(host.restart_plugin(&id).is_err());
        let context = host.slots[0].context.clone();
        assert!(host.open_inspector(&context).is_err());
        assert!(host.assign("Keypad", 1, "test.action").is_err());
        assert!(host.set_plugin_enabled(&disable).is_err()); // stale UI state
        host.plugin_disconnected(0, "old-launch", "old-session");
        host.monitor_plugins();
        assert_eq!(host.plugins[0].runtime.status, "disabled");
        // A bad manifest on explicit enable is a visible launch failure, not an
        // implicit undo of the user's saved preference.
        assert!(
            host.set_plugin_enabled(&json!({"id":id,"enabled":true,"expected_enabled":false}))
                .is_err()
        );
        assert!(host.plugin_enabled(&host.plugins[0]));
        assert_eq!(host.plugins[0].runtime.status, "failed");
    }
    #[test]
    fn missing_malformed_and_builtin_claiming_plugins_remain_visible_without_actions() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.sdPlugin");
        let malformed = root.path().join("malformed.sdPlugin");
        let claiming = root.path().join("claim.sdPlugin");
        fs::create_dir(&malformed).unwrap();
        fs::write(malformed.join("manifest.json"), "{").unwrap();
        fs::create_dir(&claiming).unwrap();
        fs::write(
            claiming.join("manifest.json"),
            json!({"UUID":"test.claim", "Actions":[{"UUID":crate::system_actions::HOTKEY}]})
                .to_string(),
        )
        .unwrap();
        let (plugins, actions) =
            load(vec![missing.clone(), malformed, claiming, missing], None).unwrap();
        assert_eq!(plugins.len(), 3);
        assert!(actions.is_empty());
        assert!(
            plugins
                .iter()
                .all(|plugin| plugin.runtime.status == "failed" && plugin.runtime.error.is_some())
        );
        assert!(
            plugins[2]
                .runtime
                .error
                .as_ref()
                .unwrap()
                .contains("built-in")
        );
        assert!(plugins[0].root.is_absolute());
    }
    #[test]
    fn stale_launch_and_socket_cleanup_cannot_disconnect_a_new_session() {
        let root = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(root.path());
        let (sender, _receiver) = mpsc::unbounded_channel();
        host.plugins[0].registration = "new-launch".into();
        host.plugins[0].runtime.session = Some("new-socket".into());
        host.plugins[0].runtime.status = "connected";
        host.plugins[0].sender = Some(sender);
        host.plugin_disconnected(0, "old-launch", "new-socket");
        host.plugin_disconnected(0, "new-launch", "old-socket");
        assert!(host.plugin_session_current(0, "new-launch", "new-socket"));
        assert!(!host.plugin_session_current(0, "old-launch", "new-socket"));
        assert!(!host.plugin_session_current(0, "new-launch", "old-socket"));
        host.plugin_disconnected(0, "new-launch", "new-socket");
        assert!(host.plugins[0].sender.is_none());
        assert_eq!(host.plugins[0].runtime.status, "disconnected");
    }
    #[test]
    fn failed_restart_preserves_settings_and_revokes_previous_connection() {
        let root = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(root.path());
        host.slots[0].settings = json!({"kept":17});
        host.globals
            .insert("test.plugin".into(), json!({"kept":23}));
        fs::write(root.path().join("manifest.json"), json!({"UUID":"test.plugin","Name":"Unavailable runtime","Version":"1.2.3","Nodejs":{"Version":"999"},"CodePath":"plugin.mjs","Actions":[{"UUID":"test.action"}]}).to_string()).unwrap();
        fs::write(root.path().join("plugin.mjs"), "").unwrap();
        let (sender, _receiver) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.plugins[0].runtime.session = Some("old".into());
        let old = host.plugins[0].registration.clone();
        assert!(host.restart_plugin("test.plugin").is_err());
        assert_eq!(host.slots[0].settings, json!({"kept":17}));
        assert_eq!(host.globals["test.plugin"], json!({"kept":23}));
        assert!(!host.plugin_session_current(0, &old, "old"));
        assert!(host.plugins[0].sender.is_none());
        assert_eq!(host.plugins[0].runtime.status, "failed");
        assert!(
            host.plugins[0]
                .runtime
                .error
                .as_ref()
                .unwrap()
                .contains("Node.js 999")
        );
        assert_eq!(host.plugins[0].snapshot()["version"], "1.2.3");
    }
    #[test]
    fn process_exit_is_reaped_once_and_unregistered_live_child_gets_a_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(root.path());
        let log = root.path().join("fixture.log");
        fs::write(&log, "fixture failure detail").unwrap();
        let mut child = Command::new("/bin/false").spawn().unwrap();
        child.wait().unwrap();
        host.plugins[0].child = Some(child);
        host.plugins[0].runtime = Runtime::starting();
        host.plugins[0].runtime.log_path = Some(log);
        host.monitor_children();
        assert!(host.plugins[0].child.is_none());
        assert_eq!(host.plugins[0].runtime.status, "failed");
        assert!(host.logs.last().unwrap().contains("fixture failure detail"));
        let count = host.logs.len();
        host.monitor_children();
        assert_eq!(host.logs.len(), count);
        host.plugins[0].child = Some(Command::new("/bin/sleep").arg("60").spawn().unwrap());
        host.plugins[0].runtime = Runtime::starting();
        host.plugins[0].runtime.started = Some(Instant::now() - Duration::from_secs(16));
        host.monitor_children();
        assert!(
            host.plugins[0]
                .runtime
                .error
                .as_ref()
                .unwrap()
                .contains("15 seconds")
        );
        assert!(host.plugins[0].child.is_some()); // No automatic restart or timeout kill.
    }
    #[test]
    fn failed_restart_cleans_owned_holds_and_inspectors_on_every_device() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(temp.path());
        let a = host.slots[0].context.clone();
        host.add_simulated_device("device-b", Model::Mini).unwrap();
        let b = host
            .with_device("device-b", |h| {
                h.assign("Keypad", 0, "test.action")?;
                Ok(h.slots[0].context.clone())
            })
            .unwrap();
        let (other_sender, mut other_events) = mpsc::unbounded_channel();
        host.plugins.push(Plugin {
            asset_generation: new_asset_generation(),
            id: "other.plugin".into(),
            name: "Other".into(),
            root: temp.path().join("other.sdPlugin"),
            manifest: json!({}),
            registration: "other-registration".into(),
            runtime: Runtime::default(),
            child: None,
            sender: Some(other_sender),
        });
        host.actions.push(Action {
            uuid: "other.action".into(),
            name: "Other".into(),
            controllers: vec!["Keypad".into()],
            plugin: "other.plugin".into(),
            raw: json!({"States":[{}]}),
        });
        let other = host
            .with_device("device-b", |h| {
                h.assign("Keypad", 1, "other.action")?;
                let context = h
                    .slots
                    .iter()
                    .find(|s| s.index == 1)
                    .unwrap()
                    .context
                    .clone();
                h.button_input(HeldControl::Key(1), true)?;
                Ok(context)
            })
            .unwrap();
        while other_events.try_recv().is_ok() {}
        let (sender, mut events) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        for (device, context, registration) in
            [("test-device", &a, "pi-a"), ("device-b", &b, "pi-b")]
        {
            host.with_device(device, |h| {
                let (tx, _rx) = mpsc::unbounded_channel();
                h.inspectors.insert(
                    registration.into(),
                    Inspector {
                        setup: None,
                        context: context.clone(),
                        plugin: 0,
                        child: None,
                        sender: Some(tx),
                    },
                );
                h.button_input(HeldControl::Key(0), true)
            })
            .unwrap();
        }
        while events.try_recv().is_ok() {}
        host.select_device("device-b").unwrap();
        assert!(host.restart_plugin("test.plugin").is_err()); // Missing manifest; no process launched.
        assert_eq!(host.device, "device-b");
        assert!(host.inspector_device("pi-a").is_none());
        assert!(host.inspector_device("pi-b").is_none());
        let events: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        for (device, context) in [("test-device", a), ("device-b", b)] {
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e["event"] == "keyUp"
                        && e["device"] == device
                        && e["context"] == context)
                    .count(),
                1
            );
            host.with_device(device, |h| {
                assert!(!h.held.contains_key(&HeldControl::Key(0)));
                if device == "device-b" {
                    assert_eq!(h.held.get(&HeldControl::Key(1)), Some(&other));
                }
                Ok(())
            })
            .unwrap();
        }
        assert!(other_events.try_recv().is_err());
        assert!(host.plugins[1].sender.is_some());
        assert!(host.plugins[0].sender.is_none());
        assert_eq!(host.plugins[0].runtime.status, "failed");
    }
}
