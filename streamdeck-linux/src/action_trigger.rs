//! Action Trigger schema, icon hit rectangles, and a pure timed pulse queue.
//!
//! Recovered from evidence/action-trigger-contracts.json: slots are CCW/push/CW,
//! holds are 60 ms, intervening gaps 15 ms, and the pending queue caps at three.
//! Starting a pulse consumes one pending item: an initial burst totals at most
//! three pulses, while later input can add three behind an already held pulse.
//! Physical dial push is independent and belongs to the host. Cancellation here
//! releases only an actually held pulse, an explicit balanced Linux policy.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

pub const UUID: &str = "com.elgato.streamdeck.keys.adaptor";
const STACK: &str = "com.elgato.streamdeck.dial.stack";
const HOLD: Duration = Duration::from_millis(60);
const GAP: Duration = Duration::from_millis(15);
const MAX_PENDING: u8 = 3;
const MAX_CONTEXTS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub uuid: String,
    pub raw: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    /// Always three positions. Blanks are never filtered out or shifted.
    pub children: Vec<Option<Entry>>,
    pub raw: Value,
}
pub fn parse(raw: &Value) -> Result<Trigger> {
    ensure!(raw.is_object(), "Action Trigger must be an object");
    if let Some(uuid) = raw.get("UUID") {
        ensure!(
            uuid.as_str() == Some(UUID),
            "Action Trigger UUID does not match its container"
        );
    }
    if let Some(settings) = raw.get("Settings") {
        ensure!(
            settings.is_null() || settings.is_object(),
            "Action Trigger Settings must be an object or null"
        );
    }
    let actions = raw["Actions"]
        .as_array()
        .context("Action Trigger Actions must be a flat positional array")?;
    ensure!(
        actions.len() <= 3,
        "Action Trigger has more than three positions; refusing to discard children"
    );
    let mut children = Vec::with_capacity(3);
    for (index, action) in actions.iter().enumerate() {
        ensure!(
            action.is_object(),
            "Action Trigger position {index} must be an object"
        );
        let uuid = match action.get("UUID") {
            None => "",
            Some(value) => value
                .as_str()
                .context("Action Trigger child UUID must be a string")?,
        };
        if uuid.is_empty() {
            ensure!(
                action
                    .get("Actions")
                    .is_none_or(|actions| actions.as_array().is_some_and(Vec::is_empty)),
                "Blank Action Trigger position contains nested actions with no UUID"
            );
            children.push(None);
            continue;
        }
        ensure!(uuid.len() <= 1024, "Action Trigger child UUID is too long");
        let mut budget = 4096usize;
        reject_recursion(action, 0, &mut budget)?;
        let id = match action.get("ActionID") {
            None => format!("trigger:{index}:missing"),
            Some(value) => {
                let id = value
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 1024)
                    .context("Action Trigger child ActionID must be nonempty text")?;
                format!("trigger:{index}:id:{id}")
            }
        };
        if let Some(settings) = action.get("Settings") {
            ensure!(
                settings.is_null() || settings.is_object(),
                "Action Trigger child Settings must be an object or null"
            );
        }
        if let Some(state) = action.get("State") {
            ensure!(
                state.as_u64().is_some_and(|v| v <= u32::MAX as u64),
                "Action Trigger child State must be a nonnegative integer"
            );
        }
        if let Some(states) = action.get("States") {
            ensure!(
                states
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_object)),
                "Action Trigger child States must be an array of objects"
            );
        }
        children.push(Some(Entry {
            id,
            uuid: uuid.into(),
            raw: action.clone(),
        }));
    }
    children.resize(3, None);
    Ok(Trigger {
        children,
        raw: raw.clone(),
    })
}
/// Inspect only reserved Actions trees, not arbitrary plugin Settings/metadata.
/// Multi Action and unknown key containers remain raw for their respective host
/// handlers, but cannot hide recursion into this adaptor or an encoder Dial Stack.
fn reject_recursion(action: &Value, depth: usize, budget: &mut usize) -> Result<()> {
    ensure!(
        depth <= 16 && *budget > 0,
        "Action Trigger nested action tree exceeds validation bounds"
    );
    *budget -= 1;
    ensure!(
        !matches!(action["UUID"].as_str(), Some(UUID | STACK)),
        "Nested Action Triggers and Dial Stacks are unsupported"
    );
    if let Some(actions) = action.get("Actions") {
        let actions = actions
            .as_array()
            .context("Nested Actions must be an array")?;
        for action in actions {
            ensure!(action.is_object(), "Nested action records must be objects");
            reject_recursion(action, depth + 1, budget)?;
        }
    }
    Ok(())
}

