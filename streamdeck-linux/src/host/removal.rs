//! Explicit plugin removal preserves the profile graph and per-plugin data.
use super::*;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{DirBuilderExt, MetadataExt},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Archive,
    Detach,
    Missing,
}
impl Mode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Detach => "detach",
            Self::Missing => "missing",
        }
    }
}
struct Archive {
    source: PathBuf,
    destination: PathBuf,
    directory: PathBuf,
    identity: (u64, u64),
}
impl Archive {
    fn prepare(source: &Path, state: &Path) -> Result<Self> {
        let parent = state.join("removed-plugins");
        private_directory(&parent)?;
        let directory = parent.join(uuid::Uuid::new_v4().to_string());
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let metadata = fs::symlink_metadata(source)?;
        Ok(Self {
            source: source.to_owned(),
            destination: directory.join(source.file_name().context("Plugin folder has no name")?),
            directory,
            identity: (metadata.dev(), metadata.ino()),
        })
    }
    fn move_files(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.source)?;
        anyhow::ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && (metadata.dev(), metadata.ino()) == self.identity,
            "Plugin folder changed during removal"
        );
        let from = std::ffi::CString::new(self.source.as_os_str().as_bytes())?;
        let to = std::ffi::CString::new(self.destination.as_os_str().as_bytes())?;
        // Never overwrite an existing backup. Both paths were prepared under
        // the host mutex; a replaced source directory is rejected above.
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}
impl Drop for Archive {
    fn drop(&mut self) {
        // Removes only our empty preparation directory. A completed archive is
        // nonempty and is deliberately retained; never recursively delete it.
        let _ = fs::remove_dir(&self.directory);
    }
}
fn private_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => anyhow::ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Plugin backup directory must be owned and private"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(path)?
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
impl Host {
    pub(super) fn plugin_removal_mode(&self, plugin: &Plugin) -> Mode {
        let metadata = match fs::symlink_metadata(&plugin.root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Mode::Missing,
            Err(_) => return Mode::Detach,
        };
        let managed = self
            .state_path
            .parent()
            .and_then(|state| state.canonicalize().ok())
            .map(|state| state.join("plugins"));
        if let Some(managed) = managed
            && plugin.root.parent() == Some(managed.as_path())
            && metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && fs::symlink_metadata(&managed)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            Mode::Archive
        } else {
            Mode::Detach
        }
    }
    pub(super) fn uninstall_plugin(&mut self, request: &Value) -> Result<()> {
        anyhow::ensure!(request["confirm"] == true, "Confirm removal of this plugin");
        let id = request["id"].as_str().context("Plugin id missing")?;
        let expected = request["expected_path"]
            .as_str()
            .context("Expected plugin folder missing")?;
        let enabled = request["expected_enabled"]
            .as_bool()
            .context("Expected plugin state missing")?;
        let index = self
            .plugins
            .iter()
            .position(|p| p.id == id)
            .context("Plugin is no longer installed")?;
        let plugin = &self.plugins[index];
        anyhow::ensure!(
            plugin.root == Path::new(expected) && self.plugin_enabled(plugin) == enabled,
            "Plugin changed; review its removal again"
        );
        let root = plugin.root.clone();
        let mode = self.plugin_removal_mode(plugin);
        let archive = if mode == Mode::Archive {
            Some(Archive::prepare(
                &root,
                self.state_path
                    .parent()
                    .context("Plugin state directory missing")?,
            )?)
        } else {
            None
        };
        // A failed save must not disrupt the current backend or its inspectors.
        // After this commit, a crash cannot automatically restart the removed
        // plugin. A crash before the archive move can leave inert files in place.
        self.persist_without_plugin(Some(index))?;
        self.stop_plugin_runtime(index, id);
        self.disabled_plugins.remove(&root);
        self.actions.retain(|action| action.plugin != id);
        self.plugins.remove(index);
        // WebSocket sessions resolve their stable launch identity per message.
        // Inspector indices are local to the mutex-owned device runtime maps.
        for inspector in self.inspectors.values_mut().chain(
            self.other_devices
                .values_mut()
                .flat_map(|device| device.inspectors.values_mut()),
        ) {
            if inspector.plugin > index {
                inspector.plugin -= 1;
            }
        }
        self.refresh_localizations();
        let moved = archive
            .as_ref()
            .map(|archive| archive.move_files())
            .transpose();
        for device in self.device_ids() {
            if let Err(error) = self.with_device(&device, |host| host.render()) {
                self.log(format!(
                    "Removed plugin; could not redraw {device}: {error:#}"
                ));
            }
        }
        if let Err(error) = moved {
            let message = format!(
                "Plugin removed from Deckforge, but its files remain at {} because archiving failed: {error}",
                root.display()
            );
            self.log(&message);
            bail!(message);
        }
        if let Some(archive) = &archive {
            self.log(format!(
                "Plugin removed: {id}. Files retained at {}",
                archive.destination.display()
            ));
        } else {
            self.log(format!(
                "Plugin removed: {id}. External files and plugin data were retained."
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn request(host: &Host) -> Value {
        json!({"command":"uninstall_plugin","id":host.plugins[0].id,"expected_path":host.plugins[0].root,
            "expected_enabled":host.plugin_enabled(&host.plugins[0]),"confirm":true})
    }
    fn managed(root: &Path) -> Host {
        let state = root.join("state");
        let plugin = state.join("plugins/test.sdPlugin");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("keep.txt"), "plugin payload").unwrap();
        let mut host = crate::host::tests::fixture(&state);
        host.plugins[0].root = plugin;
        host
    }
    #[test]
    fn failed_save_preserves_live_session_and_managed_files() {
        let root = tempfile::tempdir().unwrap();
        let mut host = managed(root.path());
        let (sender, mut receiver) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.plugins[0].runtime.session = Some("current-session".into());
        host.plugins[0].runtime.status = "connected";
        fs::create_dir(&host.state_path).unwrap();
        let requested = request(&host);
        assert!(host.uninstall_plugin(&requested).is_err());
        assert!(host.plugin_session_current(0, "secret", "current-session"));
        assert_eq!(host.plugins[0].runtime.status, "connected");
        assert!(receiver.try_recv().is_err());
        assert!(host.plugins[0].root.join("keep.txt").is_file());
        assert_eq!(
            fs::read_dir(root.path().join("state/removed-plugins"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(host.actions.len(), 1);
    }
    #[test]
    fn managed_removal_archives_files_and_keeps_profile_and_private_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut host = managed(root.path());
        let source = host.plugins[0].root.clone();
        host.disabled_plugins.insert(source.clone());
        host.slots[0].settings = json!({"counter":4,"future":{"keep":true}});
        host.globals
            .insert("test.plugin".into(), json!({"keep":42}));
        let private = root.path().join("state/plugin-data/test");
        fs::create_dir_all(&private).unwrap();
        fs::write(private.join("keep.json"), "{}").unwrap();
        host.sync_visible();
        let before = serde_json::to_value(&host.library).unwrap();
        let requested = request(&host);
        host.uninstall_plugin(&requested).unwrap();
        assert!(host.plugins.is_empty());
        assert!(host.actions.is_empty());
        assert!(!source.exists());
        let backups = root.path().join("state/removed-plugins");
        let backup = fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::metadata(&backups).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&backup).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            fs::read_to_string(backup.join("test.sdPlugin/keep.txt")).unwrap(),
            "plugin payload"
        );
        assert!(private.join("keep.json").is_file());
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert_eq!(
            host.slots[0].settings,
            json!({"counter":4,"future":{"keep":true}})
        );
        assert_eq!(host.slots[0].title, "before");
        assert_eq!(snapshot(&host)["slots"][0]["available"], false);
        assert!(!host.slots[0].preview.is_empty());
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.plugins.is_empty() && saved.disabled_plugins.is_empty());
        assert_eq!(saved.globals["test.plugin"], json!({"keep":42}));
        assert_eq!(
            serde_json::to_value(saved.library.unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn external_and_missing_entries_detach_without_moving_source_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let state = root.path().join("state");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(source.join("keep.txt"), "developer source").unwrap();
        for missing in [false, true] {
            let mut host = crate::host::tests::fixture(&state);
            host.plugins[0].root = if missing {
                source.join("missing")
            } else {
                source.clone()
            };
            assert_eq!(
                host.plugin_removal_mode(&host.plugins[0]).as_str(),
                if missing { "missing" } else { "detach" }
            );
            let requested = request(&host);
            host.uninstall_plugin(&requested).unwrap();
            assert!(host.plugins.is_empty());
            assert_eq!(
                fs::read_to_string(source.join("keep.txt")).unwrap(),
                "developer source"
            );
            assert!(!state.join("removed-plugins").exists());
        }
    }
    #[test]
    fn unsafe_backup_directory_cannot_disrupt_the_plugin() {
        let root = tempfile::tempdir().unwrap();
        let mut host = managed(root.path());
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.path().join("state/removed-plugins")).unwrap();
        let requested = request(&host);
        assert!(host.uninstall_plugin(&requested).is_err());
        assert_eq!(host.plugins.len(), 1);
        assert!(host.plugins[0].root.join("keep.txt").is_file());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }
    #[test]
    fn archive_never_overwrites_or_moves_a_replaced_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("plugin");
        fs::create_dir(&source).unwrap();
        let archive = Archive::prepare(&source, root.path()).unwrap();
        fs::create_dir(&archive.destination).unwrap();
        fs::write(archive.destination.join("keep"), "previous").unwrap();
        assert!(archive.move_files().is_err());
        assert!(source.is_dir());
        assert_eq!(
            fs::read_to_string(archive.destination.join("keep")).unwrap(),
            "previous"
        );
        let previous = root.path().join("old");
        fs::rename(&source, &previous).unwrap();
        symlink(&previous, &source).unwrap();
        assert!(archive.move_files().is_err());
        assert!(previous.is_dir());
    }
    #[test]
    fn missing_pedal_action_has_a_software_preview_without_hardware_pixels() {
        let root = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(root.path());
        host.model = Model::Pedal;
        host.slots[0].model = Model::Pedal;
        host.actions.clear();
        host.render().unwrap();
        let picture =
            image::load_from_memory(&STANDARD.decode(&host.slots[0].preview).unwrap()).unwrap();
        assert_eq!((picture.width(), picture.height()), (120, 120));
        assert_eq!(host.slots[0].title, "before");
    }
    #[test]
    fn archive_failure_after_commit_leaves_files_but_not_a_registered_plugin() {
        use std::os::unix::fs::PermissionsExt;
        // Root bypasses directory write permissions; other failure cases above
        // remain portable while this real permission-denied case needs a user.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut host = managed(root.path());
        let source = host.plugins[0].root.clone();
        let parent = source.parent().unwrap();
        let permissions = fs::metadata(parent).unwrap().permissions();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).unwrap();
        let requested = request(&host);
        let outcome = host.uninstall_plugin(&requested);
        fs::set_permissions(parent, permissions).unwrap();
        let error = outcome.unwrap_err().to_string();
        assert!(error.contains("removed from Deckforge") && error.contains("files remain"));
        assert!(host.plugins.is_empty() && host.actions.is_empty());
        assert_eq!(
            fs::read_to_string(source.join("keep.txt")).unwrap(),
            "plugin payload"
        );
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.plugins.is_empty());
        assert_eq!(saved.slots.len(), 1);
        assert_eq!(
            fs::read_dir(root.path().join("state/removed-plugins"))
                .unwrap()
                .count(),
            0
        );
    }
}
