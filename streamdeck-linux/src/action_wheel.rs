// Action Wheel's saved schema, paced navigation and display geometry.
// Recovered from Stream Deck 7.6.0.23012; see docs/ACTION_WHEEL.md.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};

pub const UUID: &str = "com.elgato.streamdeck.keys.stack";
pub const MAX_CHILDREN: usize = 512;
pub const ROTATION_INTERVAL: Duration = Duration::from_millis(61);
pub const TAP_HOLD: Duration = Duration::from_millis(100);
pub const TAP_GAP: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub uuid: String,
    pub raw: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Wheel {
    pub children: Vec<Entry>,
    pub current: usize,
    pub extended_titles: bool,
    pub invert_rotation: bool,
    pub control_offset: usize,
    /// Unmodified source, including unknown fields and out-of-range saved values.
    pub raw: Value,
}

pub fn parse(raw: &Value) -> Result<Wheel> {
    ensure!(raw.is_object(), "Action Wheel must be an object");
    if let Some(uuid) = raw.get("UUID") {
        ensure!(
            uuid == UUID,
            "Action Wheel UUID does not match its container"
        );
    }
    let actions = raw["Actions"]
        .as_array()
        .context("Action Wheel Actions must be a flat array")?;
    ensure!(
        actions.len() <= MAX_CHILDREN,
        "Action Wheel exceeds {MAX_CHILDREN} children"
    );
    let settings = &raw["Settings"];
    ensure!(
        settings.is_null() || settings.is_object(),
        "Action Wheel Settings must be an object or null"
    );
    let boolean = |name: &str| -> Result<bool> {
        settings.get(name).map_or(Ok(false), |v| {
            v.as_bool()
                .with_context(|| format!("Action Wheel {name} must be a boolean"))
        })
    };
    let number = |name: &str| -> Result<Option<f64>> {
        settings
            .get(name)
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .with_context(|| format!("Action Wheel {name} must be a finite number"))
            })
            .transpose()
    };
    let last = actions.len().saturating_sub(1);
    // Native unsigned conversion turns negative integers into a large unsigned
    // value, then min(value, count-1) selects the last child. Positive fractions
    // truncate. Keep the raw number unchanged for profile round trips.
    let current = number("CurrentIdx")?.map_or(0, |n| {
        if n.trunc() < 0.0 {
            last
        } else {
            (n as usize).min(last)
        }
    });
    // Negative ControlOffset leaves the native constructor default unchanged.
    // Bound malformed oversized offsets before the viewport adjustment loops.
    let control_offset =
        number("ControlOffset")?.map_or(0, |n| if n < 0.0 { 0 } else { (n as usize).min(last) });
    let mut ids = HashSet::new();
    let mut children = Vec::with_capacity(actions.len());
    for (index, action) in actions.iter().enumerate() {
        ensure!(action.is_object(), "Action Wheel child must be an object");
        let uuid = action["UUID"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 1024)
            .context("Action Wheel child needs a valid UUID")?;
        ensure!(uuid != UUID, "Recursive Action Wheels are unsupported");
        let id = match action.get("ActionID") {
            None => format!("wheel:index:{index}"),
            Some(value) => {
                let id = value
                    .as_str()
                    .filter(|v| !v.is_empty() && v.len() <= 1024)
                    .context("Action Wheel ActionID must be nonempty text")?;
                ensure!(ids.insert(id), "Duplicate Action Wheel ActionID");
                format!("wheel:id:{id}")
            }
        };
        if let Some(value) = action.get("Settings") {
            ensure!(
                value.is_null() || value.is_object(),
                "Wheel child settings must be an object or null"
            );
        }
        if let Some(value) = action.get("State") {
            ensure!(
                value.as_u64().is_some_and(|v| v <= u32::MAX as u64),
                "Wheel child state must be a nonnegative integer"
            );
        }
        if let Some(value) = action.get("States") {
            ensure!(
                value
                    .as_array()
                    .is_some_and(|states| states.iter().all(Value::is_object)),
                "Wheel child states must be an array of objects"
            );
        }
        children.push(Entry {
            id,
            uuid: uuid.into(),
            raw: action.clone(),
        });
    }
    Ok(Wheel {
        children,
        current,
        extended_titles: boolean("ExtTitles")?,
        invert_rotation: boolean("InvertRotation")?,
        control_offset,
        raw: raw.clone(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Movement {
    pub index: usize,
    pub delta: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Rotation {
    pending: i32,
    due: Option<Duration>,
    last_now: Duration,
}
impl Rotation {
    /// A new unpressed turn replaces, rather than adds to, pending rotation.
    /// Held turns leave an already queued turn alone, as in the native handler.
    pub fn request(
        &mut self,
        ticks: i8,
        pressed: bool,
        inverted: bool,
        count: usize,
        now: Duration,
    ) -> bool {
        if pressed || !(2..=MAX_CHILDREN).contains(&count) {
            return false;
        }
        let now = now.max(self.last_now);
        self.last_now = now;
        self.pending = (i32::from(ticks) * if inverted { -1 } else { 1 }).clamp(-5, 5);
        self.due.get_or_insert(now);
        true
    }
    pub fn advance(&mut self, current: usize, count: usize, now: Duration) -> Option<Movement> {
        let now = now.max(self.last_now);
        self.last_now = now;
        if self.due.is_none_or(|due| due > now) {
            return None;
        }
        self.due = None;
        if self.pending == 0 || !(2..=MAX_CHILDREN).contains(&count) {
            self.pending = 0;
            return None;
        }
        let steps = (self.pending.unsigned_abs() * 2).div_ceil(3) as i32;
        let delta = steps * self.pending.signum();
        self.pending -= delta;
        let next = ((current % count) as i64 + i64::from(delta)).rem_euclid(count as i64) as usize;
        // Even the last consumed batch rearms the native timer once. A turn
        // arriving before it expires must not become an immediate extra step.
        self.due = now.checked_add(ROTATION_INTERVAL);
        Some(Movement { index: next, delta })
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.due
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Previous,
    Activate,
    Next,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub index: usize,
    pub rect: [u32; 4],
    pub opacity: f32,
    pub radius: u32,
    pub hit: Hit,
}
/// Back-to-front draw order. Two-item wheels have two distinct layouts.
pub fn cards(count: usize, current: usize) -> Vec<Card> {
    if count == 0 || count > MAX_CHILDREN {
        return vec![];
    }
    let current = current.min(count - 1);
    let card = |index, rect, hit| Card {
        index,
        rect,
        hit,
        opacity: if hit == Hit::Activate { 1.0 } else { 0.6 },
        radius: if hit == Hit::Activate { 12 } else { 9 },
    };
    match count {
        1 => vec![card(0, [64, 6, 72, 72], Hit::Activate)],
        2 if current == 0 => vec![
            card(1, [102, 14, 56, 56], Hit::Next),
            card(0, [42, 6, 72, 72], Hit::Activate),
        ],
        2 => vec![
            card(0, [42, 14, 56, 56], Hit::Previous),
            card(1, [86, 6, 72, 72], Hit::Activate),
        ],
        _ => vec![
            card(
                (current + count - 1) % count,
                [20, 14, 56, 56],
                Hit::Previous,
            ),
            card((current + 1) % count, [124, 14, 56, 56], Hit::Next),
            card(current, [64, 6, 72, 72], Hit::Activate),
        ],
    }
}
pub fn hit(count: usize, current: usize, x: u16, y: u16) -> Option<Hit> {
    cards(count, current)
        .into_iter()
        .rev()
        .find(|card| {
            let [left, top, width, height] = card.rect;
            // Half-open rectangle edges are a Linux policy, not a recovered native
            // corner/rounding rule. Front cards win overlapping hit regions.
            u32::from(x) >= left
                && u32::from(x) < left + width
                && u32::from(y) >= top
                && u32::from(y) < top + height
        })
        .map(|card| card.hit)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indicator {
    pub index: Option<usize>,
    pub width: u32,
    pub height: u32,
    pub active: bool,
}
#[derive(Clone, Debug)]
pub struct Indicators {
    count: usize,
    current: usize,
    offset: usize,
    left: usize,
    right: usize,
}
impl Indicators {
    pub fn new(count: usize, current: usize, offset: usize) -> Self {
        let count = count.min(MAX_CHILDREN);
        let last = count.saturating_sub(1);
        let offset = offset.min(last);
        let mut this = Self {
            count,
            current: current.min(last),
            offset,
            left: offset.saturating_sub(2).min(2),
            right: last.saturating_sub((offset + 8).min(last)).min(2),
        };
        this.select(current);
        this
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn select(&mut self, current: usize) {
        self.current = current.min(self.count.saturating_sub(1));
        while self.current < self.offset + self.left {
            if self.offset > 0 {
                self.offset -= 1;
                self.right = (self.right + 1).min(2);
            } else {
                self.left = self.left.saturating_sub(1);
            }
        }
        let last = self.count.saturating_sub(1);
        while (self.offset + 8).min(last).saturating_sub(self.right) < self.current {
            if (self.offset + 8).min(last) < last {
                self.offset += 1;
                self.left = (self.left + 1).min(2);
            } else {
                self.right = self.right.saturating_sub(1);
            }
        }
    }
    pub fn items(&self) -> Vec<Indicator> {
        if self.count == 0 {
            return vec![];
        }
        let marker = |size| Indicator {
            index: None,
            width: size,
            height: size,
            active: false,
        };
        let mut items = vec![];
        if self.left == 2 {
            items.push(marker(4));
        }
        if self.left > 0 {
            items.push(marker(6));
        }
        for index in self.offset + self.left
            ..(self.offset + self.left + (9 - self.left - self.right).min(self.count))
        {
            if index >= self.count {
                break;
            }
            items.push(Indicator {
                index: Some(index),
                width: if index == self.current { 12 } else { 8 },
                height: 8,
                active: index == self.current,
            });
        }
        if self.right > 0 {
            items.push(marker(6));
        }
        if self.right == 2 {
            items.push(marker(4));
        }
        items
    }
}
