//! Per-plugin application subscriptions. Windows/macOS names require explicit
//! Linux executable mappings; no guessed suffix removal or process launching.
use super::*;
use crate::application_monitor::{Process, Snapshot};
use std::collections::BTreeSet;

pub(super) type Mappings = HashMap<String, BTreeMap<String, String>>;
pub(super) type Delivered = BTreeSet<(Process, String)>;

struct Application {
    name: String,
    platforms: Vec<&'static str>,
    executable: String,
}

fn valid_executable(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return false;
    }
    let path = Path::new(value);
    // Names are literal, including spaces. Relative paths and command arguments
    // have no shell interpretation and are not searched through PATH.
    path.file_name().is_some()
        && (path.is_absolute() || path.components().count() == 1)
        && !value.ends_with('/')
}

fn applications(plugin: &Plugin, overrides: Option<&BTreeMap<String, String>>) -> Vec<Application> {
    let mut result: Vec<Application> = Vec::new();
    for platform in ["windows", "mac", "linux"] {
        let Some(names) = plugin.manifest["ApplicationsToMonitor"][platform].as_array() else {
            continue;
        };
        for name in names.iter().take(256).filter_map(Value::as_str) {
            if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
                continue;
            }
            if let Some(existing) = result.iter_mut().find(|entry| entry.name == name) {
                if !existing.platforms.contains(&platform) {
                    existing.platforms.push(platform);
                }
                continue;
            }
            result.push(Application {
                name: name.into(),
                platforms: vec![platform],
                executable: String::new(),
            });
        }
    }
    for entry in &mut result {
        entry.executable = overrides
            .and_then(|values| values.get(&entry.name))
            .cloned()
            .unwrap_or_else(|| {
                if entry.platforms.contains(&"linux") {
                    entry.name.clone()
                } else {
                    String::new()
                }
            });
        if !valid_executable(&entry.executable) {
            entry.executable.clear();
        }
    }
    result
}

fn matches(process: &Process, executable: &str) -> bool {
    if executable.is_empty() {
        return false;
    }
    let path = Path::new(executable);
    if path.is_absolute() {
        process.executable == path
    } else {
        process.executable.file_name() == path.file_name()
    }
}

impl Host {
    pub(super) fn plugin_snapshot(&self, plugin: &Plugin) -> Value {
        let mut value = plugin.snapshot();
        let presentation = self.presented_manifest(plugin);
        value["name"] = json!(presentation["Name"].as_str().unwrap_or(&plugin.name));
        value["description"] = presentation["Description"].clone();
        value["localization_warning"] = json!(
            self.localizations
                .get(&plugin.id)
                .and_then(|p| p.warning.as_deref())
        );
        let mappings = self.application_mappings.get(&plugin.id);
        value["application_mappings"] = json!(mappings.cloned().unwrap_or_default());
        value["applications"] = json!(applications(plugin, mappings)
            .into_iter()
            .map(|entry| json!({"application":entry.name,"platforms":entry.platforms,"executable":entry.executable}))
            .collect::<Vec<_>>());
        value
    }

    pub(super) fn save_application_mappings(&mut self, request: &Value) -> Result<()> {
        let id = request["id"].as_str().context("Plugin id missing")?;
        let plugin = self
            .plugins
            .iter()
            .find(|p| p.id == id)
            .context("Plugin not installed")?;
        let previous = self
            .application_mappings
            .get(id)
            .cloned()
            .unwrap_or_default();
        let baseline: BTreeMap<String, String> =
            serde_json::from_value(request["baseline"].clone())
                .context("Application mapping baseline missing")?;
        anyhow::ensure!(
            baseline == previous,
            "Application mappings changed; reload the current settings before saving"
        );
        let mappings: BTreeMap<String, String> =
            serde_json::from_value(request["mappings"].clone())
                .context("Application mappings must be an object of executable names or paths")?;
        let declared = applications(plugin, None);
        anyhow::ensure!(mappings.len() <= 768, "Too many application mappings");
        for (name, executable) in &mappings {
            // Preserve old entries after a manifest update, but do not allow a
            // plugin to subscribe to identifiers absent from its own manifest.
            anyhow::ensure!(
                declared.iter().any(|entry| entry.name == *name)
                    || previous.get(name) == Some(executable),
                "Application is not declared by this plugin"
            );
            anyhow::ensure!(
                valid_executable(executable),
                "Use an executable name or absolute path without control characters"
            );
        }
        let old = self.application_mappings.insert(id.into(), mappings);
        if let Err(error) = self.persist() {
            match old {
                Some(old) => {
                    self.application_mappings.insert(id.into(), old);
                }
                None => {
                    self.application_mappings.remove(id);
                }
            }
            return Err(error);
        }
        self.poll_applications();
        Ok(())
    }

