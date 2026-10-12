//! Original opt-in audio matrix using PulseAudio's generic graph primitives.
//! No Wave Link settings/curve compatibility is claimed. Percentages represent
//! linear amplitude. System default devices are never modified.
#[path = "mixer/backend.rs"]
mod backend;
#[path = "mixer/control.rs"]
pub mod control;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub fade_ms: u32,
    pub enabled: bool,
    pub channels: Vec<Channel>,
    pub mixes: Vec<Mix>,
    pub routes: Vec<Route>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            fade_ms: 0,
            enabled: false,
            channels: vec![],
            mixes: vec![
                Mix {
                    id: "monitor".into(),
                    name: "Monitor".into(),
                    ..Mix::default()
                },
                Mix {
                    id: "stream".into(),
                    name: "Stream".into(),
                    ..Mix::default()
                },
            ],
            routes: vec![],
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Channel {
    pub linked: bool,
    pub id: String,
    pub name: String,
    pub apps: Vec<AppSelector>,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mix {
    pub output_controls: BTreeMap<String, OutputControl>,
    pub id: String,
    pub name: String,
    pub outputs: Vec<String>,
    pub volume: f64,
    pub muted: bool,
}
impl Default for Mix {
    fn default() -> Self {
        Self {
            output_controls: BTreeMap::new(),
            id: String::new(),
            name: String::new(),
            outputs: vec![],
            volume: 100.0,
            muted: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OutputControl {
    pub volume: f64,
    pub muted: bool,
}
impl Default for OutputControl {
    fn default() -> Self {
        Self {
            volume: 100.0,
            muted: false,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Level {
    pub peak: f32,
    pub available: bool,
}
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Levels {
    pub channels: BTreeMap<String, Level>,
    pub mixes: BTreeMap<String, Level>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum AppSelector {
    ApplicationId(String),
    ProcessBinary(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Route {
    pub channel: String,
    pub mix: String,
    pub volume: f64,
    pub muted: bool,
}
impl Default for Route {
    fn default() -> Self {
        Self {
            channel: String::new(),
            mix: String::new(),
            volume: 100.0,
            muted: false,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Device {
    pub name: String,
    pub description: String,
    pub owned: bool,
    pub monitor: bool,
}
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct App {
    pub index: u32,
    pub name: String,
    pub application_id: Option<String>,
    pub process_binary: Option<String>,
    pub sink: String,
}
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Snapshot {
    pub levels: Levels,
    pub fading: bool,
    pub config: Config,
    pub connected: bool,
    pub active: bool,
    pub sinks: Vec<Device>,
    pub sources: Vec<Device>,
    pub apps: Vec<App>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.fade_ms <= 10_000,
            "Fade duration must be 0–10000 milliseconds"
        );
        ensure!(
            self.channels.len() <= 12,
            "At most 12 mixer channels are supported"
        );
        ensure!(self.mixes.len() <= 5, "At most five mixes are supported");
        let mut channels = BTreeSet::new();
        let mut mixes = BTreeSet::new();
        let mut selectors = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for c in &self.channels {
            valid_id(&c.id)?;
            valid_label(&c.name)?;
            ensure!(channels.insert(&c.id), "Duplicate channel ID");
            for a in &c.apps {
                let v = match a {
                    AppSelector::ApplicationId(v) | AppSelector::ProcessBinary(v) => v,
                };
                valid_external(v)?;
                ensure!(
                    selectors.insert(a),
                    "An app selector belongs to more than one channel"
                );
            }
            for source in &c.sources {
                valid_external(source)?;
                ensure!(
                    !source.starts_with('@'),
                    "Select an exact source, not a default-device alias"
                );
                ensure!(
                    !source.starts_with("deckforge."),
                    "Mixer outputs cannot feed mixer inputs"
                );
                ensure!(
                    sources.insert(source),
                    "A source belongs to more than one channel"
                );
            }
        }
        for m in &self.mixes {
            valid_id(&m.id)?;
            valid_label(&m.name)?;
            valid_volume(m.volume)?;
            ensure!(
                m.output_controls.len() <= 128,
                "Too many saved output controls"
            );
            for (output, control) in &m.output_controls {
                valid_external(output)?;
                ensure!(
                    !output.starts_with('@') && !output.starts_with("deckforge."),
                    "Output controls require an exact external output"
                );
                valid_volume(control.volume)?;
            }
            ensure!(mixes.insert(&m.id), "Duplicate mix ID");
            let mut outputs = BTreeSet::new();
            for output in &m.outputs {
                valid_external(output)?;
                ensure!(
                    !output.starts_with('@'),
                    "Select an exact output, not a default-device alias"
                );
                ensure!(
                    !output.starts_with("deckforge."),
                    "Mixer outputs cannot target mixer sinks"
                );
                ensure!(outputs.insert(output), "Duplicate output assignment");
            }
        }
        let mut routes = BTreeSet::new();
        ensure!(self.routes.len() <= 60, "Too many mixer routes");
        for r in &self.routes {
            ensure!(
                channels.contains(&r.channel) && mixes.contains(&r.mix),
                "Route refers to an unknown channel or mix"
            );
            ensure!(routes.insert((&r.channel, &r.mix)), "Duplicate mixer route");
            valid_volume(r.volume)?;
        }
        Ok(())
    }
}
fn valid_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 40
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
        "IDs must contain 1–40 ASCII letters, digits, hyphens or underscores"
    );
    Ok(())
}
fn valid_label(label: &str) -> Result<()> {
    ensure!(
        !label.is_empty() && label.len() <= 160 && !label.chars().any(char::is_control),
        "Invalid mixer display name"
    );
    Ok(())
}
fn valid_external(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control),
        "Invalid audio identity"
    );
    Ok(())
}
fn valid_volume(value: f64) -> Result<()> {
    ensure!(
        value.is_finite() && (0.0..=100.0).contains(&value),
        "Mixer volume must be 0–100 percent"
    );
    Ok(())
}
pub fn channel_sink(namespace: &str, id: &str) -> String {
    format!("deckforge.{namespace}.channel.{id}")
}
pub fn mix_sink(namespace: &str, id: &str) -> String {
    format!("deckforge.{namespace}.mix.{id}")
}
pub fn mix_source(namespace: &str, id: &str) -> String {
    format!("deckforge.{namespace}.source.{id}")
}

struct Update {
    config: Config,
    changes: BTreeMap<control::Target, Duration>,
    immediate: bool,
}

pub struct Worker {
    pending: Arc<Mutex<Option<Update>>>,
    state: Arc<Mutex<Snapshot>>,
    wake: mpsc::SyncSender<()>,
    reports: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new(namespace: String, config: Config) -> Result<Self> {
        valid_id(&namespace)?;
        config.validate()?;
        let pending = Arc::new(Mutex::new(Some(Update {
            config,
            changes: BTreeMap::new(),
            immediate: true,
        })));
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (wake, commands) = mpsc::sync_channel(1);
        let (reporter, reports) = mpsc::sync_channel(4);
        let desired = pending.clone();
        let shared = state.clone();
        let stopped = stop.clone();
        let thread =
            thread::Builder::new()
                .name("deckforge-mixer".into())
                .spawn(move || {
                    let clock = Instant::now();
                    let mut timeline = control::Fades::new(Config::default());
                    let mut engine = backend::Engine::new(namespace);
                    let mut accepted = Config::default();
                    let mut submitted = None;
                    let mut inventory_at = Instant::now();
                    let mut snapshot = Snapshot::default();
                    let mut last_report = None;
                    while !stopped.load(Ordering::Acquire) {
                        let update = desired.lock().unwrap().take();
                        let requested = update.is_some();
                        if let Some(update) = update
                            && let Err(error) = timeline.submit(
                                update.config,
                                update.changes,
                                update.immediate,
                                clock.elapsed(),
                            )
                        {
                            snapshot.error = Some(error.to_string());
                        }
                        let current = timeline.sample(clock.elapsed());
                        let due = Instant::now() >= inventory_at;
                        if due
                            || (submitted.as_ref() != Some(&current)
                                && (snapshot.error.is_none() || requested))
                        {
                            snapshot = if !due {
                                engine.update_controls(&current, &stopped)
                            } else {
                                None
                            }
                            .unwrap_or_else(|| {
                                inventory_at = Instant::now() + Duration::from_millis(500);
                                engine.update(&current, &stopped)
                            });
                            if snapshot.error.is_none() {
                                accepted = current.clone();
                            } else {
                                snapshot.config = accepted.clone();
                            }
                            submitted = Some(current);
                        }
                        snapshot.fading = timeline.active() && snapshot.error.is_none();
                        snapshot.levels = engine.poll_levels(&stopped);
                        *shared.lock().unwrap() = snapshot.clone();
                        // Meter-only changes update the UI's shared snapshot without
                        // redrawing every physical key or filling the report queue.
                        let mut report = snapshot.clone();
                        report.levels = Levels::default();
                        if last_report.as_ref() != Some(&report) {
                            let _ = reporter.try_send(());
                            last_report = Some(report);
                        }
                        if let Err(mpsc::RecvTimeoutError::Disconnected) = commands.recv_timeout(
                            Duration::from_millis(if timeline.target().enabled { 20 } else { 500 }),
                        ) {
                            break;
                        }
                    }
                    engine.shutdown();
                })?;
        Ok(Self {
            pending,
            state,
            wake,
            reports,
            stop,
            thread: Some(thread),
        })
    }
    pub fn submit(&self, config: Config) -> Result<()> {
        self.enqueue(config, BTreeMap::new(), true)
    }
    pub fn submit_controls(
        &self,
        config: Config,
        changes: BTreeMap<control::Target, Duration>,
    ) -> Result<()> {
        let immediate = !config.enabled;
        self.enqueue(config, changes, immediate)
    }
    fn enqueue(
        &self,
        config: Config,
        changes: BTreeMap<control::Target, Duration>,
        immediate: bool,
    ) -> Result<()> {
        config.validate()?;
        for target in changes.keys() {
            control::read(&config, target)?;
        }
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "Mixer worker has stopped"
        );
        let mut pending = self.pending.lock().unwrap();
        if immediate {
            *pending = Some(Update {
                config,
                changes,
                immediate,
            });
        } else if let Some(previous) = pending.as_mut() {
            // A configuration edit with no requested fade applies its changed
            // levels immediately. Keep queued durations only for untouched levels.
            previous.changes.retain(|target, _| {
                changes.contains_key(target)
                    || control::read(&previous.config, target)
                        .ok()
                        .map(|v| v.volume)
                        == control::read(&config, target).ok().map(|v| v.volume)
            });
            previous.config = config;
            previous.changes.extend(changes);
            previous
                .changes
                .retain(|target, _| control::read(&previous.config, target).is_ok());
        } else {
            *pending = Some(Update {
                config,
                changes,
                immediate,
            });
        }
        drop(pending);
        let _ = self.wake.try_send(());
        Ok(())
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    pub fn drain(&self) -> Vec<Snapshot> {
        if self.reports.try_iter().count() > 0 {
            vec![self.snapshot()]
        } else {
            vec![]
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Config {
        Config {
            enabled: true,
            channels: vec![Channel {
                id: "music".into(),
                name: "Music".into(),
                ..Channel::default()
            }],
            routes: vec![Route {
                channel: "music".into(),
                mix: "monitor".into(),
                volume: 50.0,
                muted: false,
            }],
            ..Config::default()
        }
    }
    #[test]
    fn pending_controls_preserve_metadata_ramps_but_clear_overridden_levels() {
        let (wake, _commands) = mpsc::sync_channel(1);
        let (_reporter, reports) = mpsc::channel();
        let worker = Worker {
            pending: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(Snapshot::default())),
            wake,
            reports,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let target = control::Target::Route {
            channel: "music".into(),
            mix: "monitor".into(),
        };
        let mut config = sample();
        config.routes[0].volume = 20.0;
        worker
            .submit_controls(
                config.clone(),
                BTreeMap::from([(target.clone(), Duration::from_secs(1))]),
            )
            .unwrap();
        config.channels[0].name = "Renamed".into();
        config.routes[0].muted = true;
        worker
            .submit_controls(config.clone(), BTreeMap::new())
            .unwrap();
        assert_eq!(
            worker
                .pending
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .changes
                .len(),
            1
        );
        config.routes[0].volume = 50.0;
        worker
            .submit_controls(config.clone(), BTreeMap::new())
            .unwrap();
        assert!(
            worker
                .pending
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .changes
                .is_empty()
        );
        config.enabled = false;
        worker
            .submit_controls(config, BTreeMap::from([(target, Duration::from_secs(1))]))
            .unwrap();
        assert!(worker.pending.lock().unwrap().as_ref().unwrap().immediate);
    }

    #[test]
    fn default_is_opt_in_and_has_no_members() {
        let c = Config::default();
        c.validate().unwrap();
        assert!(!c.enabled);
        assert!(c.channels.is_empty());
        assert!(c.mixes.iter().all(|m| m.outputs.is_empty()));
    }
    #[test]
    fn reject_ambiguous_members_and_feedback() {
        let mut c = sample();
        c.channels[0].sources = vec!["deckforge.other.source.stream".into()];
        assert!(c.validate().is_err());
        c.channels[0].sources.clear();
        c.channels[0].apps = vec![
            AppSelector::ProcessBinary("browser".into()),
            AppSelector::ProcessBinary("browser".into()),
        ];
        assert!(c.validate().is_err());
    }
    #[test]
    fn bounds_and_missing_routes() {
        let mut c = sample();
        c.routes[0].volume = f64::NAN;
        assert!(c.validate().is_err());
        c.routes[0].volume = 50.;
        c.routes[0].mix = "missing".into();
        assert!(c.validate().is_err());
        c.routes.clear();
        for i in 0..6 {
            c.mixes.push(Mix {
                id: format!("mix{i}"),
                name: "Mix".into(),
                ..Mix::default()
            });
        }
        assert!(c.validate().is_err());
    }
    #[test]
    fn names_with_spaces_are_valid_but_control_bytes_are_not() {
        let mut c = sample();
        c.mixes[0].outputs = vec!["Microphone FX".into()];
        c.validate().unwrap();
        c.mixes[0].outputs[0].push('\n');
        assert!(c.validate().is_err());
    }
    #[test]
    fn app_and_output_lists_have_no_small_fixed_cap() {
        let mut c = sample();
        c.channels[0].apps = (0..64)
            .map(|i| AppSelector::ProcessBinary(format!("app-{i}")))
            .collect();
        c.channels[0].sources = (0..16).map(|i| format!("source-{i}")).collect();
        c.mixes[0].outputs = (0..16).map(|i| format!("output-{i}")).collect();
        c.validate().unwrap();
    }
}
