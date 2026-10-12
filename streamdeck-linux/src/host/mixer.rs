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
        let current = self
            .mixer
            .as_ref()
            .map(Worker::snapshot)
            .unwrap_or_else(|| self.mixer_state.clone());
        let mut state = serde_json::to_value(&current).unwrap();
        state["pending"] = json!(self.mixer_config.enabled && current.config != self.mixer_config);
        state["applied_config"] = json!(current.config);
        state["config"] = json!(self.mixer_config);
        state["revision"] = json!(self.mixer_revision);
        state["controls_unsaved"] = json!(self.mixer_save_due.is_some());
        state["save_error"] = json!(self.mixer_save_error);
        state
    }

    pub(super) fn mixer_control(&mut self, request: Value) -> Result<()> {
        use crate::mixer::control::{self, Target};
        anyhow::ensure!(self.running, "Host has stopped");
        anyhow::ensure!(
            request["revision"].as_u64() == Some(self.mixer_revision),
            "Mixer routing changed elsewhere; reload before adjusting controls"
        );
        let mut next = self.mixer_config.clone();
        let target = &request["target"];
        let fade = request
            .get("fade_ms")
            .map(|value| {
                let ms = value
                    .as_u64()
                    .context("Fade must be a whole number of milliseconds")?;
                anyhow::ensure!(
                    ms <= 10_000,
                    "Fade must be between 0 and 10000 milliseconds"
                );
                Ok::<_, anyhow::Error>(ms as u32)
            })
            .transpose()?;
        let mut changes = BTreeMap::new();
        match target["kind"].as_str() {
            Some("channel") => {
                let id = target["channel"].as_str().context("Missing channel")?;
                let linked = request["linked"]
                    .as_bool()
                    .context("Linked must be a boolean")?;
                next.channels
                    .iter_mut()
                    .find(|channel| channel.id == id)
                    .context("Mixer channel no longer exists")?
                    .linked = linked;
            }
            Some("preferences") => next.fade_ms = fade.context("Missing fade duration")?,
            Some("monitor") => {
                anyhow::ensure!(
                    request.get("output").is_some() || request.get("mix").is_some(),
                    "Choose a listening device or mix"
                );
                if let Some(value) = request.get("output") {
                    next.monitoring.output = if value.is_null() {
                        None
                    } else {
                        Some(
                            value
                                .as_str()
                                .context("Listening output must be a device name or null")?
                                .to_owned(),
                        )
                    };
                }
                if let Some(value) = request.get("mix") {
                    next.monitoring.mix = if value.is_null() {
                        None
                    } else {
                        Some(
                            value
                                .as_str()
                                .context("Listening mix must be a mix ID or null")?
                                .to_owned(),
                        )
                    };
                }
            }
            _ => {
                let target: Target = serde_json::from_value(target.clone())?;
                let volume = request
                    .get("volume")
                    .map(|value| value.as_f64().context("Volume must be numeric"))
                    .transpose()?;
                let muted = request
                    .get("muted")
                    .map(|value| value.as_bool().context("Mute must be a boolean"))
                    .transpose()?;
                anyhow::ensure!(
                    volume.is_some() || muted.is_some(),
                    "Missing mixer control value"
                );
                for target in control::patch(&mut next, &target, volume, muted)? {
                    changes.insert(
                        target,
                        Duration::from_millis(u64::from(fade.unwrap_or(next.fade_ms))),
                    );
                }
            }
        }
        self.apply_mixer_controls(next, changes)
    }

    pub(super) fn apply_mixer_controls(
        &mut self,
        next: Config,
        changes: BTreeMap<crate::mixer::control::Target, Duration>,
    ) -> Result<()> {
        next.validate()?;
        let changed = next != self.mixer_config;
        if !changed && changes.is_empty() {
            return Ok(());
        }
        if next.enabled {
            self.ensure_mixer()?;
        }
        if let Some(worker) = &self.mixer {
            worker.submit_controls(next.clone(), changes)?;
        }
        if revision_changed(&self.mixer_config, &next) {
            self.mixer_revision = self.mixer_revision.wrapping_add(1);
        }
        self.mixer_config = next;
        if changed {
            self.mixer_save_due = Some(std::time::Instant::now() + Duration::from_millis(250));
        }
        if let Err(error) = self.render_all_devices() {
            self.log(format!("Mixer display update: {error:#}"));
        }
        Ok(())
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
        let worker = self.mixer.as_ref().unwrap();
        let submitted = if crate::mixer::control::same_routing(&previous, &config) {
            worker.submit_controls(config, BTreeMap::new())
        } else {
            worker.submit(config)
        };
        if let Err(error) = submitted {
            self.mixer_config = previous;
            self.persist()?;
            return Err(error);
        }
        if revision_changed(&previous, &self.mixer_config) {
            self.mixer_revision = self.mixer_revision.wrapping_add(1);
        }
        self.render_all_devices()
    }

    pub(super) fn poll_mixer(&mut self) {
        if self
            .mixer_save_due
            .is_some_and(|due| std::time::Instant::now() >= due)
            && let Err(error) = self.persist()
        {
            let message = format!("Could not save mixer controls: {error:#}");
            if self.mixer_save_error.as_ref() != Some(&message) {
                self.log(&message);
            }
            self.mixer_save_error = Some(message);
            self.mixer_save_due = Some(std::time::Instant::now() + Duration::from_secs(5));
        }
        let reports = self.mixer.as_ref().map(Worker::drain).unwrap_or_default();
        for state in reports {
            if state != self.mixer_state {
                if state.error != self.mixer_state.error
                    && let Some(error) = &state.error
                {
                    self.log(format!("Audio mixer: {error}"));
                }
                self.mixer_state = state;
                if let Err(error) = self.render_all_devices() {
                    self.log(format!("Mixer display update: {error:#}"));
                }
            }
        }
    }
}