    pub(super) fn poll_applications(&mut self) {
        if !self.running {
            return;
        }
        let needed = self.plugins.iter().any(|p| {
            p.sender.is_some()
                && applications(p, self.application_mappings.get(&p.id))
                    .iter()
                    .any(|a| !a.executable.is_empty())
        });
        if !needed {
            self.application_monitor = None;
            // Removing the last subscription still balances its delivered state.
            self.dispatch_applications(&Snapshot {
                processes: BTreeSet::new(),
                error: None,
            });
            self.application_error = None;
            return;
        }
        let worker = self
            .application_monitor
            .get_or_insert_with(crate::application_monitor::Worker::new);
        if let Some(snapshot) = worker.snapshot() {
            self.apply_application_snapshot(&snapshot);
        }
    }

    fn apply_application_snapshot(&mut self, snapshot: &Snapshot) {
        if snapshot.error != self.application_error {
            if let Some(error) = &snapshot.error {
                self.log(format!("Application monitor: {error}"));
            }
            self.application_error = snapshot.error.clone();
        }
        // The observer retains prior identities when they are unreadable,
        // and the whole previous set after a failed scan. Other readable
        // processes can still transition during a partial inventory.
        self.dispatch_applications(snapshot);
    }

    fn dispatch_applications(&mut self, snapshot: &Snapshot) {
        for plugin in &self.plugins {
            let Some(sender) = &plugin.sender else {
                self.application_delivered.remove(&plugin.id);
                continue;
            };
            let subscriptions = applications(plugin, self.application_mappings.get(&plugin.id));
            let current: Delivered = snapshot
                .processes
                .iter()
                .filter_map(|process| {
                    // The native watcher invokes only the first matching filter per
                    // process. Preserve manifest order if explicit mappings overlap.
                    subscriptions
                        .iter()
                        .find(|a| matches(process, &a.executable))
                        .map(|a| (process.clone(), a.name.clone()))
                })
                .collect();
            let previous = self
                .application_delivered
                .entry(plugin.id.clone())
                .or_default();
            for (_, application) in previous.difference(&current) {
                let _ = sender.send(json!({"event":"applicationDidTerminate","payload":{"application":application}}));
            }
            for (_, application) in current.difference(previous) {
                let _ = sender.send(
                    json!({"event":"applicationDidLaunch","payload":{"application":application}}),
                );
            }
            *previous = current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process(pid: u32, started: u64, path: &str) -> Process {
        Process {
            pid,
            started,
            executable: path.into(),
        }
    }
    fn view(processes: Vec<Process>) -> Snapshot {
        Snapshot {
            processes: processes.into_iter().collect(),
            error: None,
        }
    }
    fn receive(receiver: &mut mpsc::UnboundedReceiver<Value>) -> Vec<Value> {
        std::iter::from_fn(|| receiver.try_recv().ok()).collect()
    }

    #[test]
    fn explicit_mappings_are_case_exact_and_never_strip_windows_suffixes() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.plugins[0].manifest = json!({"ApplicationsToMonitor":{"windows":["Game.exe"],"mac":["com.example.game"],"linux":["native"]}});
        let entries = applications(&host.plugins[0], None);
        assert_eq!(
            entries
                .iter()
                .map(|e| e.executable.as_str())
                .collect::<Vec<_>>(),
            vec!["", "", "native"]
        );
        assert!(matches(&process(1, 1, "/usr/bin/native"), "native"));
        assert!(!matches(&process(1, 1, "/usr/bin/native"), "Native"));
        assert!(!matches(&process(1, 1, "/usr/bin/Game"), "Game.exe"));
        assert!(!matches(
            &process(1, 1, "/opt/game/native"),
            "/usr/bin/native"
        ));
        for invalid in [
            "./native",
            "relative/native",
            "../native",
            "native\n",
            "/usr/bin/",
        ] {
            assert!(!valid_executable(invalid), "{invalid}");
        }
        assert!(valid_executable("/opt/My App/game"));
    }

    #[test]
    fn process_instances_initial_replay_exit_pid_reuse_and_session_replay() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let id = host.plugins[0].id.clone();
        host.plugins[0].manifest =
            json!({"ApplicationsToMonitor":{"windows":["Game.exe","Other.exe"]}});
        host.application_mappings.insert(
            id.clone(),
            BTreeMap::from([
                ("Game.exe".into(), "game".into()),
                ("Other.exe".into(), "game".into()),
            ]),
        );
        let (sender, mut received) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        let first = process(1, 10, "/bin/game");
        let second = process(2, 20, "/bin/game");
        let both = view(vec![first.clone(), second.clone()]);
        host.dispatch_applications(&both);
        let launch = json!({"event":"applicationDidLaunch","payload":{"application":"Game.exe"}});
        let exit = json!({"event":"applicationDidTerminate","payload":{"application":"Game.exe"}});
        assert_eq!(receive(&mut received), vec![launch.clone(), launch.clone()]);
        host.dispatch_applications(&both);
        assert!(receive(&mut received).is_empty());
        host.dispatch_applications(&view(vec![second.clone()]));
        assert_eq!(receive(&mut received), vec![exit.clone()]);
        host.dispatch_applications(&view(vec![process(2, 30, "/bin/game")]));
        assert_eq!(receive(&mut received), vec![exit, launch.clone()]);
        host.application_delivered.remove(&id);
        host.dispatch_applications(&both);
        assert_eq!(receive(&mut received), vec![launch.clone(), launch]);
    }

