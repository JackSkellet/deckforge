//! Multi Action data and an incremental, clock-driven scheduler.
//!
//! The original action model stores groups under Actions[].Actions[], per-child
//! MultiActionDelays.Delay1/Delay2, OverrideState, and explicit Settings.delay.
//! Unknown raw fields remain untouched. Random choice, switch state, plugin events,
//! manifest timing defaults and OS actions belong to the caller.
//!
//! A running start is idempotently ignored. The host implements native repeated
//! key-down cancellation by calling cancel. Cancellation releases held children
//! as a Linux safety policy and never emits Finished or advances switch state.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

pub const ROUTINE: &str = "com.elgato.streamdeck.multiactions.routine";
pub const SWITCH: &str = "com.elgato.streamdeck.multiactions.routine2";
pub const RANDOM: &str = "com.elgato.streamdeck.multiactions.random";
pub const DELAY: &str = "com.elgato.streamdeck.multiactions.delay";
const DEFAULT_TIMING_MS: u64 = 100;
const MAX_STEPS: usize = 512;
const MAX_TIMER_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_RUNNING: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Scoped to the group. Explicit ActionIDs survive reordering. Missing IDs
    /// fall back to a group/index identity, stable only until that entry is moved.
    pub id: String,
    pub uuid: String,
    pub raw: Value,
    pub press_ms: u64,
    pub gap_ms: u64,
    pub delay_ms: Option<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub uuid: String,
    pub raw: Value,
    pub groups: Vec<Vec<Entry>>,
}
impl Sequence {
    pub fn parse(uuid: &str, raw: &Value) -> Result<Self> {
        ensure!(
            matches!(uuid, ROUTINE | SWITCH | RANDOM),
            "Unsupported Multi Action container UUID"
        );
        ensure!(raw.is_object(), "Multi Action must be an object");
        if let Some(actual) = raw.get("UUID") {
            ensure!(
                actual.as_str() == Some(uuid),
                "Multi Action UUID does not match its container"
            );
        }
        let raw_groups = raw["Actions"]
            .as_array()
            .context("Multi Action Actions must be an array of groups")?;
        ensure!(
            !raw_groups.is_empty() && raw_groups.len() <= MAX_STEPS,
            "Multi Action must contain 1 to {MAX_STEPS} groups"
        );
        let mut groups = Vec::new();
        let mut total = 0usize;
        for (group_index, group) in raw_groups.iter().enumerate() {
            ensure!(
                group.is_object(),
                "Multi Action group {group_index} must be an object"
            );
            let raw_entries = group["Actions"].as_array().with_context(|| {
                format!("Multi Action group {group_index} needs an Actions array")
            })?;
            total = total
                .checked_add(raw_entries.len())
                .context("Multi Action step count overflow")?;
            ensure!(
                total <= MAX_STEPS,
                "Multi Action exceeds {MAX_STEPS} total steps"
            );
            let mut entries = Vec::new();
            let mut ids = HashSet::new();
            for (index, action) in raw_entries.iter().enumerate() {
                ensure!(
                    action.is_object(),
                    "Multi Action child {group_index}/{index} must be an object"
                );
                let uuid = action["UUID"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 1024)
                    .context("Multi Action child needs a valid UUID")?;
                ensure!(
                    !uuid.starts_with("com.elgato.streamdeck.multiactions.") || uuid == DELAY,
                    "Nested Multi Actions are unsupported"
                );
                let id = match action.get("ActionID") {
                    Some(value) => {
                        let id = value
                            .as_str()
                            .filter(|id| !id.is_empty() && id.len() <= 1024)
                            .context("Child ActionID must be a nonempty string")?;
                        ensure!(
                            ids.insert(id),
                            "Duplicate child ActionID in group {group_index}: {id}"
                        );
                        format!("g{group_index}:id:{id}")
                    }
                    None => format!("g{group_index}:index:{index}"),
                };
                if let Some(settings) = action.get("Settings") {
                    ensure!(
                        settings.is_null() || settings.is_object(),
                        "Child Settings must be an object or null"
                    );
                }
                if let Some(state) = action.get("State") {
                    ensure!(
                        state.as_u64().is_some_and(|value| value <= u32::MAX as u64),
                        "Child State must be a nonnegative 32-bit integer"
                    );
                }
                if let Some(states) = action.get("States") {
                    ensure!(
                        states
                            .as_array()
                            .is_some_and(|states| states.iter().all(Value::is_object)),
                        "Child States must be an array of objects"
                    );
                }
                if let Some(state) = action.get("OverrideState") {
                    ensure!(
                        state.as_u64().is_some_and(|value| value <= u32::MAX as u64),
                        "Child OverrideState must be a nonnegative 32-bit integer"
                    );
                }
                let (press_ms, gap_ms) = match action.get("MultiActionDelays") {
                    None => (DEFAULT_TIMING_MS, DEFAULT_TIMING_MS),
                    Some(delays) => {
                        ensure!(delays.is_object(), "MultiActionDelays must be an object");
                        (timing(delays, "Delay1")?, timing(delays, "Delay2")?)
                    }
                };
                let delay_ms = if uuid == DELAY {
                    let delay = action["Settings"]["delay"].as_u64().context(
                        "Explicit Delay Settings.delay must be nonnegative integer milliseconds",
                    )?;
                    ensure!(delay <= MAX_TIMER_MS, "Explicit Delay exceeds 24 hours");
                    Some(delay)
                } else {
                    None
                };
                entries.push(Entry {
                    id,
                    uuid: uuid.into(),
                    raw: action.clone(),
                    press_ms,
                    gap_ms,
                    delay_ms,
                });
            }
            validate_steps(&entries)?;
            groups.push(entries);
        }
        Ok(Self {
            uuid: uuid.into(),
            raw: raw.clone(),
            groups,
        })
    }
}
fn timing(value: &Value, key: &str) -> Result<u64> {
    let timing = value
        .get(key)
        .map(|v| {
            v.as_u64()
                .with_context(|| format!("MultiActionDelays.{key} must be an integer"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_TIMING_MS);
    ensure!(
        (1..=100).contains(&timing),
        "MultiActionDelays.{key} must be between 1 and 100 milliseconds"
    );
    Ok(timing)
}
fn validate_steps(steps: &[Entry]) -> Result<Duration> {
    ensure!(
        steps.len() <= MAX_STEPS,
        "Multi Action exceeds {MAX_STEPS} steps"
    );
    let mut ids = HashSet::new();
    let mut total = 0u64;
    for entry in steps {
        ensure!(
            !entry.id.is_empty() && entry.id.len() <= 2048 && ids.insert(&entry.id),
            "Missing or duplicate scheduled child identity"
        );
        ensure!(
            !entry.uuid.is_empty() && entry.uuid.len() <= 1024,
            "Missing scheduled action UUID"
        );
        ensure!(
            !entry
                .uuid
                .starts_with("com.elgato.streamdeck.multiactions.")
                || entry.uuid == DELAY,
            "Nested Multi Actions are unsupported"
        );
        let duration = if let Some(delay) = entry.delay_ms {
            ensure!(
                entry.uuid == DELAY,
                "Only Delay actions can schedule an explicit delay"
            );
            ensure!(delay <= MAX_TIMER_MS, "Explicit Delay exceeds 24 hours");
            delay
        } else {
            ensure!(entry.uuid != DELAY, "Delay action has no duration");
            ensure!(
                (1..=100).contains(&entry.press_ms) && (1..=100).contains(&entry.gap_ms),
                "Action press/gap timings must be between 1 and 100 milliseconds"
            );
            entry.press_ms + entry.gap_ms
        };
        total = total
            .checked_add(duration)
            .context("Multi Action duration overflow")?;
        ensure!(
            total <= MAX_TIMER_MS,
            "Multi Action group duration exceeds 24 hours"
        );
    }
    Ok(Duration::from_millis(total))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Press {
        parent: String,
        entry: Entry,
    },
    Release {
        parent: String,
        entry: Entry,
    },
    /// Normal completion after the final action gap. Never sent by cancellation.
    Finished {
        parent: String,
    },
}
#[derive(Clone, Copy)]
enum Phase {
    Waiting(Duration),
    Held(Duration),
}
impl Phase {
    fn deadline(self) -> Duration {
        match self {
            Self::Waiting(at) | Self::Held(at) => at,
        }
    }
}
struct Run {
    steps: Vec<Entry>,
    index: usize,
    phase: Phase,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RunSnapshot {
    pub parent: String,
    pub current_entry: Option<String>,
    pub held: bool,
    pub remaining_steps: usize,
    pub next_due: Duration,
}
#[derive(Default)]
pub struct Scheduler {
    runs: BTreeMap<String, Run>,
    last_now: Duration,
}
impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }
    /// Existing runs are unchanged and return false. Caller-selected groups may
    /// have manifest timings applied to their Entry fields before this call.
    pub fn start(&mut self, parent: &str, steps: Vec<Entry>, now: Duration) -> Result<bool> {
        ensure!(
            !parent.is_empty() && parent.len() <= 4096,
            "Invalid parent Multi Action context"
        );
        if self.runs.contains_key(parent) {
            return Ok(false);
        }
        ensure!(
            self.runs.len() < MAX_RUNNING,
            "Too many concurrently running Multi Actions"
        );
        let total = validate_steps(&steps)?;
        let now = now.max(self.last_now);
        ensure!(
            now.checked_add(total).is_some(),
            "Multi Action clock deadline overflow"
        );
        self.last_now = now;
        self.runs.insert(
            parent.into(),
            Run {
                steps,
                index: 0,
                phase: Phase::Waiting(now),
            },
        );
        Ok(true)
    }
    /// Dispatch at most one press or release per run in a call. A late poll moves
    /// subsequent hold/gap deadlines forward from actual dispatch time, never from
    /// an obsolete planned time. Regressing clocks are clamped to the latest clock.
    pub fn due(&mut self, now: Duration) -> Vec<Event> {
        let now = now.max(self.last_now);
        self.last_now = now;
        let mut events = Vec::new();
        let mut finished = Vec::new();
        for (parent, run) in &mut self.runs {
            loop {
                if run.phase.deadline() > now {
                    break;
                }
                match run.phase {
                    Phase::Held(_) => {
                        let entry = run.steps[run.index].clone();
                        run.index += 1;
                        run.phase =
                            Phase::Waiting(now.saturating_add(Duration::from_millis(entry.gap_ms)));
                        events.push(Event::Release {
                            parent: parent.clone(),
                            entry,
                        });
                        break;
                    }
                    Phase::Waiting(at) => {
                        let Some(entry) = run.steps.get(run.index) else {
                            finished.push(parent.clone());
                            events.push(Event::Finished {
                                parent: parent.clone(),
                            });
                            break;
                        };
                        if let Some(delay) = entry.delay_ms {
                            // Pure timers have no virtual down/up and no default
                            // press/gap of their own. Their start follows the prior
                            // gate, so late timer polls do not add another delay.
                            run.index += 1;
                            run.phase =
                                Phase::Waiting(at.saturating_add(Duration::from_millis(delay)));
                            continue;
                        }
                        run.phase =
                            Phase::Held(now.saturating_add(Duration::from_millis(entry.press_ms)));
                        events.push(Event::Press {
                            parent: parent.clone(),
                            entry: entry.clone(),
                        });
                        break;
                    }
                }
            }
        }
        for parent in finished {
            self.runs.remove(&parent);
        }
        events
    }
    /// Cancel before processing due events when cancellation and a deadline share
    /// a timestamp. A held child gets exactly one release, then the run is removed.
    pub fn cancel(&mut self, parent: &str) -> Vec<Event> {
        let Some(run) = self.runs.remove(parent) else {
            return Vec::new();
        };
        if matches!(run.phase, Phase::Held(_)) {
            vec![Event::Release {
                parent: parent.into(),
                entry: run.steps[run.index].clone(),
            }]
        } else {
            Vec::new()
        }
    }
    pub fn cancel_all(&mut self) -> Vec<Event> {
        let parents: Vec<_> = self.runs.keys().cloned().collect();
        parents
            .into_iter()
            .flat_map(|parent| self.cancel(&parent))
            .collect()
    }
    pub fn running(&self, parent: &str) -> bool {
        self.runs.contains_key(parent)
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.runs.values().map(|run| run.phase.deadline()).min()
    }
    pub fn snapshot(&self) -> Vec<RunSnapshot> {
        self.runs
            .iter()
            .map(|(parent, run)| RunSnapshot {
                parent: parent.clone(),
                current_entry: run.steps.get(run.index).map(|entry| entry.id.clone()),
                held: matches!(run.phase, Phase::Held(_)),
                remaining_steps: run.steps.len().saturating_sub(run.index),
                next_due: run.phase.deadline(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }
    fn action(id: &str, press: u64, gap: u64) -> Entry {
        Entry {
            id: id.into(),
            uuid: "test.action".into(),
            raw: json!({"ActionID":id,"UUID":"test.action"}),
            press_ms: press,
            gap_ms: gap,
            delay_ms: None,
        }
    }
    fn delay(id: &str, length: u64) -> Entry {
        Entry {
            id: id.into(),
            uuid: DELAY.into(),
            raw: json!({"UUID":DELAY,"Settings":{"delay":length}}),
            press_ms: 100,
            gap_ms: 100,
            delay_ms: Some(length),
        }
    }
    #[test]
    fn parse_preserves_unknown_fields_override_state_and_group_scoped_ids() {
        let child = json!({"ActionID":"same","UUID":"plugin.action","State":1,"OverrideState":0,"Settings":{"future":7},"MultiActionDelays":{"Delay1":2,"Delay2":3},"FutureField":[1,2]});
        let raw = json!({"UUID":SWITCH,"Actions":[{"Actions":[child.clone()]},{"Actions":[child.clone()]}],"FutureRoot":"kept"});
        let parsed = Sequence::parse(SWITCH, &raw).unwrap();
        assert_eq!(parsed.raw, raw);
        assert_eq!(parsed.groups[0][0].raw, child);
        assert_ne!(parsed.groups[0][0].id, parsed.groups[1][0].id);
        assert_eq!(parsed.groups[0][0].press_ms, 2);
        assert_eq!(parsed.groups[0][0].gap_ms, 3);
        let raw = json!({"Actions":[{"Actions":[{"UUID":"plugin.action"}]}]});
        let parsed = Sequence::parse(ROUTINE, &raw).unwrap();
        assert_eq!(parsed.groups[0][0].id, "g0:index:0");
        assert_eq!(parsed.groups[0][0].press_ms, 100);
    }
    #[test]
    fn null_child_settings_are_retained_for_runtime_defaulting() {
        let raw = json!({"Actions":[{"Actions":[{"UUID":"test.action","Settings":null}]}]});
        let parsed = Sequence::parse(ROUTINE, &raw).unwrap();
        assert_eq!(parsed.raw, raw);
        assert_eq!(parsed.groups[0][0].raw["Settings"], Value::Null);
    }
    #[test]
    fn malformed_groups_duplicate_ids_nesting_and_timings_reject() {
        for raw in [
            json!({"Actions":[]}),
            json!({"Actions":[{}]}),
            json!({"Actions":[{"Actions":{}}]}),
            json!({"Actions":[{"Actions":[{"ActionID":"a","UUID":"x"},{"ActionID":"a","UUID":"x"}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":ROUTINE}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":"x","MultiActionDelays":{"Delay1":0}}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":"x","OverrideState":-1}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":"x","State":-1}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":"x","States":[1]}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":DELAY,"Settings":{"delay":1.5}}]}]}),
        ] {
            assert!(Sequence::parse(ROUTINE, &raw).is_err(), "{raw}");
        }
        let raw = json!({"Actions":[{"Actions":vec![json!({"UUID":"x"});513]}]});
        assert!(Sequence::parse(ROUTINE, &raw).is_err());
    }
    #[test]
    fn press_release_gap_and_explicit_delay_are_ordered() {
        let mut scheduler = Scheduler::new();
        scheduler
            .start(
                "parent",
                vec![action("a", 10, 20), delay("wait", 30), action("b", 5, 7)],
                ms(0),
            )
            .unwrap();
        assert!(matches!(&scheduler.due(ms(0))[..],[Event::Press{entry,..}] if entry.id=="a"));
        assert!(scheduler.due(ms(9)).is_empty());
        assert!(matches!(&scheduler.due(ms(10))[..],[Event::Release{entry,..}] if entry.id=="a"));
        assert_eq!(scheduler.next_due(), Some(ms(30)));
        assert!(scheduler.due(ms(30)).is_empty());
        assert_eq!(scheduler.next_due(), Some(ms(60)));
        assert!(scheduler.due(ms(59)).is_empty());
        assert!(matches!(&scheduler.due(ms(60))[..],[Event::Press{entry,..}] if entry.id=="b"));
        assert!(matches!(&scheduler.due(ms(65))[..],[Event::Release{entry,..}] if entry.id=="b"));
        assert!(scheduler.running("parent"));
        assert_eq!(
            scheduler.due(ms(72)),
            vec![Event::Finished {
                parent: "parent".into()
            }]
        );
        assert!(!scheduler.running("parent"));
    }
    #[test]
    fn late_polling_never_compresses_a_hold_or_gap() {
        let mut scheduler = Scheduler::new();
        scheduler
            .start("p", vec![action("a", 10, 20), action("b", 5, 7)], ms(0))
            .unwrap();
        assert!(matches!(
            &scheduler.due(ms(1000))[..],
            [Event::Press { .. }]
        ));
        assert_eq!(scheduler.next_due(), Some(ms(1010)));
        assert!(scheduler.due(ms(1001)).is_empty());
        assert!(matches!(
            &scheduler.due(ms(2000))[..],
            [Event::Release { .. }]
        ));
        assert_eq!(scheduler.next_due(), Some(ms(2020)));
        assert!(scheduler.due(ms(2019)).is_empty());
        assert!(matches!(
            &scheduler.due(ms(3000))[..],
            [Event::Press { .. }]
        ));
        assert_eq!(scheduler.next_due(), Some(ms(3005)));
        assert!(scheduler.due(ms(2999)).is_empty());
    }
    #[test]
    fn cancellation_before_press_or_at_release_deadline_is_balanced() {
        let mut scheduler = Scheduler::new();
        scheduler
            .start("a", vec![action("one", 10, 20)], ms(0))
            .unwrap();
        assert!(scheduler.cancel("a").is_empty());
        assert!(scheduler.due(ms(100)).is_empty());
        scheduler
            .start("a", vec![action("one", 10, 20)], ms(100))
            .unwrap();
        scheduler.due(ms(100));
        assert!(matches!(
            &scheduler.cancel("a")[..],
            [Event::Release { .. }]
        ));
        assert!(scheduler.due(ms(110)).is_empty());
        assert!(scheduler.cancel("a").is_empty());
        scheduler
            .start("a", vec![action("one", 10, 20)], ms(110))
            .unwrap();
        scheduler.due(ms(110));
        scheduler.due(ms(120));
        assert!(scheduler.cancel("a").is_empty());
        assert!(scheduler.due(ms(140)).is_empty());
    }
    #[test]
    fn repeated_start_does_not_replace_a_held_child_and_cancel_all_is_scoped() {
        let mut scheduler = Scheduler::new();
        scheduler
            .start("a", vec![action("original", 10, 20)], ms(0))
            .unwrap();
        scheduler.due(ms(0));
        assert!(
            !scheduler
                .start("a", vec![action("replacement", 1, 1)], ms(1))
                .unwrap()
        );
        scheduler
            .start("b", vec![delay("wait", 20)], ms(1))
            .unwrap();
        let events = scheduler.cancel_all();
        assert!(
            matches!(&events[..],[Event::Release{parent,entry}] if parent=="a" && entry.id=="original")
        );
        assert!(scheduler.snapshot().is_empty());
    }
    #[test]
    fn timer_bounds_empty_runs_and_zero_delays_are_finite() {
        let mut scheduler = Scheduler::new();
        assert!(
            scheduler
                .start("x", vec![delay("wait", MAX_TIMER_MS + 1)], ms(0))
                .is_err()
        );
        assert!(
            scheduler
                .start("x", vec![delay("a", MAX_TIMER_MS), delay("b", 1)], ms(0))
                .is_err()
        );
        assert!(
            scheduler
                .start("x", vec![action("a", 1, 1)], Duration::MAX)
                .is_err()
        );
        scheduler.start("empty", vec![], ms(0)).unwrap();
        assert_eq!(
            scheduler.due(ms(0)),
            vec![Event::Finished {
                parent: "empty".into()
            }]
        );
        scheduler
            .start("wait", vec![delay("zero", 0), delay("positive", 5)], ms(0))
            .unwrap();
        assert!(scheduler.due(ms(0)).is_empty());
        assert_eq!(scheduler.next_due(), Some(ms(5)));
        assert_eq!(
            scheduler.due(ms(5)),
            vec![Event::Finished {
                parent: "wait".into()
            }]
        );
    }
}
