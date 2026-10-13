//! Network/browser work stays outside the host lock; existing package policy owns installation.
use super::*;
use crate::marketplace;

impl Controller {
    pub fn marketplace_request(&self, action: &str, variant: Option<uuid::Uuid>) -> Result<()> {
        anyhow::ensure!(
            matches!(
                action,
                "login" | "check" | "logout" | "install" | "retry" | "apply" | "cancel"
            ),
            "Unknown Marketplace action"
        );
        let (state, variant) = {
            let mut host = self.shared.lock().unwrap();
            anyhow::ensure!(host.running, "Deckforge is shutting down");
            if host.marketplace.busy && variant.is_some() && variant == host.marketplace.pending {
                return Ok(());
            }
            anyhow::ensure!(
                !host.marketplace.busy,
                "A Marketplace request is already in progress"
            );
            if let Some(review) = &host.marketplace.review {
                anyhow::ensure!(
                    matches!(action, "apply" | "cancel"),
                    "Review or cancel the downloaded plugin update first"
                );
                let ticket = review["ticket"].clone();
                drop(host);
                self.command(if action == "apply" {
                    json!({"command":"plugin_update_apply","ticket":ticket,"confirm":true})
                } else {
                    json!({"command":"plugin_update_cancel","ticket":ticket})
                })?;
                self.marketplace_change(|s| {
                    s.review = None;
                    s.pending = None;
                    s.error = None;
                    s.message = if action == "apply" {
                        "Marketplace plugin updated"
                    } else {
                        "Marketplace update cancelled"
                    }
                    .into();
                });
                return Ok(());
            }
            anyhow::ensure!(
                !matches!(action, "apply" | "cancel"),
                "No Marketplace update is waiting for review"
            );
            if action == "install" {
                host.marketplace.pending = Some(variant.context("Marketplace variant missing")?);
            }
            if action == "retry" {
                anyhow::ensure!(
                    host.marketplace.pending.is_some(),
                    "No Marketplace install is waiting"
                );
            }
            let pending = if matches!(action, "install" | "retry" | "login") {
                host.marketplace.pending
            } else {
                None
            };
            host.marketplace.busy = true;
            host.marketplace.error = None;
            if action != "logout" {
                // A previously successful check is not evidence of a current
                // session if reauthentication is cancelled or the network fails.
                host.marketplace.authenticated = None;
                host.marketplace.account = None;
            }
            host.marketplace.message = match action {
                "logout" => "Signing out of Marketplace…",
                "check" => "Checking Marketplace session…",
                _ if pending.is_some() => {
                    "Contacting Marketplace. Complete sign-in if a window opens…"
                }
                _ => "Complete sign-in in the Elgato Marketplace window…",
            }
            .into();
            (
                host.state_path
                    .parent()
                    .context("State directory missing")?
                    .to_owned(),
                pending,
            )
        };
        let controller = self.clone();
        let action = action.to_owned();
        let result = std::thread::Builder::new()
            .name("marketplace".into())
            .spawn(move || {
                let result = controller.marketplace_work(&state, &action, variant);
                controller.marketplace_change(|s| {
                    s.busy = false;
                    if let Err(error) = result {
                        s.error = Some(error.to_string());
                        s.message = "Marketplace request did not complete".into();
                    }
                });
                if action != "check" {
                    let mut host = controller.shared.lock().unwrap();
                    host.system_status = host
                        .marketplace
                        .error
                        .clone()
                        .unwrap_or_else(|| host.marketplace.message.clone());
                }
            });
        if result.is_err() {
            self.marketplace_change(|s| {
                s.busy = false;
                s.error = Some("Could not start Marketplace request".into());
            });
            bail!("Could not start Marketplace request");
        }
        Ok(())
    }

    fn marketplace_change(&self, update: impl FnOnce(&mut marketplace::State)) {
        let mut host = self.shared.lock().unwrap();
        update(&mut host.marketplace);
        if let Some(redraw) = &host.redraw {
            redraw();
        }
    }