    #[test]
    fn mapping_validation_stale_updates_and_failed_save_preserve_previous_config() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let id = host.plugins[0].id.clone();
        host.plugins[0].manifest = json!({"ApplicationsToMonitor":{"windows":["Game.exe"]}});
        let request = json!({"id":id,"baseline":{},"mappings":{"Game.exe":"game"}});
        host.save_application_mappings(&request).unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.application_mappings[&id]["Game.exe"], "game");
        assert!(host.save_application_mappings(&request).is_err());
        assert!(host.save_application_mappings(&json!({"id":id,"baseline":{"Game.exe":"game"},"mappings":{"Undeclared.exe":"other"}})).is_err());
        host.state_path = root.path().join("absent/state.json");
        assert!(
            host.save_application_mappings(
                &json!({"id":id,"baseline":{"Game.exe":"game"},"mappings":{"Game.exe":"other"}})
            )
            .is_err()
        );
        assert_eq!(host.application_mappings[&id]["Game.exe"], "game");
    }

    #[test]
    fn partial_scan_warning_does_not_block_other_process_transitions() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.plugins[0].manifest = json!({"ApplicationsToMonitor":{"linux":["game"]}});
        let (sender, mut received) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        let retained = process(42, 10, "/bin/game");
        host.apply_application_snapshot(&view(vec![retained.clone()]));
        receive(&mut received);
        host.apply_application_snapshot(&Snapshot {
            processes: [retained.clone(), process(43, 20, "/bin/game")]
                .into_iter()
                .collect(),
            error: Some("PID42 temporarily unreadable; previous identity retained".into()),
        });
        assert_eq!(
            receive(&mut received),
            vec![json!({"event":"applicationDidLaunch","payload":{"application":"game"}})]
        );
        host.apply_application_snapshot(&Snapshot {
            processes: [retained].into_iter().collect(),
            error: Some("PID42 temporarily unreadable; previous identity retained".into()),
        });
        assert_eq!(
            receive(&mut received),
            vec![json!({"event":"applicationDidTerminate","payload":{"application":"game"}})]
        );
    }
}