fn revision_changed(a: &Config, b: &Config) -> bool {
    !crate::mixer::control::same_routing(a, b)
        || a.fade_ms != b.fade_ms
        || a.channels
            .iter()
            .map(|v| (&v.id, &v.name, v.linked))
            .ne(b.channels.iter().map(|v| (&v.id, &v.name, v.linked)))
        || a.mixes
            .iter()
            .map(|v| (&v.id, &v.name))
            .ne(b.mixes.iter().map(|v| (&v.id, &v.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_without_audio(root: &Path) -> Host {
        let mut host = super::super::tests::fixture(root);
        host.mixer_config.channels.push(crate::mixer::Channel {
            id: "music".into(),
            name: "Music".into(),
            ..Default::default()
        });
        host.mixer_config.routes = vec![
            crate::mixer::Route {
                channel: "music".into(),
                mix: "monitor".into(),
                volume: 40.,
                ..Default::default()
            },
            crate::mixer::Route {
                channel: "music".into(),
                mix: "stream".into(),
                volume: 70.,
                ..Default::default()
            },
        ];
        assert!(!host.mixer_config.enabled && host.mixer.is_none());
        host
    }
    fn route_request(revision: u64, mix: &str, changes: Value) -> Value {
        let mut request =
            json!({"revision":revision,"target":{"kind":"route","channel":"music","mix":mix}});
        request
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        request
    }

    #[test]
    fn live_disabled_controls_change_intent_without_creating_an_audio_worker() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        host.mixer_control(route_request(
            0,
            "monitor",
            json!({"volume":55,"muted":true,"fade_ms":750}),
        ))
        .unwrap();
        assert!(host.mixer.is_none());
        assert!(!host.mixer_config.enabled);
        assert_eq!(host.mixer_config.routes[0].volume, 55.);
        assert!(host.mixer_config.routes[0].muted);
        assert_eq!(host.mixer_revision, 0);
        assert!(host.mixer_save_due.is_some());
        let snapshot = host.mixer_snapshot();
        assert_eq!(snapshot["controls_unsaved"], true);
        assert_eq!(snapshot["pending"], false);
        assert_eq!(snapshot["active"], false);
        assert!(!host.state_path.exists()); // Debounce has not saved yet.
    }

    #[test]
    fn graph_and_link_revisions_reject_stale_commands_without_losing_latest_levels() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        host.mixer_control(route_request(0, "monitor", json!({"volume":45})))
            .unwrap();
        assert_eq!(host.mixer_revision, 0); // Repeated live faders need no graph revision refresh.
        host.mixer_control(
            json!({"revision":0,"target":{"kind":"channel","channel":"music"},"linked":true}),
        )
        .unwrap();
        assert_eq!(host.mixer_revision, 1);
        let linked = host.mixer_config.clone();
        assert!(
            host.mixer_control(route_request(0, "monitor", json!({"volume":5})))
                .is_err()
        );
        assert_eq!(host.mixer_config, linked);
        host.mixer_control(json!({"revision":1,"target":{"kind":"preferences"},"fade_ms":900}))
            .unwrap();
        assert_eq!(host.mixer_revision, 2);
        let mut changed_graph = host.mixer_config.clone();
        changed_graph.routes.pop();
        host.apply_mixer_controls(changed_graph.clone(), BTreeMap::new())
            .unwrap();
        assert_eq!(host.mixer_revision, 3);
        assert!(
            host.mixer_control(route_request(2, "monitor", json!({"muted":true})))
                .is_err()
        );
        assert_eq!(host.mixer_config, changed_graph);
        assert_eq!(host.mixer_config.routes[0].volume, 45.);
        assert!(host.mixer.is_none());
    }

    #[test]
    fn live_linked_volume_changes_both_existing_routes_but_mute_is_independent() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        host.mixer_control(
            json!({"revision":0,"target":{"kind":"channel","channel":"music"},"linked":true}),
        )
        .unwrap();
        host.mixer_control(route_request(
            1,
            "monitor",
            json!({"volume":80,"muted":true}),
        ))
        .unwrap();
        assert_eq!(host.mixer_config.routes[0].volume, 80.);
        assert_eq!(host.mixer_config.routes[1].volume, 100.);
        assert!(host.mixer_config.routes[0].muted);
        assert!(!host.mixer_config.routes[1].muted);
        host.mixer_control(route_request(1, "stream", json!({"muted":true})))
            .unwrap();
        assert_eq!(host.mixer_config.routes.len(), 2);
        assert_eq!(host.mixer_config.routes[0].volume, 80.);
        assert_eq!(host.mixer_config.routes[1].volume, 100.);
        assert_eq!(host.mixer_revision, 1);
        assert!(host.mixer.is_none());
    }

    #[test]
    fn failed_debounced_save_retries_latest_chosen_intent_without_audio() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        host.mixer_control(route_request(0, "monitor", json!({"volume":65})))
            .unwrap();
        // Make atomic rename fail without touching directory permissions or the user's filesystem.
        fs::create_dir(&host.state_path).unwrap();
        host.mixer_save_due = Some(std::time::Instant::now());
        host.poll_mixer();
        assert_eq!(host.mixer_config.routes[0].volume, 65.);
        assert!(host.mixer_save_error.is_some());
        assert!(host.mixer_save_due.unwrap() > std::time::Instant::now());
        host.mixer_control(route_request(
            0,
            "monitor",
            json!({"volume":68,"muted":true}),
        ))
        .unwrap();
        assert!(host.mixer_save_error.is_some()); // Keep failure visible until an actual successful save.
        fs::remove_dir(&host.state_path).unwrap();
        host.mixer_save_due = Some(std::time::Instant::now());
        host.poll_mixer();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.mixer.routes[0].volume, 68.);
        assert!(saved.mixer.routes[0].muted);
        assert_eq!(saved.mixer, host.mixer_config);
        assert!(host.mixer_save_due.is_none() && host.mixer_save_error.is_none());
        assert!(host.mixer.is_none());
    }

    #[test]
    fn malformed_live_request_is_atomic_and_cannot_assign_an_unknown_output() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        let before = host.mixer_config.clone();
        for request in [
            route_request(0, "monitor", json!({"volume":101,"muted":true})),
            route_request(0, "monitor", json!({"volume":20,"fade_ms":10001})),
            route_request(0, "monitor", json!({"volume":20,"muted":"true"})),
            route_request(0, "missing", json!({"volume":20})),
            json!({"revision":0,"target":{"kind":"output","mix":"monitor","output":"unassigned"},"volume":20}),
        ] {
            assert!(host.mixer_control(request).is_err());
            assert_eq!(host.mixer_config, before);
            assert_eq!(host.mixer_revision, 0);
            assert!(host.mixer_save_due.is_none());
            assert!(host.mixer.is_none());
        }
    }

    #[test]
    fn listening_preferences_save_while_inactive_without_starting_audio() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        host.mixer_control(
            json!({"revision":0,"target":{"kind":"monitor"},"output":"headphones","mix":"monitor"}),
        )
        .unwrap();
        assert!(host.mixer.is_none());
        assert!(!host.mixer_config.enabled);
        assert_eq!(
            host.mixer_config.monitoring.output.as_deref(),
            Some("headphones")
        );
        assert_eq!(host.mixer_revision, 1);
        assert!(
            host.mixer_control(json!({"revision":0,"target":{"kind":"monitor"},"mix":"stream"}))
                .is_err()
        );
        host.mixer_control(json!({"revision":1,"target":{"kind":"monitor"},"mix":"stream"}))
            .unwrap();
        assert_eq!(host.mixer_config.monitoring.mix.as_deref(), Some("stream"));
        assert_eq!(
            host.mixer_config.monitoring.output.as_deref(),
            Some("headphones")
        );
        host.mixer_control(json!({"revision":2,"target":{"kind":"monitor"},"mix":null}))
            .unwrap();
        assert!(host.mixer_config.monitoring.mix.is_none());
        assert_eq!(
            host.mixer_config.monitoring.output.as_deref(),
            Some("headphones")
        );
        host.persist().unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.mixer.monitoring, host.mixer_config.monitoring);
        assert!(host.mixer.is_none());
    }

    #[test]
    fn invalid_or_duplicate_listening_destination_cannot_change_intent() {
        let root = tempfile::tempdir().unwrap();
        let mut host = configured_without_audio(root.path());
        host.mixer_config.mixes[0].outputs = vec!["fixed-speakers".into()];
        let before = host.mixer_config.clone();
        for values in [
            json!({}),
            json!({"mix":false}),
            json!({"mix":"missing"}),
            json!({"output":"fixed-speakers"}),
            json!({"output":"@DEFAULT_SINK@"}),
            json!({"output":"deckforge.another.mix.monitor"}),
        ] {
            let mut request = json!({"revision":0,"target":{"kind":"monitor"}});
            request
                .as_object_mut()
                .unwrap()
                .extend(values.as_object().unwrap().clone());
            assert!(host.mixer_control(request).is_err());
            assert_eq!(host.mixer_config, before);
            assert_eq!(host.mixer_revision, 0);
            assert!(host.mixer.is_none());
        }
    }

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
