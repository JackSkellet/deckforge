//! Explicit Linux application bindings; foreign AppIdentifier metadata is retained.
use super::*;
use crate::focus_monitor::{Focus, Snapshot, Worker};
use anyhow::ensure;
use std::sync::atomic::{AtomicU8, Ordering};
pub(super) mod archive;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    DesktopId,
    Executable,
    WindowClass,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> (Host, String, String) {
        let mut host = super::super::tests::fixture(root);
        let fallback = host.library.active_profile.clone();
        host.library_command(&json!({"command":"profile_create","name":"Editor"}))
            .unwrap();
        let editor = host.library.active_profile.clone();
        host.select_profile(&fallback, None).unwrap();
        (host, fallback, editor)
    }
    fn frame(class: &str, pid: u32) -> Snapshot {
        Snapshot {
            revision: 1,
            editor_minimized: None,
            focus: Some(Focus {
                pid,
                desktop_id: String::new(),
                window_class: class.into(),
                executable: None,
            }),
            recent: vec![],
            error: None,
        }
    }
    fn configure(host: &mut Host, fallback: &str, editor: &str) -> Result<()> {
        let expected = serde_json::to_value(host.smart_config())?;
        host.smart_command(&json!({"command":"smart_config","expected":expected,"settings":{
            "enabled":true,"fallback":fallback,"bindings":{editor:{"kind":"window_class","value":"Editor","name":"Editor app"}}
        }}))
    }
    #[test]
    fn focus_routes_once_respects_editing_and_preserves_manual_selection_until_app_change() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        configure(&mut host, &base, &editor).unwrap();
        host.smart.editor_state.store(1, Ordering::Release);
        host.route_smart_profiles(&frame("Editor", 100));
        assert_eq!(host.library.active_profile, base);
        host.smart.editor_state.store(0, Ordering::Release);
        host.route_smart_profiles(&frame("Editor", 100));
        assert_eq!(host.library.active_profile, editor);
        host.select_profile(&base, None).unwrap();
        host.route_smart_profiles(&frame("Editor", 101));
        assert_eq!(host.library.active_profile, base);
        host.route_smart_profiles(&frame("Other", 102));
        host.route_smart_profiles(&frame("Editor", 103));
        assert_eq!(host.library.active_profile, editor);
        let mut unavailable = frame("Other", 104);
        unavailable.error = Some("Disconnected".into());
        host.route_smart_profiles(&unavailable);
        assert_eq!(host.library.active_profile, editor);
        host.route_smart_profiles(&frame("Other", 104));
        assert_eq!(host.library.active_profile, base);
    }
    #[test]
    fn compositor_minimize_state_can_override_stale_toolkit_state_without_reapplying_manual_choice()
    {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        configure(&mut host, &base, &editor).unwrap();
        host.smart.editor_state.store(1, Ordering::Release);
        let mut observed = frame("Editor", 100);
        observed.editor_minimized = Some(false);
        host.route_smart_profiles(&observed);
        assert_eq!(host.library.active_profile, base);
        observed.editor_minimized = Some(true);
        host.route_smart_profiles(&observed);
        assert_eq!(host.library.active_profile, editor);
        host.select_profile(&base, None).unwrap();
        host.route_smart_profiles(&observed);
        assert_eq!(host.library.active_profile, base);
        host.smart.editor_state.store(2, Ordering::Release);
        observed.editor_minimized = Some(false);
        host.route_smart_profiles(&observed);
        assert_eq!(host.library.active_profile, base);
    }
    #[test]
    fn rules_are_device_scoped_and_switching_preserves_page_cursors() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        host.select_profile(&editor, None).unwrap();
        host.library_command(&json!({"command":"page_create","name":"Second"}))
            .unwrap();
        let page = host.library.active_page.clone();
        host.select_profile(&base, None).unwrap();
        configure(&mut host, &base, &editor).unwrap();
        host.add_simulated_device("B", Model::Mini).unwrap();
        let b = host
            .with_device("B", |host| Ok(host.library.active_profile.clone()))
            .unwrap();
        host.route_smart_profiles(&frame("Editor", 10));
        assert_eq!(host.library.active_profile, editor);
        assert_eq!(host.library.active_page, page);
        assert_eq!(
            host.with_device("B", |host| Ok(host.library.active_profile.clone()))
                .unwrap(),
            b
        );
    }
    #[test]
    fn duplicate_matches_use_sidebar_order_and_foreign_or_stale_config_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        configure(&mut host, &base, &editor).unwrap();
        let expected = serde_json::to_value(host.smart_config()).unwrap();
        let mut settings = expected.clone();
        settings["bindings"][&base] = settings["bindings"][&editor].clone();
        host.smart_command(
            &json!({"command":"smart_config","expected":expected,"settings":settings}),
        )
        .unwrap();
        host.select_profile(&editor, None).unwrap();
        host.route_smart_profiles(&frame("Editor", 42));
        assert_eq!(host.library.active_profile, base);
        host.add_simulated_device("other", Model::Plus).unwrap();
        let foreign = host
            .with_device("other", |host| Ok(host.library.active_profile.clone()))
            .unwrap();
        let expected = serde_json::to_value(host.smart_config()).unwrap();
        let mut settings = expected.clone();
        settings["fallback"] = json!(foreign);
        assert!(
            host.smart_command(
                &json!({"command":"smart_config","expected":expected,"settings":settings})
            )
            .is_err()
        );
        assert!(
            host.smart_command(
                &json!({"command":"smart_config","expected":{},"settings":expected})
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(host.smart_config()).unwrap(), expected);
    }
    #[test]
    fn inactive_layout_bindings_are_preserved_but_never_selected_on_the_wrong_model() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        configure(&mut host, &base, &editor).unwrap();
        host.library
            .profiles
            .iter_mut()
            .find(|p| p.id == editor)
            .unwrap()
            .model = Model::Mini;
        let config = host.smart_config();
        assert!(config.bindings.contains_key(&editor));
        host.route_smart_profiles(&frame("Editor", 10));
        assert_eq!(host.library.active_profile, base);
        host.smart_command(&json!({"command":"smart_config","expected":config,"settings":config}))
            .unwrap();
        assert!(
            host.smart_profiles[&host.scope]
                .bindings
                .contains_key(&editor)
        );
    }
    #[test]
    fn duplicated_profile_has_an_independent_binding_and_keeps_device_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        configure(&mut host, &base, &editor).unwrap();
        host.library
            .profiles
            .iter_mut()
            .find(|p| p.id == editor)
            .unwrap()
            .raw["AppIdentifier"] = json!("C:\\Original.exe");
        let before = host.smart_config();
        host.library_command(
            &json!({"command":"profile_duplicate","id":editor,"name":"Editor copy"}),
        )
        .unwrap();
        let copied = host.library.active_profile.clone();
        assert_ne!(copied, editor);
        let config = host.smart_config();
        assert_eq!(config.enabled, before.enabled);
        assert_eq!(config.fallback, before.fallback);
        assert_eq!(config.bindings[&copied], before.bindings[&editor]);
        assert_eq!(config.bindings[&editor], before.bindings[&editor]);
        assert_eq!(
            host.library.current().unwrap().raw["AppIdentifier"],
            "C:\\Original.exe"
        );
        let mut changed = config.clone();
        changed.bindings.get_mut(&copied).unwrap().value = "Other app".into();
        host.smart_command(&json!({"command":"smart_config","expected":config,"settings":changed}))
            .unwrap();
        assert_eq!(host.smart_config().bindings[&editor].value, "Editor");
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(
            saved.smart_profiles[&host.scope].bindings[&copied].value,
            "Other app"
        );
    }
    #[test]
    fn cross_device_copy_keeps_destination_preferences_and_manual_activation() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        configure(&mut host, &base, &editor).unwrap();
        let source = host.smart_config();
        let scope = host.scope.clone();
        host.add_simulated_device("B", Model::Plus).unwrap();
        let destination = host
            .with_device("B", |host| {
                let base = host.library.active_profile.clone();
                let expected = host.smart_config();
                let config = Config {
                    enabled: true,
                    fallback: Some(base.clone()),
                    bindings: BTreeMap::from([(
                        base,
                        Binding {
                            kind: Kind::WindowClass,
                            value: "Existing app".into(),
                            name: "Existing".into(),
                        },
                    )]),
                };
                host.smart_command(
                    &json!({"command":"smart_config","expected":expected,"settings":config}),
                )?;
                Ok(config)
            })
            .unwrap();
        host.route_smart_profiles(&frame("Other", 99));
        host.copy_profile_to_device(&json!({"profile":editor,"target_device":"B","name":"Copied"}))
            .unwrap();
        assert_eq!(host.scope, scope);
        assert_eq!(host.library.active_profile, base);
        assert_eq!(host.smart_config(), source);
        let copy = host
            .with_device("B", |host| {
                let id = host.library.active_profile.clone();
                let config = host.smart_config();
                assert!(config.enabled);
                assert_eq!(config.fallback, destination.fallback);
                for (id, binding) in &destination.bindings {
                    assert_eq!(config.bindings[id], *binding);
                }
                assert_eq!(config.bindings[&id], source.bindings[&editor]);
                Ok(id)
            })
            .unwrap();
        host.route_smart_profiles(&frame("Other", 100));
        assert_eq!(
            host.with_device("B", |host| Ok(host.library.active_profile.clone()))
                .unwrap(),
            copy
        );
        host.add_simulated_device("C", Model::Plus).unwrap();
        host.copy_profile_to_device(
            &json!({"profile":editor,"target_device":"C","name":"Inactive copy"}),
        )
        .unwrap();
        host.with_device("C", |host| {
            assert!(!host.smart_config().enabled);
            assert!(host.smart_config().fallback.is_none());
            assert_eq!(host.smart_config().bindings.len(), 1);
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn failed_copy_rolls_back_profile_rules_navigation_and_preserves_held_actions() {
        for cross_device in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (mut host, base, editor) = fixture(dir.path());
            configure(&mut host, &base, &editor).unwrap();
            host.add_simulated_device("B", Model::Plus).unwrap();
            let device = if cross_device {
                "B".to_owned()
            } else {
                host.device.clone()
            };
            let context = host
                .with_device(&device, |host| {
                    host.assign("Keypad", 0, "test.action")?;
                    Ok(host.slots[0].context.clone())
                })
                .unwrap();
            let (tx, mut rx) = mpsc::unbounded_channel();
            host.plugins[0].sender = Some(tx);
            host.with_device(&device, |host| {
                host.input(InputEvent::Key {
                    index: 0,
                    pressed: true,
                })
            })
            .unwrap();
            while rx.try_recv().is_ok() {}
            host.sync_all_devices();
            let before = serde_json::to_value(&host.library).unwrap();
            let rules = host.smart_profiles.clone();
            let records = serde_json::to_value(host.device_records()).unwrap();
            host.state_path = dir.path().join("directory");
            fs::create_dir(&host.state_path).unwrap();
            let result = if cross_device {
                host.copy_profile_to_device(
                    &json!({"profile":editor,"target_device":"B","name":"Failed"}),
                )
            } else {
                host.library_command(
                    &json!({"command":"profile_duplicate","id":editor,"name":"Failed"}),
                )
            };
            assert!(result.is_err());
            assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
            assert_eq!(host.smart_profiles, rules);
            assert_eq!(
                serde_json::to_value(host.device_records()).unwrap(),
                records
            );
            host.with_device(&device, |host| {
                assert_eq!(host.held.get(&HeldControl::Key(0)), Some(&context));
                Ok(())
            })
            .unwrap();
            assert!(rx.try_recv().is_err());
        }
    }
    #[test]
    fn configuration_and_profile_switch_failures_do_not_publish_partial_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        let state = host.state_path.clone();
        host.state_path = dir.path().join("directory");
        fs::create_dir(&host.state_path).unwrap();
        assert!(configure(&mut host, &base, &editor).is_err());
        assert!(host.smart_profiles.is_empty());
        host.state_path = state;
        configure(&mut host, &base, &editor).unwrap();
        let before = serde_json::to_value(&host.library).unwrap();
        host.state_path = dir.path().join("directory");
        host.route_smart_profiles(&frame("Editor", 10));
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert!(host.smart.routing_errors.contains_key(&host.scope));
    }
    #[test]
    fn imported_default_marker_is_suggested_without_rewriting_foreign_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let (mut host, base, editor) = fixture(dir.path());
        host.library
            .profiles
            .iter_mut()
            .find(|p| p.id == base)
            .unwrap()
            .raw["AppIdentifier"] = json!("*");
        host.library
            .profiles
            .iter_mut()
            .find(|p| p.id == editor)
            .unwrap()
            .raw["AppIdentifier"] = json!("C:\\Example.exe");
        assert_eq!(host.smart_config().fallback, Some(base.clone()));
        configure(&mut host, &base, &editor).unwrap();
        assert_eq!(
            host.library
                .profiles
                .iter()
                .find(|p| p.id == editor)
                .unwrap()
                .raw["AppIdentifier"],
            "C:\\Example.exe"
        );
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.smart_profiles[&host.scope].enabled);
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub kind: Kind,
    pub value: String,
    #[serde(default)]
    pub name: String,
}
impl Binding {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.value.is_empty()
                && self.value.len() <= 4096
                && !self.value.chars().any(char::is_control),
            "Application identity must be nonempty text"
        );
        ensure!(
            self.name.chars().count() <= 128 && !self.name.chars().any(char::is_control),
            "Application label is too long or invalid"
        );
        if self.kind == Kind::Executable {
            ensure!(
                Path::new(&self.value).is_absolute(),
                "Executable matching requires an absolute path, without arguments"
            );
        } else {
            ensure!(
                self.value.len() <= 1024,
                "Application identifier is too long"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Config {
    pub enabled: bool,
    pub fallback: Option<String>,
    pub bindings: BTreeMap<String, Binding>,
}
pub(super) type Configs = HashMap<String, Config>;
#[derive(Default)]
pub(super) struct Runtime {
    observer: Option<Worker>,
    retired: Vec<Worker>,
    // 0 hidden, 1 visible, 2 toolkit-minimized, 3 quitting. KWin's observed
    // minimized state overrides the toolkit hint while a window exists.
    pub editor_state: Arc<AtomicU8>,
    was_visible: bool,
    preview: bool,
    last: HashMap<String, (Option<Focus>, Model)>,
    latest: Option<Snapshot>,
    error: Option<String>,
    routing_errors: HashMap<String, String>,
}
impl Runtime {
    fn editor_paused(&self) -> bool {
        match self.editor_state.load(Ordering::Acquire) {
            0 => false,
            3 => true,
            state => !self
                .latest
                .as_ref()
                .and_then(|s| s.editor_minimized)
                .unwrap_or(state == 2),
        }
    }
}
fn matches(binding: &Binding, focus: &Focus) -> bool {
    match binding.kind {
        Kind::DesktopId => !focus.desktop_id.is_empty() && focus.desktop_id == binding.value,
        Kind::WindowClass => !focus.window_class.is_empty() && focus.window_class == binding.value,
        Kind::Executable => focus
            .executable
            .as_ref()
            .is_some_and(|p| p == Path::new(&binding.value)),
    }
}
fn same(a: &Option<Focus>, b: &Option<Focus>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.same_application(b),
        (None, None) => true,
        _ => false,
    }
}
impl Controller {
    pub fn smart_visibility(&self) -> Arc<AtomicU8> {
        self.shared.lock().unwrap().smart.editor_state.clone()
    }
    pub fn smart_editor_visible(&self, visible: bool) {
        self.smart_visibility()
            .store(u8::from(visible), Ordering::Release);
    }
}
impl Host {
    pub(super) fn smart_config(&self) -> Config {
        let mut config = self
            .smart_profiles
            .get(&self.scope)
            .cloned()
            .unwrap_or_else(|| {
                let defaults: Vec<_> = self
                    .library
                    .profiles
                    .iter()
                    .filter(|p| {
                        p.device == self.device
                            && p.model.profile_compatible(self.model)
                            && p.raw["AppIdentifier"] == "*"
                    })
                    .collect();
                Config {
                    fallback: if defaults.len() == 1 {
                        Some(defaults[0].id.clone())
                    } else {
                        None
                    },
                    ..Default::default()
                }
            });
        let available: HashSet<_> = self
            .library
            .profiles
            .iter()
            .filter(|p| p.device == self.device)
            .map(|p| p.id.as_str())
            .collect();
        config
            .bindings
            .retain(|id, _| available.contains(id.as_str()));
        config.fallback = config.fallback.filter(|id| available.contains(id.as_str()));
        config
    }
    pub(super) fn smart_snapshot(&self) -> Value {
        let recent: Vec<_> = self
            .smart
            .latest
            .as_ref()
            .map(|s| {
                s.recent
                    .iter()
                    .filter(|f| f.pid != std::process::id())
                    .collect()
            })
            .unwrap_or_default();
        json!({"config":self.smart_config(),"backend":"KDE KWin",
            "observing":self.smart.observer.is_some(),"paused":self.smart.editor_paused()||!self.inspectors.is_empty(),
            "focus":self.smart.latest.as_ref().and_then(|s|s.focus.as_ref()),
            "recent":recent,
            "error":self.smart.routing_errors.get(&self.scope).or(self.smart.error.as_ref()).or_else(||self.smart.latest.as_ref().and_then(|s|s.error.as_ref()))})
    }
    pub(super) fn smart_command(&mut self, request: &Value) -> Result<()> {
        if request["command"] == "smart_preview" {
            self.smart.preview = request["enabled"]
                .as_bool()
                .context("Preview flag missing")?;
            return Ok(());
        }
        let current = self.smart_config();
        ensure!(
            request.get("expected") == Some(&serde_json::to_value(&current)?),
            "Smart Profile settings changed; reopen the dialog"
        );
        let config: Config = serde_json::from_value(request["settings"].clone())?;
        ensure!(
            config.bindings.len() <= 512,
            "Too many Smart Profile bindings"
        );
        for id in config.bindings.keys().chain(config.fallback.iter()) {
            ensure!(
                self.library
                    .profiles
                    .iter()
                    .any(|p| p.id == *id && p.device == self.device),
                "Smart Profile target is missing or belongs to another device"
            );
        }
        for binding in config.bindings.values() {
            binding.validate()?;
        }
        let old = self.smart_profiles.insert(self.scope.clone(), config);
        if let Err(error) = self.persist() {
            if let Some(old) = old {
                self.smart_profiles.insert(self.scope.clone(), old);
            } else {
                self.smart_profiles.remove(&self.scope);
            }
            return Err(error);
        }
        self.invalidate_smart_scope(&self.scope.clone());
        Ok(())
    }
    pub(super) fn invalidate_smart_scope(&mut self, scope: &str) {
        self.smart.last.remove(scope);
        self.smart.routing_errors.remove(scope);
        self.smart.error = None;
    }
    pub(super) fn poll_smart_profiles(&mut self) {
        let scopes: HashSet<_> = std::iter::once(self.scope.clone())
            .chain(self.other_devices.keys().cloned())
            .collect();
        self.smart.last.retain(|scope, _| scopes.contains(scope));
        self.smart
            .routing_errors
            .retain(|scope, _| scopes.contains(scope));
        if self.smart.editor_state.load(Ordering::Acquire) == 0 {
            self.smart.preview = false;
        }
        self.smart.retired.retain(|worker| !worker.finished());
        let needed = self.smart.preview
            || self
                .smart_profiles
                .iter()
                .any(|(scope, c)| scopes.contains(scope) && c.enabled);
        if !needed {
            if let Some(mut worker) = self.smart.observer.take() {
                worker.request_stop();
                self.smart.retired.push(worker);
            }
            self.smart.last.clear();
            return;
        }
        if self.smart.observer.is_none() && self.smart.retired.is_empty() {
            match Worker::for_state(self.state_path.parent().unwrap_or(Path::new("."))) {
                Ok(worker) => {
                    self.smart.observer = Some(worker);
                    self.smart.error = None;
                }
                Err(error) => {
                    self.smart.error = Some(format!("Focus observer: {error}"));
                    return;
                }
            }
        }
        let Some(worker) = &self.smart.observer else {
            return;
        };
        let observed = worker.snapshot();
        self.route_smart_profiles(&observed);
    }
    pub(super) fn route_smart_profiles(&mut self, observed: &Snapshot) {
        if let Some(error) = &observed.error
            && error != "Starting KDE focus observer"
            && self.smart.latest.as_ref().and_then(|s| s.error.as_ref()) != Some(error)
        {
            self.log(error.clone());
        }
        self.smart.latest = Some(observed.clone());
        let visible = self.smart.editor_paused();
        if self.smart.was_visible != visible {
            self.smart.last.clear();
            self.smart.was_visible = visible;
        }
        if self.smart.editor_paused()
            || !self.inspectors.is_empty()
            || observed.error.is_some()
            || observed
                .focus
                .as_ref()
                .is_some_and(|f| f.pid == std::process::id())
        {
            self.smart.last.clear();
            return;
        }
        for device in self.device_ids() {
            let scope = self.device_scope(&device).unwrap_or_else(|| device.clone());
            let result = self.with_device(&device, |host| {
                let config = host.smart_config();
                if !config.enabled {
                    host.smart.last.remove(&host.scope);
                    return Ok(());
                }
                if host
                    .smart
                    .last
                    .get(&host.scope)
                    .is_some_and(|(old, model)| *model == host.model && same(old, &observed.focus))
                {
                    return Ok(());
                }
                host.smart.routing_errors.remove(&host.scope);
                host.smart
                    .last
                    .insert(host.scope.clone(), (observed.focus.clone(), host.model));
                let available: Vec<_> = host
                    .library
                    .profiles
                    .iter()
                    .filter(|p| p.device == host.device && p.model.profile_compatible(host.model))
                    .collect();
                let target = observed
                    .focus
                    .as_ref()
                    .and_then(|focus| {
                        available.iter().find(|p| {
                            config
                                .bindings
                                .get(&p.id)
                                .is_some_and(|b| matches(b, focus))
                        })
                    })
                    .map(|p| p.id.clone())
                    .or_else(|| {
                        config
                            .fallback
                            .filter(|id| available.iter().any(|p| p.id == *id))
                    });
                if let Some(target) = target
                    && target != host.library.active_profile
                {
                    host.smart_select_profile(&target)?;
                    host.log(format!(
                        "Smart Profile selected: {}",
                        host.library.current()?.name
                    ));
                }
                Ok(())
            });
            if let Err(error) = result {
                let message = format!("Smart Profile: {error}");
                if self.smart.routing_errors.get(&scope) != Some(&message) {
                    self.log(message.clone());
                }
                self.smart.routing_errors.insert(scope, message);
            }
        }
    }
    fn smart_select_profile(&mut self, id: &str) -> Result<()> {
        self.sync_all_devices();
        let target = self
            .library
            .profiles
            .iter()
            .find(|p| p.id == id)
            .context("Profile disappeared")?;
        ensure!(
            target.device == self.device && target.model.profile_compatible(self.model),
            "Smart Profile belongs to another device/model"
        );
        let mut next = self.library.clone();
        next.previous = Some((next.active_profile.clone(), next.active_page.clone()));
        next.active_profile = id.into();
        next.active_page = next
            .cursors
            .get(id)
            .filter(|id| target.pages.iter().any(|p| p.id == **id))
            .cloned()
            .unwrap_or_else(|| target.page_order[0].clone());
        next.folders = next.folder_paths.get(id).cloned().unwrap_or_default();
        self.commit_profile_change(next, None)
    }
    pub(super) fn stop_smart_profiles(&mut self) -> Vec<Worker> {
        if let Some(mut worker) = self.smart.observer.take() {
            worker.request_stop();
            self.smart.retired.push(worker);
        }
        std::mem::take(&mut self.smart.retired)
    }
}