/// Hit-test the recovered 56x56 icons in the dial's local 200x100 coordinates.
/// Integer rectangles use half-open right/bottom edges, an explicit Linux policy;
/// exact Windows edge inclusivity was not recovered.
pub fn point(x: u16, y: u16) -> Option<usize> {
    [(12, 29), (72, 14), (132, 29)]
        .iter()
        .position(|&(left, top)| x >= left && x < left + 56 && y >= top && y < top + 56)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Press { parent: String, index: usize },
    Release { parent: String, index: usize },
}
#[derive(Clone, Copy)]
enum Phase {
    Held(Duration),
    Gap(Duration),
}
impl Phase {
    fn at(self) -> Duration {
        match self {
            Self::Held(at) | Self::Gap(at) => at,
        }
    }
}
struct Run {
    index: usize,
    pending: u8,
    phase: Phase,
}
#[derive(Default)]
pub struct Repeater {
    runs: BTreeMap<String, Run>,
    last_now: Duration,
}
impl Repeater {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }
    /// Start immediately or refill the same target's queue. A different target
    /// releases the old held pulse before pressing the replacement. Zero counts,
    /// invalid indices/contexts and excess new contexts are no-ops; existing runs
    /// are never evicted to make room. Caller validates child presence before use.
    pub fn request(&mut self, parent: &str, index: usize, count: u8, now: Duration) -> Vec<Event> {
        if parent.is_empty() || parent.len() > 4096 || index >= 3 || count == 0 {
            return Vec::new();
        }
        let now = now.max(self.last_now);
        self.last_now = now;
        if let Some(run) = self.runs.get_mut(parent)
            && run.index == index
        {
            run.pending = run.pending.saturating_add(count).min(MAX_PENDING);
            return Vec::new();
        }
        if !self.runs.contains_key(parent) && self.runs.len() >= MAX_CONTEXTS {
            return Vec::new();
        }
        let mut events = self.cancel(parent);
        let Some(deadline) = now.checked_add(HOLD) else {
            return events;
        };
        self.runs.insert(
            parent.into(),
            Run {
                index,
                pending: count.min(MAX_PENDING) - 1,
                phase: Phase::Held(deadline),
            },
        );
        events.push(Event::Press {
            parent: parent.into(),
            index,
        });
        events
    }
    /// One transition per context per poll. Hold and gap deadlines start at the
    /// actual dispatch time, so late polling never compresses a new pulse.
    pub fn due(&mut self, now: Duration) -> Vec<Event> {
        let now = now.max(self.last_now);
        self.last_now = now;
        let mut events = Vec::new();
        let mut remove = Vec::new();
        for (parent, run) in &mut self.runs {
            if run.phase.at() > now {
                continue;
            }
            match run.phase {
                Phase::Held(_) => {
                    events.push(Event::Release {
                        parent: parent.clone(),
                        index: run.index,
                    });
                    if run.pending > 0
                        && let Some(deadline) = now.checked_add(GAP)
                    {
                        run.phase = Phase::Gap(deadline);
                    } else {
                        remove.push(parent.clone());
                    }
                }
                Phase::Gap(_) => {
                    if run.pending > 0
                        && let Some(deadline) = now.checked_add(HOLD)
                    {
                        run.pending -= 1;
                        run.phase = Phase::Held(deadline);
                        events.push(Event::Press {
                            parent: parent.clone(),
                            index: run.index,
                        });
                    } else {
                        remove.push(parent.clone());
                    }
                }
            }
        }
        for parent in remove {
            self.runs.remove(&parent);
        }
        events
    }
    pub fn cancel(&mut self, parent: &str) -> Vec<Event> {
        match self.runs.remove(parent) {
            Some(Run {
                index,
                phase: Phase::Held(_),
                ..
            }) => vec![Event::Release {
                parent: parent.into(),
                index,
            }],
            _ => Vec::new(),
        }
    }
    #[cfg(test)]
    pub fn cancel_all(&mut self) -> Vec<Event> {
        let parents: Vec<_> = self.runs.keys().cloned().collect();
        parents
            .into_iter()
            .flat_map(|parent| self.cancel(&parent))
            .collect()
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.runs.values().map(|run| run.phase.at()).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }
    fn press(parent: &str, index: usize) -> Event {
        Event::Press {
            parent: parent.into(),
            index,
        }
    }
    fn release(parent: &str, index: usize) -> Event {
        Event::Release {
            parent: parent.into(),
            index,
        }
    }
    #[test]
    fn original_fixture_fields_and_positional_children_survive_unchanged() {
        let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
        for raw in fixtures.as_array().unwrap() {
            let parsed = parse(raw).unwrap();
            assert_eq!(parsed.raw, *raw);
            assert_eq!(parsed.children.len(), 3);
            for index in 0..3 {
                assert_eq!(
                    parsed.children[index].as_ref().unwrap().raw,
                    raw["Actions"][index]
                );
            }
        }
    }
    #[test]
    fn blanks_stay_in_position_and_unknowns_nested_raw_and_duplicate_ids_survive() {
        let raw = json!({"UUID":UUID,"Actions":[{"Actions":[],"Future":"kept"},{"UUID":"unknown.key","ActionID":"same","Settings":null,"Future":[1,2]},{"UUID":"com.elgato.streamdeck.keys.stack","ActionID":"same","Actions":[]}]});
        let parsed = parse(&raw).unwrap();
        assert!(parsed.children[0].is_none());
        assert_eq!(parsed.raw, raw);
        assert_ne!(
            parsed.children[1].as_ref().unwrap().id,
            parsed.children[2].as_ref().unwrap().id
        );
        let multi = json!({"Actions":[{"UUID":"com.elgato.streamdeck.multiactions.routine","Actions":[{"Actions":[{"UUID":"test.action"}]}]}]});
        assert_eq!(
            parse(&multi).unwrap().children[0].as_ref().unwrap().raw,
            multi["Actions"][0]
        );
        assert_eq!(
            parse(&json!({"Actions":[]})).unwrap().children,
            vec![None, None, None]
        );
    }
    #[test]
    fn malformed_or_recursive_children_are_not_silently_discarded() {
        for raw in [
            json!({"Actions":vec![json!({"Actions":[]});4]}),
            json!({"Actions":[null]}),
            json!({"Actions":[{"UUID":UUID}]}),
            json!({"Actions":[{"UUID":STACK}]}),
            json!({"Actions":[{"UUID":"com.elgato.streamdeck.multiactions.routine","Actions":[{"Actions":[{"UUID":UUID}]}]}]}),
            json!({"Actions":[{"Actions":[{"UUID":"hidden"}]}]}),
            json!({"Actions":[{"UUID":"test","Settings":[]}]}),
        ] {
            assert!(parse(&raw).is_err(), "{raw}");
        }
    }
    #[test]
    fn initial_burst_caps_at_three_but_active_pulse_can_have_three_more_pending() {
        let mut queue = Repeater::new();
        assert_eq!(queue.request("p", 2, u8::MAX, ms(0)), vec![press("p", 2)]);
        let mut presses = 1;
        let mut releases = 0;
        for time in [60, 75, 135, 150, 210] {
            for event in queue.due(ms(time)) {
                match event {
                    Event::Press { .. } => presses += 1,
                    Event::Release { .. } => releases += 1,
                }
            }
        }
        assert_eq!((presses, releases), (3, 3));
        assert_eq!(queue.next_due(), None);
        assert_eq!(queue.request("p", 2, 1, ms(300)), vec![press("p", 2)]);
        assert!(queue.request("p", 2, 100, ms(301)).is_empty());
        presses = 1;
        releases = 0;
        for time in [360, 375, 435, 450, 510, 525, 585] {
            for event in queue.due(ms(time)) {
                match event {
                    Event::Press { .. } => presses += 1,
                    Event::Release { .. } => releases += 1,
                }
            }
        }
        assert_eq!((presses, releases), (4, 4));
        assert_eq!(queue.next_due(), None);
    }
    #[test]
    fn target_change_releases_only_held_old_target_before_new_down() {
        let mut queue = Repeater::new();
        queue.request("p", 0, 3, ms(0));
        assert_eq!(
            queue.request("p", 2, 1, ms(10)),
            vec![release("p", 0), press("p", 2)]
        );
        assert!(queue.due(ms(60)).is_empty());
        assert_eq!(queue.due(ms(70)), vec![release("p", 2)]);
        assert!(queue.due(ms(1000)).is_empty());
        queue.request("p", 0, 2, ms(1000));
        assert_eq!(queue.due(ms(1060)), vec![release("p", 0)]);
        assert_eq!(queue.request("p", 1, 1, ms(1061)), vec![press("p", 1)]);
    }
    #[test]
    fn late_polls_keep_full_hold_and_gap_and_cancellation_balances() {
        let mut queue = Repeater::new();
        queue.request("p", 0, 2, ms(0));
        assert_eq!(queue.due(ms(1000)), vec![release("p", 0)]);
        assert_eq!(queue.next_due(), Some(ms(1015)));
        assert!(queue.due(ms(1014)).is_empty());
        assert_eq!(queue.due(ms(2000)), vec![press("p", 0)]);
        assert_eq!(queue.next_due(), Some(ms(2060)));
        assert!(queue.due(ms(2001)).is_empty());
        assert_eq!(queue.cancel("p"), vec![release("p", 0)]);
        assert!(queue.cancel("p").is_empty());
        assert!(queue.due(ms(2060)).is_empty());
        queue.request("a", 0, 2, ms(3000));
        queue.request("b", 2, 1, ms(3000));
        queue.due(ms(3060));
        assert!(queue.cancel_all().is_empty());
        assert_eq!(queue.next_due(), None);
    }
    #[test]
    fn zero_invalid_and_excess_context_requests_leave_existing_runs_intact() {
        let mut queue = Repeater::new();
        for index in 0..64 {
            assert_eq!(queue.request(&format!("p{index}"), 0, 1, ms(0)).len(), 1);
        }
        assert!(queue.request("overflow", 0, 1, ms(0)).is_empty());
        assert_eq!(queue.runs.len(), 64);
        assert!(queue.request("p0", 2, 0, ms(1)).is_empty());
        assert!(queue.request("p0", 3, 1, ms(1)).is_empty());
        assert_eq!(queue.cancel("p0"), vec![release("p0", 0)]);
        assert_eq!(queue.cancel_all().len(), 63);
    }
    #[test]
    fn icon_hit_rectangles_use_half_open_edges_not_full_width_strips() {
        assert_eq!(point(12, 29), Some(0));
        assert_eq!(point(67, 84), Some(0));
        assert_eq!(point(68, 29), None);
        assert_eq!(point(12, 85), None);
        assert_eq!(point(72, 14), Some(1));
        assert_eq!(point(127, 69), Some(1));
        assert_eq!(point(128, 14), None);
        assert_eq!(point(72, 70), None);
        assert_eq!(point(132, 29), Some(2));
        assert_eq!(point(187, 84), Some(2));
        assert_eq!(point(188, 29), None);
        assert_eq!(point(0, 50), None);
        assert_eq!(point(u16::MAX, u16::MAX), None);
    }

    // Exact action objects from the allowed original-installer evidence report.
    const FIXTURES: &str = r####"[{"ActionID":"fb848cc2-3239-49d6-9161-841ebb1896ec","Actions":[{"ActionID":"8e2e8086-8409-4a74-b7a5-a4a84abfc6a0","LinkedTitle":true,"Name":"Previous Page","Plugin":{"Name":"Pages","UUID":"com.elgato.streamdeck.page","Version":"1.0"},"Resources":null,"Settings":{},"State":0,"States":[{"Image":"Images/8HTNSJ7GNP74L8P8FC42D00FNKZ.png"}],"UUID":"com.elgato.streamdeck.page.previous"},{"ActionID":"61fdb664-b9ff-44e6-87a2-afa1bd9ff0ad","LinkedTitle":true,"Name":"Go to Page","Plugin":{"Name":"Pages","UUID":"com.elgato.streamdeck.page","Version":"1.0"},"Resources":null,"Settings":{"PageIndex":1},"State":0,"States":[{"FontFamily":"","FontSize":12,"FontStyle":"","FontUnderline":false,"Image":"Images/GL1GH3NB05677BV07BM01EOSG8Z.png","OutlineThickness":2,"ShowTitle":false,"TitleAlignment":"bottom","TitleColor":"#ffffff"}],"UUID":"com.elgato.streamdeck.page.goto"},{"ActionID":"347b06dc-6699-4434-a695-17a654afa293","LinkedTitle":true,"Name":"Next Page","Plugin":{"Name":"Pages","UUID":"com.elgato.streamdeck.page","Version":"1.0"},"Resources":null,"Settings":{},"State":0,"States":[{"Image":"Images/VLDVEK6EMP5FW6EMH71MMF590SZ.png"}],"UUID":"com.elgato.streamdeck.page.next"}],"LinkedTitle":true,"Name":"Action Trigger","Plugin":{"Name":"Keys","UUID":"com.elgato.streamdeck.keys","Version":"1.0"},"Resources":null,"Settings":{},"State":0,"States":[{}],"UUID":"com.elgato.streamdeck.keys.adaptor"},{"ActionID":"e5dbdf37-fb8f-4974-9526-b0c84f7bb68a","Actions":[{"ActionID":"5e5900e5-0ad7-4c70-b9c2-54b7e0d2d613","LinkedTitle":true,"Name":"Multimedia","Plugin":{"Name":"Multimedia","UUID":"com.elgato.streamdeck.system.multimedia","Version":"1.0"},"Resources":null,"Settings":{"actionIdx":0},"State":0,"States":[{"Image":"Images/NDF0DJQ6R92CL36JEVP8G1R1EOZ.png"}],"UUID":"com.elgato.streamdeck.system.multimedia"},{"ActionID":"17307b82-e565-4ec9-80f4-8524106c1085","LinkedTitle":true,"Name":"Multimedia","Plugin":{"Name":"Multimedia","UUID":"com.elgato.streamdeck.system.multimedia","Version":"1.0"},"Resources":null,"Settings":{"actionIdx":1},"State":0,"States":[{"Image":"Images/3793QI1I4H1DB2QG028A60VNJSZ.png"}],"UUID":"com.elgato.streamdeck.system.multimedia"},{"ActionID":"85c122d8-270c-4827-b6c6-593b0b11ae88","LinkedTitle":true,"Name":"Multimedia","Plugin":{"Name":"Multimedia","UUID":"com.elgato.streamdeck.system.multimedia","Version":"1.0"},"Resources":null,"Settings":{"actionIdx":2},"State":0,"States":[{"Image":"Images/EV70QS8SK17GP1P5J3H2BAS64CZ.png"}],"UUID":"com.elgato.streamdeck.system.multimedia"}],"LinkedTitle":true,"Name":"Action Trigger","Plugin":{"Name":"Keys","UUID":"com.elgato.streamdeck.keys","Version":"1.0"},"Resources":null,"Settings":{},"State":0,"States":[{}],"UUID":"com.elgato.streamdeck.keys.adaptor"}]"####;
}
