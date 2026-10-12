//! Review staged packages before replacing a managed plugin's files/runtime.
use super::*;
use crate::plugin_update::{Backup, Prepared};
use sha2::{Digest, Sha256};
use std::time::Instant;

pub(super) struct Previous {
    pub backup: Backup,
    pub version: String,
    pub id: String,
}
pub(super) struct Pending {
    id: String,
    replacement_id: String,
    root: PathBuf,
    launch: String,
    enabled: bool,
    created: Instant,
    source: Source,
    manifest_hash: [u8; 32],
}
enum Source {
    Package(Prepared),
    Rollback(Backup),
}
impl Source {
    fn path(&self) -> &Path {
        match self {
            Self::Package(p) => p.path(),
            Self::Rollback(b) => b.path(),
        }
    }
}
fn version(manifest: &Value) -> String {
    manifest["Version"].as_str().unwrap_or("Unknown").to_owned()
}
fn comparison(old: &str, new: &str) -> &'static str {
    fn parts(value: &str) -> Option<[u32; 4]> {
        let fields: Vec<_> = value.split('.').collect();
        if fields.is_empty() || fields.len() > 4 {
            return None;
        }
        let mut result = [0; 4];
        for (index, field) in fields.iter().enumerate() {
            result[index] = field.parse().ok()?;
        }
        Some(result)
    }
    match (parts(old), parts(new)) {
        (Some(a), Some(b)) if b > a => "upgrade",
        (Some(a), Some(b)) if b < a => "downgrade",
        (Some(_), Some(_)) => "same",
        _ => "unknown",
    }
}
fn runnable(plugin: &Plugin) -> Result<()> {
    let entry = crate::plugin_entry::resolve(&plugin.root, &plugin.manifest)?;
    if entry.kind == crate::plugin_entry::Kind::Node {
        crate::plugin_entry::node_runtime(
            plugin.manifest["Nodejs"]["Version"]
                .as_str()
                .context("Nodejs.Version missing")?,
        )?;
    }
    let mut layouts = HashSet::new();
    for action in plugin.manifest["Actions"].as_array().into_iter().flatten() {
        if action["Controllers"]
            .as_array()
            .is_some_and(|controllers| controllers.iter().any(|controller| controller == "Encoder"))
        {
            let layout = action["Encoder"]["layout"].as_str().unwrap_or("$B1");
            if layouts.insert(layout) {
                display::load_layout(Some(&plugin.root), layout)
                    .with_context(|| format!("Replacement dial layout is invalid: {layout}"))?;
            }
        }
    }
    Ok(())
}
impl Controller {
    pub fn prepare_plugin_update(&self, request: Value, rollback: bool) -> Result<Value> {
        let expired = self.shared.lock().unwrap().take_expired_updates();
        drop(expired);
        let id = request["id"]
            .as_str()
            .context("Plugin id missing")?
            .to_owned();
        let (
            root,
            launch,
            enabled,
            current_version,
            state,
            reference,
            old_actions,
            backup,
            replacement_id,
        ) = {
            let host = self.shared.lock().unwrap();
            anyhow::ensure!(host.running, "Host stopped");
            anyhow::ensure!(
                host.pending_updates.len() < 4,
                "Finish or cancel an existing plugin update first"
            );
            let plugin = host
                .plugins
                .iter()
                .find(|p| p.id == id)
                .context("Plugin is no longer installed")?;
            anyhow::ensure!(
                request["expected_path"].as_str().map(Path::new) == Some(plugin.root.as_path())
                    && request.get("expected_version") == Some(&plugin.manifest["Version"]),
                "Plugin changed; review its current version again"
            );
            anyhow::ensure!(
                host.plugin_removal_mode(plugin) == removal::Mode::Archive,
                "Only managed plugin folders can be updated. Edit an external folder and use Restart."
            );
            (
                plugin.root.clone(),
                plugin.registration.clone(),
                host.plugin_enabled(plugin),
                version(&plugin.manifest),
                host.state_path
                    .parent()
                    .context("State directory missing")?
                    .to_owned(),
                host.reference_app.clone(),
                host.actions
                    .iter()
                    .filter(|a| a.plugin == id)
                    .map(|a| a.uuid.clone())
                    .collect::<HashSet<_>>(),
                host.previous_plugins
                    .get(&plugin.root)
                    .map(|previous| previous.backup.clone()),
                if rollback && plugin.runtime.load_failed {
                    host.previous_plugins
                        .get(&plugin.root)
                        .context("No validated previous identity is available")?
                        .id
                        .clone()
                } else {
                    plugin.id.clone()
                },
            )
        };
        // Archive validation and extraction must not hold up physical input or
        // other plugins while the native editor waits for this review.
        let source = if rollback {
            let backup =
                backup.context("No previous version is available for this installation")?;
            anyhow::ensure!(
                backup.target() == root,
                "Previous version belongs to another plugin folder"
            );
            Source::Rollback(backup)
        } else {
            Source::Package(Prepared::new(
                Path::new(request["path"].as_str().context("Package path missing")?),
                &root,
                &state,
                reference.as_deref(),
            )?)
        };
        let (candidate, incoming) = plugins::parse(source.path(), reference.as_deref())?;
        anyhow::ensure!(
            candidate.id == replacement_id,
            "The package belongs to a different plugin"
        );
        runnable(&candidate)?;
        let new_version = version(&candidate.manifest);
        let manifest_hash = Sha256::digest(serde_json::to_vec(&candidate.manifest)?).into();
        let new_actions: HashSet<_> = incoming.iter().map(|a| a.uuid.clone()).collect();
        let mut removed: Vec<_> = old_actions.difference(&new_actions).cloned().collect();
        removed.sort();
        let mut added: Vec<_> = new_actions.difference(&old_actions).cloned().collect();
        added.sort();
        let ticket = uuid::Uuid::new_v4().to_string();
        let review = json!({"ticket":ticket,"id":id,"name":candidate.name,"current_version":current_version,
            "new_version":new_version,"comparison":comparison(&current_version,&new_version),
            "removed_actions":removed,"added_actions":added,"was_enabled":enabled,"kind":if rollback {"rollback"} else {"update"}});
        let mut host = self.shared.lock().unwrap();
        anyhow::ensure!(host.running, "Host stopped while inspecting the package");
        anyhow::ensure!(
            host.plugins.iter().any(|p| p.id == id
                && p.root == root
                && p.registration == launch
                && host.plugin_enabled(p) == enabled),
            "Plugin changed while inspecting the package; try again"
        );
        host.check_update_actions(&id, &incoming)?;
        anyhow::ensure!(
            !host
                .plugins
                .iter()
                .any(|plugin| plugin.id == replacement_id && plugin.root != root),
            "Replacement plugin UUID belongs to another installed folder"
        );
        anyhow::ensure!(
            host.pending_updates.len() < 4,
            "Finish or cancel an existing plugin update first"
        );
        host.pending_updates.insert(
            ticket,
            Pending {
                id,
                replacement_id,
                root,
                launch,
                enabled,
                created: Instant::now(),
                source,
                manifest_hash,
            },
        );
        Ok(review)
    }
    pub fn cancel_plugin_update(&self, ticket: &str) -> Result<()> {
        let pending = self.shared.lock().unwrap().pending_updates.remove(ticket);
        // Deleting uncommitted staging files does not hold the host mutex.
        drop(pending);
        Ok(())
    }
}
impl Host {
    pub(super) fn take_expired_updates(&mut self) -> Vec<Pending> {
        let expired: Vec<_> = self
            .pending_updates
            .iter()
            .filter(|(_, pending)| pending.created.elapsed() >= Duration::from_secs(600))
            .map(|(key, _)| key.clone())
            .collect();
        expired
            .into_iter()
            .filter_map(|key| self.pending_updates.remove(&key))
            .collect()
    }
    fn check_update_actions(&self, id: &str, incoming: &[Action]) -> Result<()> {
        anyhow::ensure!(
            !incoming.iter().any(|new| self
                .actions
                .iter()
                .any(|old| old.plugin != id && old.uuid == new.uuid)),
            "Package action UUID belongs to another installed plugin"
        );
        Ok(())
    }
    pub(super) fn refresh_previous_plugins(&mut self) {
        self.previous_plugins.clear();
        let Some(state) = self.state_path.parent().map(Path::to_owned) else {
            return;
        };
        let mut errors = Vec::new();
        for plugin in &self.plugins {
            if self.plugin_removal_mode(plugin) != removal::Mode::Archive {
                continue;
            }
            let result = (|| -> Result<Option<Previous>> {
                let Some(backup) = crate::plugin_update::find_backup(&plugin.root, &state)? else {
                    return Ok(None);
                };
                let manifest = crate::package::read_installed_manifest(
                    backup.path(),
                    self.reference_app.as_deref(),
                )?;
                anyhow::ensure!(
                    plugin.runtime.load_failed || manifest["UUID"] == plugin.id,
                    "Previous package identity changed"
                );
                Ok(Some(Previous {
                    version: version(&manifest),
                    id: manifest["UUID"]
                        .as_str()
                        .context("Previous plugin UUID missing")?
                        .to_owned(),
                    backup,
                }))
            })();
            match result {
                Ok(Some(previous)) => {
                    self.previous_plugins.insert(plugin.root.clone(), previous);
                }
                Ok(None) => {}
                Err(error) => errors.push(format!(
                    "Previous version unavailable for {}: {error:#}",
                    plugin.id
                )),
            }
        }
        for error in errors {
            self.log(error);
        }
    }
    pub(super) fn apply_plugin_update(&mut self, request: &Value) -> Result<()> {
        anyhow::ensure!(
            request["confirm"] == true,
            "Confirm the reviewed plugin replacement"
        );
        let ticket = request["ticket"]
            .as_str()
            .context("Update review ticket missing")?;
        let pending = self
            .pending_updates
            .remove(ticket)
            .context("Update review expired; inspect the package again")?;
        anyhow::ensure!(
            pending.created.elapsed() < Duration::from_secs(600),
            "Update review expired; inspect the package again"
        );
        let index = self
            .plugins
            .iter()
            .position(|p| {
                p.id == pending.id
                    && p.root == pending.root
                    && p.registration == pending.launch
                    && self.plugin_enabled(p) == pending.enabled
            })
            .context("Plugin changed after review; inspect the package again")?;
        let (candidate, incoming) =
            plugins::parse(pending.source.path(), self.reference_app.as_deref())?;
        anyhow::ensure!(
            candidate.id == pending.replacement_id,
            "Replacement plugin identity changed"
        );
        anyhow::ensure!(
            <[u8; 32]>::from(Sha256::digest(serde_json::to_vec(&candidate.manifest)?))
                == pending.manifest_hash,
            "Replacement manifest changed after review; inspect the package again"
        );
        self.check_update_actions(&pending.id, &incoming)?;
        anyhow::ensure!(
            !self
                .plugins
                .iter()
                .enumerate()
                .any(|(other, plugin)| other != index && plugin.id == pending.replacement_id),
            "Replacement plugin UUID belongs to another installed folder"
        );
        runnable(&candidate)?;
        self.persist()?;
        self.stop_plugin_runtime(index, &pending.id);
        let swapped = match pending.source {
            Source::Package(prepared) => prepared.commit().map(|backup| (backup, true)),
            Source::Rollback(backup) => backup.restore().map(|()| (backup, false)),
        };
        let (backup, forward) = match swapped {
            Ok(value) => value,
            Err(error) => {
                let recovery = self.reload_updated_plugin(index, &pending.id, pending.enabled);
                if let Err(failure) = &recovery {
                    self.plugins[index].runtime.fail(failure.to_string());
                }
                self.refresh_previous_plugins();
                bail!(
                    "Plugin files were not replaced: {error:#}. Previous runtime reload: {}",
                    recovery
                        .err()
                        .map_or_else(|| "succeeded".into(), |e| e.to_string())
                );
            }
        };
        if let Err(error) =
            self.reload_updated_plugin(index, &pending.replacement_id, pending.enabled)
        {
            let stopped_id = self.plugins[index].id.clone();
            self.stop_plugin_runtime(index, &stopped_id);
            let restored = if forward {
                backup.restore()
            } else {
                backup.reapply()
            };
            if let Err(restore) = restored {
                self.plugins[index].runtime.fail(format!(
                    "Replacement failed: {error:#}; restoring files failed: {restore:#}"
                ));
                self.refresh_previous_plugins();
                bail!("Replacement failed and its files could not be restored: {restore:#}");
            }
            let recovery = self.reload_updated_plugin(index, &pending.id, pending.enabled);
            if let Err(failure) = &recovery {
                self.plugins[index].runtime.fail(failure.to_string());
            }
            self.refresh_previous_plugins();
            bail!(
                "Replacement could not start; the previous files were restored. Runtime reload: {}. Cause: {error:#}",
                recovery
                    .err()
                    .map_or_else(|| "succeeded".into(), |e| e.to_string())
            );
        }
        self.refresh_previous_plugins();
        self.log(format!(
            "Plugin {}: {}",
            if forward { "updated" } else { "restored" },
            pending.id
        ));
        Ok(())
    }
    fn reload_updated_plugin(&mut self, index: usize, id: &str, enabled: bool) -> Result<()> {
        let root = self.plugins[index].root.clone();
        let old_id = self.plugins[index].id.clone();
        let (mut replacement, incoming) = plugins::parse(&root, self.reference_app.as_deref())?;
        anyhow::ensure!(replacement.id == id, "Replacement plugin UUID changed");
        self.check_update_actions(&old_id, &incoming)?;
        let previous: Vec<_> = self
            .actions
            .iter()
            .filter(|a| a.plugin == old_id)
            .cloned()
            .collect();
        self.actions.retain(|a| a.plugin != old_id);
        self.actions.extend(incoming.clone());
        replacement.runtime.status = if enabled { "stopped" } else { "disabled" };
        self.plugins[index] = replacement;
        self.refresh_localizations();
        self.refresh_updated_plugin_assets(&previous, &incoming);
        self.record_plugin_asset_identity(index)?;
        for device in self.device_ids() {
            self.with_device(&device, |host| {
                host.render()?;
                host.sync_visible();
                Ok(())
            })?;
        }
        self.persist()?;
        if enabled {
            let state = self
                .state_path
                .parent()
                .context("State directory missing")?
                .to_owned();
            let devices = self.registration_devices();
            launch_plugin(
                &mut self.plugins[index],
                &devices,
                self.port,
                &state,
                self.language,
            )?;
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
    use std::io::Write;
    fn manifest(version: &str) -> Value {
        json!({"UUID":"test.plugin","Name":"Update fixture","Version":version,"CodePath":"plugin.html",
            "Actions":[{"UUID":"test.action","Name":"Fixture","Controllers":["Keypad"],"States":[{}]}]})
    }
    fn package(root: &Path, version: &str) -> PathBuf {
        package_manifest(root, version, manifest(version))
    }
    fn package_manifest(root: &Path, version: &str, manifest: Value) -> PathBuf {
        let path = root.join(format!("{version}.streamDeckPlugin"));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        zip.start_file("test.plugin.sdPlugin/manifest.json", options)
            .unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.start_file("test.plugin.sdPlugin/plugin.html", options)
            .unwrap();
        zip.write_all(b"<!doctype html><title>Inert update test</title>")
            .unwrap();
        zip.finish().unwrap();
        path
    }
    fn setup(root: &Path) -> Controller {
        let state = root.join("state");
        let plugin = state.join("plugins/test.plugin.sdPlugin");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("manifest.json"),
            serde_json::to_vec(&manifest("1.0.0")).unwrap(),
        )
        .unwrap();
        fs::write(
            plugin.join("plugin.html"),
            "<!doctype html><title>Old inert test</title>",
        )
        .unwrap();
        let mut host = crate::host::tests::fixture(&state);
        let (loaded, actions) = plugins::parse(&plugin, None).unwrap();
        host.plugins[0] = loaded;
        host.actions = actions;
        host.disabled_plugins.insert(plugin);
        host.plugins[0].runtime.status = "disabled";
        host.slots[0].settings = json!({"count":42,"unknown":{"keep":true}});
        host.sync_visible();
        Controller {
            shared: Arc::new(Mutex::new(host)),
        }
    }
    fn request(controller: &Controller, path: Option<&Path>) -> Value {
        let host = controller.shared.lock().unwrap();
        let p = &host.plugins[0];
        json!({"id":p.id,"path":path,"expected_path":p.root,"expected_version":p.manifest["Version"]})
    }
    fn apply(controller: &Controller, review: &Value) -> Result<()> {
        controller.command(
            json!({"command":"plugin_update_apply","ticket":review["ticket"],"confirm":true}),
        )
    }
    #[test]
    fn disabled_update_and_restore_preserve_current_settings_and_refresh_icon_generation() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let package = package(root.path(), "2.0.0");
        let old_generation = controller.shared.lock().unwrap().plugins[0].asset_generation;
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        assert_eq!(review["comparison"], "upgrade");
        assert_eq!(review["was_enabled"], false);
        assert_eq!(
            controller.shared.lock().unwrap().plugins[0].manifest["Version"],
            "1.0.0"
        );
        apply(&controller, &review).unwrap();
        {
            let mut host = controller.shared.lock().unwrap();
            assert_eq!(host.plugins[0].manifest["Version"], "2.0.0");
            assert_eq!(host.plugins[0].runtime.status, "disabled");
            assert!(host.plugins[0].child.is_none());
            assert_ne!(host.plugins[0].asset_generation, old_generation);
            assert_eq!(
                host.previous_plugins[&host.plugins[0].root].version,
                "1.0.0"
            );
            assert_eq!(host.slots[0].settings["count"], 42);
            host.slots[0].settings["count"] = json!(99);
            host.sync_visible();
            host.persist().unwrap();
        }
        let restore = controller
            .prepare_plugin_update(request(&controller, None), true)
            .unwrap();
        assert_eq!(restore["kind"], "rollback");
        assert_eq!(restore["comparison"], "downgrade");
        apply(&controller, &restore).unwrap();
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.plugins[0].manifest["Version"], "1.0.0");
        assert!(!host.plugin_enabled(&host.plugins[0]));
        assert_eq!(
            host.slots[0].settings,
            json!({"count":99,"unknown":{"keep":true}})
        );
        assert!(!host.previous_plugins.contains_key(&host.plugins[0].root));
    }
    #[test]
    fn stale_review_and_cancel_never_replace_files() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let package = package(root.path(), "2.0.0");
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        controller.shared.lock().unwrap().plugins[0].registration = "different-launch".into();
        assert!(
            apply(&controller, &review)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        controller
            .cancel_plugin_update(review["ticket"].as_str().unwrap())
            .unwrap();
        assert!(apply(&controller, &review).is_err());
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.plugins[0].manifest["Version"], "1.0.0");
        assert!(host.pending_updates.is_empty());
        assert!(host.previous_plugins.is_empty());
        assert_eq!(
            fs::read_dir(root.path().join("state/plugin-updates"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    fn failed_save_keeps_the_previous_connection_and_installation() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let package = package(root.path(), "2.0.0");
        let (sender, mut receiver) = mpsc::unbounded_channel();
        {
            let mut host = controller.shared.lock().unwrap();
            host.disabled_plugins.clear();
            host.plugins[0].sender = Some(sender);
            host.plugins[0].runtime.session = Some("connected".into());
            host.plugins[0].runtime.status = "connected";
            fs::create_dir(&host.state_path).unwrap();
        }
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        assert!(apply(&controller, &review).is_err());
        let host = controller.shared.lock().unwrap();
        assert!(host.plugins[0].sender.is_some());
        assert_eq!(host.plugins[0].runtime.status, "connected");
        assert_eq!(host.plugins[0].manifest["Version"], "1.0.0");
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            crate::package::read_installed_manifest(&host.plugins[0].root, None).unwrap()["Version"],
            "1.0.0"
        );
    }
    #[test]
    fn expired_review_is_reaped_and_confirmation_is_required() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let package = package(root.path(), "2.0.0");
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        let ticket = review["ticket"].as_str().unwrap();
        assert!(
            controller
                .command(json!({"command":"plugin_update_apply","ticket":ticket}))
                .is_err()
        );
        let expired = {
            let mut host = controller.shared.lock().unwrap();
            host.pending_updates.get_mut(ticket).unwrap().created -= Duration::from_secs(601);
            host.take_expired_updates()
        };
        assert_eq!(expired.len(), 1);
        drop(expired);
        assert!(apply(&controller, &review).is_err());
    }
    #[test]
    fn replacement_manifest_cannot_change_after_version_review() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let package = package(root.path(), "2.0.0");
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        let candidate = controller.shared.lock().unwrap().pending_updates
            [review["ticket"].as_str().unwrap()]
        .source
        .path()
        .to_owned();
        fs::write(
            candidate.join("manifest.json"),
            serde_json::to_vec(&manifest("3.0.0")).unwrap(),
        )
        .unwrap();
        assert!(
            apply(&controller, &review)
                .unwrap_err()
                .to_string()
                .contains("changed after review")
        );
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.plugins[0].manifest["Version"], "1.0.0");
        assert_eq!(
            crate::package::read_installed_manifest(&host.plugins[0].root, None).unwrap()["Version"],
            "1.0.0"
        );
    }
    #[test]
    fn damaged_current_manifest_can_restore_its_validated_previous_identity() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let package = package(root.path(), "2.0.0");
        let review = controller
            .prepare_plugin_update(request(&controller, Some(&package)), false)
            .unwrap();
        apply(&controller, &review).unwrap();
        {
            let mut host = controller.shared.lock().unwrap();
            let installed = host.plugins[0].root.clone();
            fs::write(installed.join("manifest.json"), b"damaged manifest").unwrap();
            let (plugins, actions) = plugins::load(vec![installed.clone()], None).unwrap();
            host.plugins = plugins;
            host.actions = actions;
            assert!(host.plugins[0].runtime.load_failed);
            assert!(host.plugins[0].id.starts_with("unavailable."));
            host.refresh_previous_plugins();
            assert_eq!(host.previous_plugins[&installed].id, "test.plugin");
        }
        let repair = controller
            .prepare_plugin_update(request(&controller, None), true)
            .unwrap();
        assert_eq!(repair["current_version"], "Unknown");
        assert_eq!(repair["new_version"], "1.0.0");
        apply(&controller, &repair).unwrap();
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.plugins[0].id, "test.plugin");
        assert_eq!(host.plugins[0].manifest["Version"], "1.0.0");
        assert!(!host.plugins[0].runtime.load_failed);
        assert_eq!(host.slots[0].settings["count"], 42);
        assert!(
            host.actions
                .iter()
                .any(|action| action.uuid == "test.action")
        );
        assert_eq!(snapshot(&host)["slots"][0]["plugin_disabled"], true);
    }
    #[test]
    fn missing_default_dial_layout_is_rejected_before_replacement() {
        let root = tempfile::tempdir().unwrap();
        let controller = setup(root.path());
        let mut manifest = manifest("2.0.0");
        manifest["Actions"][0]["Controllers"] = json!(["Encoder"]);
        manifest["Actions"][0]["Encoder"] = json!({"layout":"missing.json"});
        let package = package_manifest(root.path(), "invalid-layout", manifest);
        assert!(
            controller
                .prepare_plugin_update(request(&controller, Some(&package)), false)
                .unwrap_err()
                .to_string()
                .contains("dial layout")
        );
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.plugins[0].manifest["Version"], "1.0.0");
        assert!(host.pending_updates.is_empty());
        assert_eq!(
            fs::read_dir(root.path().join("state/plugin-updates"))
                .unwrap()
                .count(),
            0
        );
    }
}