    fn marketplace_work(
        &self,
        state: &Path,
        action: &str,
        variant: Option<uuid::Uuid>,
    ) -> Result<()> {
        let _lock = marketplace::lock(state)?;
        if action == "logout" {
            marketplace::clear_session(state)?;
            self.marketplace_change(|s| {
                s.authenticated = Some(false);
                s.account = None;
                s.message = "Signed out of Marketplace on this device".into();
            });
            return Ok(());
        }
        let operation = if variant.is_some() {
            "install"
        } else if action == "check" {
            "check"
        } else {
            "login"
        };
        let reply = marketplace::session(state, operation, variant, || self.is_running())?;
        if reply.kind == "signed_out" {
            self.marketplace_change(|s| {
                s.authenticated = Some(false);
                s.account = None;
                s.message = "Sign in to Marketplace to install plugins".into();
            });
            return Ok(());
        }
        if reply.name.is_some() {
            self.marketplace_change(|s| {
                s.authenticated = Some(true);
                s.account = reply.name.clone();
            });
        }
        match reply.kind.as_str() {
            "error" => bail!(
                "{}",
                reply
                    .error
                    .unwrap_or_else(|| "Marketplace request failed".into())
            ),
            "authenticated" if variant.is_none() => {
                self.marketplace_change(|s| s.message = "Marketplace session ready".into());
                return Ok(());
            }
            "download" if variant.is_some() => {}
            _ => bail!("Unexpected Marketplace response"),
        }
        let temporary = tempfile::Builder::new()
            .prefix("download-")
            .tempdir_in(state.join("marketplace"))?;
        let link = reply.url.context("Marketplace download URL missing")?;
        self.marketplace_change(|s| s.message = "Downloading Marketplace plugin…".into());
        let mut last_update = std::time::Instant::now();
        let package = marketplace::download(&link, temporary.path(), |bytes, total| {
            if last_update.elapsed() >= Duration::from_millis(200) {
                last_update = std::time::Instant::now();
                self.marketplace_change(|s| {
                    s.message = match total {
                        Some(total) if total > 0 => {
                            format!("Downloading Marketplace plugin… {}%", bytes * 100 / total)
                        }
                        _ => format!("Downloading Marketplace plugin… {} KiB", bytes / 1024),
                    }
                });
            }
            self.is_running()
        })?;
        self.marketplace_change(|s| s.message = "Verifying Marketplace package…".into());
        self.install_marketplace_package(&package, temporary.path())
    }

