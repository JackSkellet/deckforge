//! Built-in Timer. Duration is seconds, matching the original serialized contract.
use super::*;
use anyhow::ensure;
pub(super) const UUID: &str = "com.elgato.streamdeck.system.timer";
#[derive(Default)]
pub(super) struct Runtime {
    active: HashMap<String, Duration>,
    completed: HashSet<String>,
    next: Option<Duration>,
}
fn duration(settings: &Value) -> Result<u64> {
    let seconds = settings
        .get("duration")
        .map(|v| v.as_u64().context("Timer duration must be whole seconds"))
        .transpose()?
        .unwrap_or(30);
    ensure!(
        (1..=86400 * 7).contains(&seconds),
        "Timer duration must be 1 second to 7 days"
    );
    Ok(seconds)
}
fn sound(settings: &Value) -> Result<i64> {
    let kind = settings
        .get("actionIdx")
        .map(|v| v.as_i64().context("Timer sound must be a selection"))
        .transpose()?
        .unwrap_or(0);
    ensure!(
        matches!(kind, 0..=4 | 6),
        "Unknown timer sound; choose an available sound"
    );
    if kind == 4 {
        ensure!(
            settings["lastUserFile"]
                .as_str()
                .is_some_and(|p| !p.is_empty()),
            "Choose a custom timer sound"
        );
    }
    Ok(kind)
}
fn formatted(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}
pub(super) fn title(slot: &Slot) -> &str {
    if slot.uuid == UUID {
        slot.feedback["timerTitle"].as_str().unwrap_or(&slot.title)
    } else {
        &slot.title
    }
}
impl Host {
    pub(super) fn timer_input(
        &mut self,
        slot: &Slot,
        trigger: crate::system_actions::Trigger,
    ) -> Result<()> {
        if !matches!(trigger, crate::system_actions::Trigger::Up) {
            return Ok(());
        }
        let seconds = duration(&slot.settings)?;
        sound(&slot.settings)?;
        if let Some(audio) = &self.audio {
            audio.stop_context(&format!("timer:{}", slot.context))?;
        }
        if self.timers.active.remove(&slot.context).is_some() {
            self.system_status = "Timer cancelled".into();
        } else {
            self.timers.completed.remove(&slot.context);
            self.timers.active.insert(
                slot.context.clone(),
                self.multi_clock.elapsed() + Duration::from_secs(seconds),
            );
            self.system_status = "Timer started".into();
        }
        self.timers.next = Some(self.multi_clock.elapsed());
        self.multi_notify.notify_one();
        self.render()
    }
    pub(super) fn update_timer_feedback(&mut self) {
        let now = self.multi_clock.elapsed();
        let contexts: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| s.uuid == UUID)
            .map(|s| s.context.clone())
            .collect();
        for context in contexts {
            let Some(slot) = self.slots.iter_mut().find_map(|s| s.find_mut(&context)) else {
                continue;
            };
            let remaining = self
                .timers
                .active
                .get(&context)
                .map(|due| due.saturating_sub(now).as_millis().div_ceil(1000) as u64);
            let label = if let Some(seconds) = remaining {
                formatted(seconds)
            } else if self.timers.completed.contains(&context) {
                "Done".into()
            } else {
                duration(&slot.settings)
                    .map(formatted)
                    .unwrap_or_else(|_| "Set timer".into())
            };
            slot.feedback = json!({"timerTitle":label,"timerRunning":remaining.is_some()});
        }
    }
    fn timer_alarm(&mut self, slot: &Slot) -> Result<()> {
        let kind = sound(&slot.settings)?;
        if kind == 6 {
            return Ok(());
        }
        let path = if kind == 4 {
            if slot.settings.get("lastUserFile")
                == slot.raw.get("Settings").and_then(|s| s.get("lastUserFile"))
                && let Some(resource) = slot.raw.pointer("/Resources/file").and_then(Value::as_str)
            {
                self.soundboard_resource(&slot.context, resource)?
            } else {
                PathBuf::from(
                    slot.settings["lastUserFile"]
                        .as_str()
                        .context("Choose timer sound")?,
                )
            }
        } else {
            let directory = self
                .state_path
                .parent()
                .context("State directory unavailable")?
                .join("builtin-audio");
            fs::create_dir_all(&directory)?;
            let path = directory.join(format!("timer-{kind}.wav"));
            let bytes = alarm_wave(kind);
            if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
                fs::write(&path, bytes)?;
            }
            path
        };
        let config = crate::soundboard::Config::parse(
            &json!({"path":path,"actionType":2,"volume":slot.settings.get("linuxAlarmVolume").cloned().unwrap_or(json!(50)),"fadeType":0,"fadeLen":1,"outputType":"","linuxOutput":slot.settings.get("linuxOutput").cloned().unwrap_or(json!(""))}),
        )?;
        self.audio
            .get_or_insert_with(crate::audio_output::Worker::new)
            .play(&format!("timer:{}", slot.context), config)
    }
    pub(super) fn poll_timers(&mut self) {
        let now = self.multi_clock.elapsed();
        if self.timers.next.is_none_or(|due| now < due) {
            return;
        }
        let keys: Vec<_> = self.timers.active.keys().cloned().collect();
        for context in keys {
            let Some(slot) = self.slot_any(&context).filter(|s| s.uuid == UUID).cloned() else {
                self.timers.active.remove(&context);
                continue;
            };
            if self.timers.active[&context] <= now {
                self.timers.active.remove(&context);
                self.timers.completed.insert(context.clone());
                match self.timer_alarm(&slot) {
                    Ok(()) => self.system_status = "Timer complete".into(),
                    Err(e) => self.system_status = format!("Timer complete; alarm failed: {e:#}"),
                }
            }
        }
        self.timers.next = self
            .timers
            .active
            .values()
            .copied()
            .min()
            .map(|due| due.min(now + Duration::from_secs(1)));
        let _ = self.render();
    }
    pub(super) fn next_timer(&self) -> Option<Duration> {
        self.timers.next
    }
}
// Original, generated tones. Vendor audio files are not redistributed.
fn alarm_wave(kind: i64) -> Vec<u8> {
    let rate = 24000u32;
    let frames = rate / 2;
    let length = frames * 2;
    let mut out = Vec::with_capacity(44 + length as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + length).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&length.to_le_bytes());
    for n in 0..frames {
        let t = n as f64 / rate as f64;
        let fade = (t / 0.01).min(1.) * ((0.5 - t) / 0.08).clamp(0., 1.);
        let hz = 440. + kind as f64 * 110.;
        out.extend_from_slice(
            &(((t * hz * std::f64::consts::TAU).sin() * fade * 10000.).round() as i16)
                .to_le_bytes(),
        );
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_alarm_decodes_and_stops_only_its_own_voice() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("alarm.wav");
        fs::write(&file, alarm_wave(0)).unwrap();
        let config = || {
            crate::soundboard::Config::parse(&json!({"path":file,"actionType":2,"volume":50}))
                .unwrap()
        };
        let mut mixer = crate::soundboard::Mixer::new();
        mixer.trigger("timer:one", config()).unwrap();
        mixer.trigger("other", config()).unwrap();
        let mut samples = vec![0.; 960];
        mixer.mix("", &mut samples);
        assert!(samples.iter().any(|v| v.abs() > 0.01));
        mixer.stop_context("timer:one");
        assert!(!mixer.active().contains_key("timer:one"));
        assert!(mixer.active().contains_key("other"));
    }
    #[test]
    fn native_timer_settings_are_seconds_and_unknown_modes_are_rejected() {
        assert_eq!(duration(&json!({})).unwrap(), 30);
        assert_eq!(duration(&json!({"duration":90})).unwrap(), 90);
        assert_eq!(formatted(3661), "1:01:01");
        assert!(duration(&json!({"duration":-1})).is_err());
        assert_eq!(sound(&json!({"actionIdx":6})).unwrap(), 6);
        assert!(sound(&json!({"actionIdx":5})).is_err());
        assert!(sound(&json!({"actionIdx":4})).is_err());
    }
}
