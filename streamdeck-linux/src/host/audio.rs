//! Soundboard ownership and immutable imported media lookup.
use super::*;
use std::io::Write;

impl Host {
    pub(super) fn soundboard_action(&mut self, slot: &Slot) -> Result<()> {
        if slot.uuid == crate::audio_output::STOP {
            if let Some(worker) = &self.audio {
                worker.stop_all();
            }
            return Ok(());
        }
        let mut settings = slot.settings.clone();
        // A user-selected replacement path takes precedence over imported media.
        // Original archives often retain a foreign path despite carrying the clip.
        if slot.settings.get("path") == slot.raw.get("Settings").and_then(|v| v.get("path"))
            && let Some(resource) = slot.raw.pointer("/Resources/file").and_then(Value::as_str)
        {
            let path = self.soundboard_resource(&slot.context, resource)?;
            settings["path"] = json!(path);
        }
        let config = crate::soundboard::Config::parse(&settings)?;
        self.audio
            .get_or_insert_with(crate::audio_output::Worker::new)
            .play(&slot.context, config)?;
        self.system_status = "Soundboard action queued".into();
        Ok(())
    }
    pub(super) fn soundboard_resource(&mut self, context: &str, resource: &str) -> Result<PathBuf> {
        let slot = self
            .slot_any(context)
            .context("Audio resource owner missing")?;
        let origin = self
            .asset_origin(slot)
            .context("Audio resource archive missing")?;
        let source = origin.source;
        let id = uuid::Uuid::parse_str(&origin.profile)
            .context("Invalid resource owner identity")?
            .to_string();
        let resource_id = uuid::Uuid::parse_str(resource)
            .context("Invalid audio resource identity")?
            .to_string();
        if !self.profile_assets.contains_key(&source) {
            self.profile_assets
                .insert(source.clone(), profile::load_bundle(&source)?);
        }
        let (_name, bytes) = self.profile_assets[&source].resource(resource)?;
        let directory = self
            .state_path
            .parent()
            .context("State directory missing")?
            .join("audio-assets")
            .join(id);
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{resource_id}.media"));
        if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            let mut file = tempfile::NamedTempFile::new_in(directory)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            file.persist(&path).map_err(|error| error.error)?;
        }
        Ok(path)
    }
    pub(super) fn poll_audio(&mut self) {
        let reports = self
            .audio
            .as_ref()
            .map(|worker| worker.drain())
            .unwrap_or_default();
        for report in reports {
            match report {
                crate::audio_output::Report::Active(active) => self.audio_active = active,
                crate::audio_output::Report::Outputs(outputs) => self.audio_outputs = outputs,
                crate::audio_output::Report::Status(message) => {
                    self.system_status = message.clone();
                    self.log(message);
                }
            }
        }
    }
}