    fn install_marketplace_package(&self, package: &Path, staging: &Path) -> Result<()> {
        let reference = self.shared.lock().unwrap().reference_app.clone();
        let root = crate::package::install_with_reference(
            package,
            &staging.join("verified"),
            reference.as_deref(),
        )?;
        let (incoming, actions) = plugins::parse(&root, reference.as_deref())?;
        let mut host = self.shared.lock().unwrap();
        anyhow::ensure!(host.running, "Deckforge stopped before installation");
        anyhow::ensure!(
            host.reference_app == reference,
            "Package verification reference changed. Retry installation."
        );
        if let Some(existing) = host.plugins.iter().find(|p| p.id == incoming.id) {
            if existing.manifest["Version"] == incoming.manifest["Version"]
                && !existing.runtime.load_failed
            {
                let name = existing.name.clone();
                host.marketplace.pending = None;
                host.marketplace.message = format!("{name} is already installed at this version");
                return Ok(());
            }
            let request = json!({"id":existing.id,"expected_path":existing.root,"expected_version":existing.manifest["Version"],"path":package});
            drop(host);
            let review = self.prepare_plugin_update(request, false)?;
            self.marketplace_change(|s| {
                s.review = Some(review);
                s.message =
                    "Plugin update downloaded. Review it in Preferences → Marketplace.".into();
            });
            return Ok(());
        }
        anyhow::ensure!(
            !actions
                .iter()
                .any(|incoming| host.actions.iter().any(|a| a.uuid == incoming.uuid)),
            "Plugin declares an action UUID already owned by another installed plugin"
        );
        let destination = host
            .state_path
            .parent()
            .context("State directory missing")?
            .join("plugins");
        fs::create_dir_all(&destination)?;
        anyhow::ensure!(
            !fs::symlink_metadata(&destination)?.file_type().is_symlink(),
            "Unsafe managed plugin directory"
        );
        let installed = destination.join(root.file_name().context("Plugin folder missing")?);
        crate::package::rename_without_replacing(&root, &installed)?;
        // Reuse registration, disabled-state, bundled-profile and persistence policy.
        apply_control(
            &mut host,
            json!({"command":"install_plugin","path":installed}),
        )?;
        host.marketplace.pending = None;
        let plugin = host
            .plugins
            .iter()
            .find(|p| p.id == incoming.id)
            .context("Installed plugin missing")?;
        host.marketplace.message = format!(
            "Installed {}. Check Plugins for runtime status.",
            plugin.name
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    const ID: &str = "com.deckforge.marketplace-test";
    fn archive(root: &Path, version: &str, action: &str) -> PathBuf {
        let path = root.join(format!("{version}.streamDeckPlugin"));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file(format!("{ID}.sdPlugin/manifest.json"), opts)
            .unwrap();
        zip.write_all(json!({"UUID":ID,"Name":"Marketplace fixture","Version":version,"CodePath":"plugin.html","Actions":[{"UUID":action,"Name":"Fixture","States":[{}]}]}).to_string().as_bytes()).unwrap();
        zip.start_file(format!("{ID}.sdPlugin/plugin.html"), opts)
            .unwrap();
        zip.write_all(b"<!doctype html><title>Inert fixture</title>")
            .unwrap();
        zip.finish().unwrap();
        path
    }
    fn controller(root: &Path) -> Controller {
        let mut host = super::super::tests::fixture(root);
        host.disabled_plugins
            .insert(root.join(format!("plugins/{ID}.sdPlugin")));
        Controller {
            shared: Arc::new(Mutex::new(host)),
        }
    }
    #[test]
    fn marketplace_install_reuses_lifecycle_and_stages_updates_without_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let c = controller(root.path());
        let package = archive(root.path(), "1.0.0", "marketplace.action");
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        c.install_marketplace_package(&package, staging.path())
            .unwrap();
        let s = c.snapshot();
        assert_eq!(s["plugins"][1]["id"], ID);
        assert_eq!(s["plugins"][1]["status"], "disabled");
        let destination = root
            .path()
            .join(format!("plugins/{ID}.sdPlugin/manifest.json"));
        let before = fs::read(&destination).unwrap();
        let duplicate = tempfile::tempdir_in(root.path()).unwrap();
        c.install_marketplace_package(&package, duplicate.path())
            .unwrap();
        assert_eq!(c.snapshot()["plugins"].as_array().unwrap().len(), 2);
        assert!(
            c.snapshot()["marketplace"]["message"]
                .as_str()
                .unwrap()
                .contains("already installed")
        );
        let update = archive(root.path(), "2.0.0", "marketplace.action");
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        c.install_marketplace_package(&update, staging.path())
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), before);
        assert_eq!(
            c.snapshot()["marketplace"]["review"]["new_version"],
            "2.0.0"
        );
        c.marketplace_request("apply", None).unwrap();
        assert_eq!(c.snapshot()["plugins"][1]["version"], "2.0.0");
        assert_eq!(c.snapshot()["plugins"][1]["status"], "disabled");
        assert!(c.snapshot()["marketplace"]["review"].is_null());
        let saved = fs::read_to_string(root.path().join("state.json")).unwrap();
        assert!(!saved.contains("authenticated"));
        assert!(!saved.contains("purchase-link"));
    }
    #[test]
    fn invalid_or_conflicting_packages_leave_managed_directory_untouched() {
        let root = tempfile::tempdir().unwrap();
        let c = controller(root.path());
        let package = archive(root.path(), "1.0.0", "test.action");
        let staging = tempfile::tempdir_in(root.path()).unwrap();
        assert!(
            c.install_marketplace_package(&package, staging.path())
                .is_err()
        );
        assert!(!root.path().join(format!("plugins/{ID}.sdPlugin")).exists());
        assert_eq!(c.snapshot()["plugins"].as_array().unwrap().len(), 1);
        fs::write(&package, "not a zip").unwrap();
        assert!(
            c.install_marketplace_package(&package, staging.path())
                .is_err()
        );
    }
    #[test]
    fn duplicate_inflight_links_are_accepted_without_starting_another_job() {
        let root = tempfile::tempdir().unwrap();
        let c = controller(root.path());
        let id = uuid::Uuid::new_v4();
        {
            let mut host = c.shared.lock().unwrap();
            host.marketplace.busy = true;
            host.marketplace.pending = Some(id);
        }
        c.deliver_deep_link(&format!("streamdeck://plugins/install/{id}"))
            .unwrap();
        assert!(
            c.marketplace_request("install", Some(uuid::Uuid::new_v4()))
                .is_err()
        );
        assert!(c.marketplace_request("logout", None).is_err());
    }
}
