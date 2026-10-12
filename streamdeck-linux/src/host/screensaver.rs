//! Software screensavers preserve the live profile underneath the display layer.
use super::*;
use crate::{screensaver::Animation, session_activity};
use sha2::{Digest, Sha256};

pub(super) const UUID: &str = "com.elgato.streamdeck.system.sleep";
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Preferences {
    pub after_minutes: u32,
    pub image: Option<PathBuf>,
    pub disable_on_lock: bool,
}
impl Preferences {
    pub(super) fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            matches!(self.after_minutes, 0 | 5 | 15 | 30 | 60 | 120),
            "Choose Never,5,15,30,60 or120 minutes"
        );
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Runtime {
    pub active: bool,
    pub generation: u64,
    pub locked: bool,
    pub started: Duration,
    pub activity: Duration,
    pub reason: String,
    pub animation: Option<Arc<Animation>>,
    pub loaded: Option<PathBuf>,
    pub error: Option<String>,
    pub preview: String,
    pub normal_signature: Option<u64>,
    pub rendered_frame: Option<(usize, bool)>,
}
pub(super) fn adopt_preferences(
    preferences: &mut HashMap<String, Preferences>,
    previous: Option<&str>,
    device: &str,
) {
    if !preferences.contains_key(device)
        && let Some(previous) = previous.filter(|id| id.starts_with("offline-streamdeck-plus-"))
        && let Some(value) = preferences.remove(previous)
    {
        preferences.insert(device.into(), value);
    }
}
pub(super) struct Prepared {
    pub previous: Preferences,
    pub preferences: Preferences,
    pub animation: Option<Arc<Animation>>,
}
/// Decode and copy artwork before acquiring the host mutex.
pub(super) fn prepare(
    request: &Value,
    previous: &Preferences,
    state_path: &Path,
) -> Result<Prepared> {
    anyhow::ensure!(
        request
            .as_object()
            .is_some_and(|values| values.keys().all(|key| matches!(
                key.as_str(),
                "command" | "after_minutes" | "disable_on_lock" | "image" | "device"
            ))),
        "Unknown screensaver setting"
    );
    let mut preferences = previous.clone();
    preferences.after_minutes = request["after_minutes"]
        .as_u64()
        .and_then(|n| n.try_into().ok())
        .context("Sleep interval missing or invalid")?;
    if let Some(value) = request.get("disable_on_lock") {
        preferences.disable_on_lock = value
            .as_bool()
            .context("Lock preference must be true or false")?;
    }
    preferences.validate()?;
    let mut animation = None;
    if let Some(path) = request.get("image") {
        if path.is_null() || path.as_str() == Some("") {
            preferences.image = None;
        } else {
            let path = Path::new(
                path.as_str()
                    .context("Screensaver image path must be text")?,
            );
            let (name, bytes) = crate::resources::read_regular(path)?;
            let decoded = Animation::from_bytes(&bytes)?;
            let cache = state_path
                .parent()
                .context("Session directory unavailable")?
                .join("screensaver-cache");
            let stored = crate::resources::materialize(
                &cache,
                "images",
                &format!("{:x}", Sha256::digest(&bytes)),
                &name,
                &bytes,
            )?;
            preferences.image = Some(stored);
            animation = Some(Arc::new(decoded));
        }
    }
    Ok(Prepared {
        previous: previous.clone(),
        preferences,
        animation,
    })
}
impl Host {
    pub(super) fn saver_preferences(&self) -> Preferences {
        self.screensavers
            .get(&self.device)
            .cloned()
            .unwrap_or_default()
    }
    pub(super) fn saver_animation(&mut self) -> Result<Arc<Animation>> {
        let path = self.saver_preferences().image;
        if self.saver.animation.is_none() || self.saver.loaded != path {
            let loaded = path
                .as_deref()
                .map(Animation::load)
                .unwrap_or_else(Animation::default_logo);
            self.saver.animation = Some(Arc::new(match loaded {
                Ok(animation) => {
                    self.saver.error = None;
                    animation
                }
                Err(error) => {
                    self.saver.error = Some(format!("Screensaver: {error:#}"));
                    Animation::default_logo()?
                }
            }));
            self.saver.loaded = path;
        }
        Ok(self.saver.animation.as_ref().unwrap().clone())
    }
    #[cfg(test)]
    pub(super) fn configure_saver(&mut self, request: &Value) -> Result<()> {
        let prepared = prepare(request, &self.saver_preferences(), &self.state_path)?;
        self.apply_saver(prepared)
    }
    pub(super) fn apply_saver(&mut self, prepared: Prepared) -> Result<()> {
        anyhow::ensure!(
            self.model == Model::Plus,
            "Software screensaver layout is currently supported on Stream Deck + only"
        );
        anyhow::ensure!(
            self.saver_preferences() == prepared.previous,
            "Screensaver settings changed while the image was loading; try again"
        );
        let previous_lock_policy = prepared.previous.disable_on_lock;
        let preferences = prepared.preferences;
        let animation = prepared.animation;
        let old = self
            .screensavers
            .insert(self.device.clone(), preferences.clone());
        if let Err(error) = self.persist() {
            match old {
                Some(old) => {
                    self.screensavers.insert(self.device.clone(), old);
                }
                None => {
                    self.screensavers.remove(&self.device);
                }
            }
            return Err(error);
        }
        if animation.is_some() || self.saver.loaded != preferences.image {
            self.saver.animation = animation;
            self.saver.loaded = preferences.image.clone();
        }
        self.saver.activity = self.multi_clock.elapsed();
        self.saver.error = None;
        self.send_device_controls();
        self.multi_notify.notify_one();
        if self.saver.locked {
            if preferences.disable_on_lock && self.saver.reason == "lock" {
                self.wake_device()?;
            } else if !preferences.disable_on_lock {
                if previous_lock_policy && self.saver.active {
                    self.saver.generation = self.saver.generation.saturating_add(1);
                    self.saver.reason = "lock".into();
                }
                self.sleep_device("lock")?;
            }
        }
        if self.saver.active {
            self.render()?;
        }
        Ok(())
    }
    pub(super) fn sleep_device(&mut self, reason: &str) -> Result<()> {
        anyhow::ensure!(
            self.model == Model::Plus,
            "Software screensaver layout is currently supported on Stream Deck + only"
        );
        if self.saver.active {
            return Ok(());
        }
        self.saver_animation()?;
        self.saver.generation = self.saver.generation.saturating_add(1);
        self.saver.active = true;
        self.saver.reason = reason.into();
        self.saver.started = self.multi_clock.elapsed();
        self.release_device_holds();
        self.send_device_controls();
        self.render()?;
        self.multi_notify.notify_one();
        Ok(())
    }
    pub(super) fn wake_device(&mut self) -> Result<()> {
        if self.saver.locked && !self.saver_preferences().disable_on_lock {
            return Ok(());
        }
        if !self.saver.active {
            return Ok(());
        }
        self.saver.active = false;
        self.saver.reason.clear();
        self.saver.preview.clear();
        self.saver.activity = self.multi_clock.elapsed();
        if let Some(hardware) = &self.hardware {
            let _ = hardware.send(Draw::Screensaver {
                generation: self.saver.generation,
                image: None,
                wake_on_input: true,
            });
        }
        self.send_device_controls();
        self.render()?;
        self.multi_notify.notify_one();
        Ok(())
    }
    pub(super) fn saver_input(&mut self, input: &InputEvent) -> Result<bool> {
        self.saver.activity = self.multi_clock.elapsed();
        if !self.saver.active {
            return Ok(false);
        }
        // Diagnostic input has no HID decoder; consume its matching release too.
        if self.simulate {
            match input {
                InputEvent::Key {
                    index,
                    pressed: true,
                } => {
                    self.released.insert(HeldControl::Key(*index));
                }
                InputEvent::DialPress {
                    index,
                    pressed: true,
                } => {
                    self.released.insert(HeldControl::Dial(*index));
                }
                _ => {}
            }
        }
        self.wake_device()?;
        Ok(true)
    }
    pub(super) fn render_saver(&mut self) -> Result<()> {
        if !self.saver.active {
            return Ok(());
        }
        let animation = self.saver_animation()?;
        let elapsed = if self.saver.locked {
            Duration::ZERO
        } else {
            self.multi_clock
                .elapsed()
                .saturating_sub(self.saver.started)
        };
        let image = animation.frame(elapsed);
        for slot in &mut self.slots {
            let picture = if slot.controller == "Keypad" {
                crate::screensaver::key_crop(image, slot.index)?
            } else {
                crate::screensaver::strip_crop(image).crop_imm(
                    u32::from(slot.index) * 200,
                    0,
                    200,
                    100,
                )
            };
            let mut output = Cursor::new(Vec::new());
            picture.write_to(&mut output, image::ImageFormat::Png)?;
            slot.preview = STANDARD.encode(output.into_inner());
        }
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, image::ImageFormat::Png)?;
        self.saver.preview = STANDARD.encode(output.into_inner());
        if self
            .device_config
            .as_ref()
            .is_some_and(|config| config.serial.is_some())
            && let Some(hardware) = &self.hardware
        {
            let _ = hardware.send(Draw::Screensaver {
                generation: self.saver.generation,
                image: Some(image.clone()),
                wake_on_input: !self.saver.locked || self.saver_preferences().disable_on_lock,
            });
        }
        self.saver.rendered_frame = Some((image as *const _ as usize, self.saver.locked));
        Ok(())
    }
    pub(super) fn next_saver(&self) -> Option<Duration> {
        if !self.connected {
            return None;
        }
        if self.saver.active {
            if self.saver.locked {
                return None;
            }
            let now = self.multi_clock.elapsed();
            self.saver
                .animation
                .as_ref()?
                .next_frame_in(now.saturating_sub(self.saver.started))
                .map(|delay| now + delay)
        } else {
            let minutes = self.saver_preferences().after_minutes;
            (minutes > 0)
                .then_some(self.saver.activity + Duration::from_secs(u64::from(minutes) * 60))
        }
    }
    pub(super) fn poll_saver(&mut self) {
        if !self.connected {
            return;
        }
        let result = if self.saver.active {
            let elapsed = if self.saver.locked {
                Duration::ZERO
            } else {
                self.multi_clock
                    .elapsed()
                    .saturating_sub(self.saver.started)
            };
            let changed = self.saver.animation.as_ref().is_none_or(|animation| {
                self.saver.rendered_frame
                    != Some((
                        animation.frame(elapsed) as *const _ as usize,
                        self.saver.locked,
                    ))
            });
            if changed { self.render_saver() } else { Ok(()) }
        } else if self
            .next_saver()
            .is_some_and(|due| due <= self.multi_clock.elapsed())
        {
            self.sleep_device("timer")
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.saver.error = Some(format!("Screensaver: {error:#}"));
        }
    }
    pub(super) fn session_update(&mut self, state: session_activity::State) -> Result<()> {
        if self.model != Model::Plus {
            if let Some(locked) = state.locked {
                self.saver.locked = locked;
            }
            self.session_state = Some(state);
            return Ok(());
        }
        if let Some(locked) = state.locked {
            let changed = self.saver.locked != locked;
            self.saver.locked = locked;
            if changed {
                if locked && !self.saver_preferences().disable_on_lock {
                    if self.saver.active {
                        self.saver.generation = self.saver.generation.saturating_add(1);
                        self.saver.reason = "lock".into();
                    }
                    self.sleep_device("lock")?;
                    self.render_saver()?;
                } else if !locked {
                    self.wake_device()?;
                }
            }
        }
        self.session_state = Some(state);
        Ok(())
    }
    pub(super) fn poll_session(&mut self) {
        let reports = self
            .session_monitor
            .as_ref()
            .map(|monitor| monitor.drain())
            .unwrap_or_default();
        for state in reports {
            for device in self.device_ids() {
                let _ = self.with_device(&device, |host| {
                    if let Err(error) = host.session_update(state.clone()) {
                        host.saver.error = Some(format!("Session: {error:#}"));
                    }
                    Ok(())
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host(root: &Path) -> Host {
        crate::host::tests::fixture(root)
    }
    #[test]
    fn software_timer_uses_device_activity_deadline_and_never_zero() {
        let root = tempfile::tempdir().unwrap();
        let mut host = host(root.path());
        assert!(host.next_saver().is_none());
        host.screensavers.insert(
            host.device.clone(),
            Preferences {
                after_minutes: 5,
                ..Default::default()
            },
        );
        assert_eq!(host.next_saver(), Some(Duration::from_secs(300)));
        host.multi_clock = std::time::Instant::now() - Duration::from_secs(299);
        host.poll_saver();
        assert!(!host.saver.active);
        host.device_event(DeviceEvent::Activity).unwrap();
        host.multi_clock = std::time::Instant::now() - Duration::from_secs(301);
        host.poll_saver();
        assert!(!host.saver.active);
        host.multi_clock = std::time::Instant::now() - Duration::from_secs(601);
        host.poll_saver();
        assert!(host.saver.active);
        assert_eq!(host.saver.reason, "timer");
        host.wake_device().unwrap();
        assert!(!host.saver.active);
        assert!(host.next_saver().unwrap() > Duration::from_secs(900));
    }
    #[test]
    fn lock_rearms_generation_and_stale_wake_cannot_uncover_profile() {
        let root = tempfile::tempdir().unwrap();
        let mut host = host(root.path());
        host.sleep_device("manual").unwrap();
        let manual = host.saver.generation;
        host.session_update(session_activity::State {
            idle_ms: None,
            locked: Some(true),
            source: "fixture".into(),
            error: None,
        })
        .unwrap();
        assert!(host.saver.generation > manual);
        host.device_event(DeviceEvent::Wake { generation: manual })
            .unwrap();
        assert!(host.saver.active);
        host.session_update(session_activity::State {
            idle_ms: None,
            locked: None,
            source: "lost".into(),
            error: Some("Disconnected".into()),
        })
        .unwrap();
        host.wake_device().unwrap();
        assert!(host.saver.active);
        host.session_update(session_activity::State {
            idle_ms: None,
            locked: Some(false),
            source: "fixture".into(),
            error: None,
        })
        .unwrap();
        assert!(!host.saver.active);
    }
    #[test]
    fn offline_screensaver_does_not_reach_an_unidentified_device() {
        let root = tempfile::tempdir().unwrap();
        let mut host = host(root.path());
        host.simulate = false;
        host.device_config = Some(device::Config::select(None, None));
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        host.sleep_device("manual").unwrap();
        assert!(
            !receiver
                .try_iter()
                .any(|draw| matches!(draw, Draw::Screensaver { image: Some(_), .. }))
        );
    }
    #[test]
    fn offline_preferences_adopt_at_startup_but_never_replace_known_serial() {
        let mut values = HashMap::new();
        values.insert(
            "offline-streamdeck-plus-test".into(),
            Preferences {
                after_minutes: 5,
                ..Default::default()
            },
        );
        adopt_preferences(&mut values, Some("offline-streamdeck-plus-test"), "real-a");
        assert_eq!(values["real-a"].after_minutes, 5);
        assert!(!values.contains_key("offline-streamdeck-plus-test"));
        values.insert(
            "offline-streamdeck-plus-test".into(),
            Preferences {
                after_minutes: 30,
                ..Default::default()
            },
        );
        adopt_preferences(&mut values, Some("offline-streamdeck-plus-test"), "real-a");
        assert_eq!(values["real-a"].after_minutes, 5);
    }
    #[test]
    fn enabling_lock_policy_while_already_locked_rearms_active_generation() {
        let root = tempfile::tempdir().unwrap();
        let mut host = host(root.path());
        host.screensavers.insert(
            host.device.clone(),
            Preferences {
                disable_on_lock: true,
                ..Default::default()
            },
        );
        host.saver.locked = true;
        host.sleep_device("manual").unwrap();
        let old = host.saver.generation;
        host.configure_saver(
            &json!({"command":"screensaver","after_minutes":0,"disable_on_lock":false}),
        )
        .unwrap();
        assert!(host.saver.generation > old);
        host.device_event(DeviceEvent::Wake { generation: old })
            .unwrap();
        assert!(host.saver.active);
    }
}
