//! Desktop file dispatch reuses the same import/install operations as the editor.
use super::*;
use crate::file_open::Kind;
impl Controller {
    pub fn open_desktop_file(&self, path: &Path) -> Result<String> {
        let device = self.snapshot()["device"]
            .as_str()
            .context("Selected device missing")?
            .to_owned();
        self.open_desktop_file_on(path, &device)
    }
    pub fn open_desktop_file_on(&self, path: &Path, device: &str) -> Result<String> {
        let result = (|| -> Result<&str> {
            let path = crate::file_open::parse(path.to_str().context("File path must be UTF-8")?)?;
            match crate::file_open::kind(&path)? {
                Kind::Plugin => {
                    self.command(json!({"command":"install_plugin","path":path}))?;
                    Ok("Plugin installed. Check Plugins for its runtime status.")
                }
                Kind::IconPack => {
                    self.icon_packs(&json!({"operation":"install","path":path}))?;
                    Ok("Icon pack installed. Open Appearance → Icon library to use it.")
                }
                kind => {
                    let mut host = self.shared.lock().unwrap();
                    anyhow::ensure!(host.running, "Host has stopped");
                    host.with_device(device, |host| match kind {
                        Kind::Action => {
                            host.load_action_clipboard(&path)?;
                            Ok("Action copied from file. Select a control and paste.")
                        }
                        Kind::Profile => {
                            host.history_edit("Import profile", None, |host| {
                                host.import_profile(&path)
                            })?;
                            Ok("Profile imported from file.")
                        }
                        _ => unreachable!(),
                    })
                }
            }
        })();
        let mut host = self.shared.lock().unwrap();
        host.system_status = match &result {
            Ok(message) => message.to_string(),
            Err(error) => format!("Could not open file: {error}"),
        };
        let status = host.system_status.clone();
        host.log(status);
        if let Some(redraw) = &host.redraw {
            redraw();
        }
        result.map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn failed_plugin_open_preserves_history_and_icon_pack_open_preserves_assignments() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = crate::host::tests::fixture(dir.path());
        let id = host.library.active_profile.clone();
        apply_control(
            &mut host,
            json!({"command":"profile_rename","id":id,"name":"Keep history"}),
        )
        .unwrap();
        let history = host.history.snapshot();
        let graph = serde_json::to_value(&host.library).unwrap();
        let saved = fs::read(&host.state_path).unwrap();
        let controller = Controller {
            shared: Arc::new(Mutex::new(host)),
        };
        let bad = dir.path().join("bad.streamDeckPlugin");
        fs::write(&bad, b"invalid zip").unwrap();
        assert!(controller.open_desktop_file(&bad).is_err());
        {
            let host = controller.shared.lock().unwrap();
            assert_eq!(host.history.snapshot(), history);
            assert_eq!(serde_json::to_value(&host.library).unwrap(), graph);
        }
        let pack = dir.path().join("icons.streamDeckIconPack");
        let mut zip = zip::ZipWriter::new(fs::File::create(&pack).unwrap());
        for (name,bytes) in [
            ("owned.sdIconPack/manifest.json",br#"{"Name":"Owned icons","Version":"1","Author":"Fixture","Icon":"icon.svg"}"#.as_slice()),
            ("owned.sdIconPack/icons/icon.svg",br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"##),
        ] {zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(bytes).unwrap();}
        zip.finish().unwrap();
        assert!(
            controller
                .open_desktop_file_on(&pack, "a-forgotten-device")
                .unwrap()
                .contains("Icon pack installed")
        );
        assert!(controller.open_desktop_file(&pack).is_err());
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.history.snapshot(), history);
        assert_eq!(serde_json::to_value(&host.library).unwrap(), graph);
        assert_eq!(fs::read(&host.state_path).unwrap(), saved);
        assert!(
            dir.path()
                .join("icon-packs/owned.sdIconPack/icons/icon.svg")
                .is_file()
        );
    }
}
