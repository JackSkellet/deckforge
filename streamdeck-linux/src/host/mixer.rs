//! Saved mixer intent is independent from asynchronous audio-server state.
use super::*;
use crate::mixer::{Config, Worker};
use sha2::{Digest, Sha256};
use std::os::unix::ffi::OsStrExt;

impl Host {
    pub(super) fn ensure_mixer(&mut self) -> Result<()> {
        anyhow::ensure!(self.running, "Host has stopped");
        if self.mixer.is_none() {
            let parent = self
                .state_path
                .parent()
                .context("Missing session directory")?;
            let canonical = parent
                .canonicalize()
                .context("Resolve mixer session directory")?;
            let digest = format!("{:x}", Sha256::digest(canonical.as_os_str().as_bytes()));
            self.mixer = Some(Worker::new(digest[..24].into(), self.mixer_config.clone())?);
        }
        Ok(())
    }

    pub(super) fn mixer_snapshot(&self) -> Value {
        let mut state = serde_json::to_value(&self.mixer_state).unwrap();
        state["pending"] = json!(self.mixer_state.config != self.mixer_config);
        state["applied_config"] = json!(self.mixer_state.config);
        state["config"] = json!(self.mixer_config);
        state
    }

    pub(super) fn set_mixer_config(
        &mut self,
        config: Config,
        expected: Option<&Config>,
    ) -> Result<()> {
        anyhow::ensure!(self.running, "Host has stopped");
        anyhow::ensure!(
            expected.is_none_or(|previous| previous == &self.mixer_config),
            "Mixer settings changed elsewhere; reload before applying"
        );
        config.validate()?;
        self.ensure_mixer()?;
        let previous = std::mem::replace(&mut self.mixer_config, config.clone());
        if let Err(error) = self.persist() {
            self.mixer_config = previous;
            return Err(error);
        }
        // Persist intent before touching the graph. The worker validates before
        // enqueueing and reports asynchronous failures through the snapshot.
        if let Err(error) = self.mixer.as_ref().unwrap().submit(config) {
            self.mixer_config = previous;
            self.persist()?;
            return Err(error);
        }
        self.render()
    }

    pub(super) fn poll_mixer(&mut self) {
        let reports = self.mixer.as_ref().map(Worker::drain).unwrap_or_default();
        for state in reports {
            if state != self.mixer_state {
                if state.error != self.mixer_state.error
                    && let Some(error) = &state.error
                {
                    self.log(format!("Audio mixer: {error}"));
                }
                self.mixer_state = state;
                if let Err(error) = self.render() {
                    self.log(format!("Mixer display update: {error:#}"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_or_invalid_config_cannot_start_worker_or_overwrite_saved_intent() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let original = host.mixer_config.clone();
        let mut stale = original.clone();
        stale.enabled = true;
        assert!(
            host.set_mixer_config(original.clone(), Some(&stale))
                .is_err()
        );
        let mut invalid = original.clone();
        invalid.mixes[0].volume = -1.0;
        assert!(host.set_mixer_config(invalid, Some(&original)).is_err());
        assert!(host.mixer.is_none());
        assert_eq!(host.mixer_config, original);
    }

    #[test]
    fn disabled_mixer_settings_roundtrip_without_worker_or_audio_access() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.mixer_config.channels.push(crate::mixer::Channel {
            id: "music".into(),
            name: "Music".into(),
            ..Default::default()
        });
        host.persist().unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.mixer, host.mixer_config);
        assert_eq!(host.mixer_snapshot()["active"], false);
        assert!(host.mixer.is_none());
    }
}
