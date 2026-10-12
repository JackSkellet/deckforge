//! Dial Stack's flat child schema and deferred-selection gestures.
//!
//! Fixture: the shipped 7.6 bundle flsLUxNAo3HxRBo8yay7EbNtFyJC9c,
//! page ECA0299E-0612-46EA-A23F-1A62DDA43FA5, member SHA-256
//! cec9b394b4eaf3b133702d6647f72a5f13a7b7485ffd2ad2d7e01666db266fbc.
//! Native routing was recovered separately by the contracts investigation.
//! The bounded native loader path inspected so far did not restore FastNavigation;
//! other application paths are not excluded. Linux honors the saved flag.
//! Zero-tick rotation is a no-op here because
//! its original branch behavior remains unverified.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::HashSet;

pub const UUID: &str = "com.elgato.streamdeck.dial.stack";
const MAX_CHILDREN: usize = 512;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub uuid: String,
    pub raw: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub children: Vec<Entry>,
    /// Normalized runtime selection. The original raw record remains unchanged.
    pub current: usize,
    pub fast_navigation: bool,
    pub show_stack_icon: bool,
    pub raw: Value,
}

pub fn parse(raw: &Value) -> Result<Stack> {
    ensure!(raw.is_object(), "Dial Stack must be an object");
    if let Some(uuid) = raw.get("UUID") {
        ensure!(
            uuid.as_str() == Some(UUID),
            "Dial Stack UUID does not match its container"
        );
    }
    let actions = raw["Actions"]
        .as_array()
        .context("Dial Stack Actions must be a flat array")?;
    ensure!(
        actions.len() <= MAX_CHILDREN,
        "Dial Stack exceeds {MAX_CHILDREN} children"
    );
    if let Some(settings) = raw.get("Settings") {
        ensure!(
            settings.is_null() || settings.is_object(),
            "Dial Stack Settings must be an object or null"
        );
    }
    let settings = &raw["Settings"];
    let fast_navigation = boolean(settings, "FastNavigation", false)?;
    let show_stack_icon = boolean(settings, "ShowStackIcon", true)?;
    let current = match settings.get("CurrentIdx") {
        None => 0,
        Some(value) => {
            if let Some(index) = value.as_u64() {
                usize::try_from(index)
                    .ok()
                    .filter(|&index| index < actions.len())
                    .unwrap_or(0)
            } else if value.as_i64().is_some_and(|index| index < 0) {
                0
            } else {
                bail!("Dial Stack CurrentIdx must be an integer");
            }
        }
    };
    let mut ids = HashSet::new();
    let mut children = Vec::with_capacity(actions.len());
    for (index, action) in actions.iter().enumerate() {
        ensure!(
            action.is_object(),
            "Dial Stack child {index} must be an object"
        );
        let uuid = action["UUID"]
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 1024)
            .context("Dial Stack child needs a valid UUID")?;
        ensure!(
            uuid != UUID && !uuid.starts_with("com.elgato.streamdeck.multiactions."),
            "Nested Dial Stacks and Multi Actions are unsupported"
        );
        let id = match action.get("ActionID") {
            None => format!("stack:index:{index}"),
            Some(value) => {
                let id = value
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 1024)
                    .context("Dial Stack child ActionID must be a nonempty string")?;
                ensure!(ids.insert(id), "Duplicate Dial Stack child ActionID: {id}");
                format!("stack:id:{id}")
            }
        };
        if let Some(settings) = action.get("Settings") {
            ensure!(
                settings.is_null() || settings.is_object(),
                "Dial Stack child Settings must be an object or null"
            );
        }
        if let Some(state) = action.get("State") {
            ensure!(
                state.as_u64().is_some_and(|state| state <= u32::MAX as u64),
                "Dial Stack child State must be a nonnegative 32-bit integer"
            );
        }
        if let Some(states) = action.get("States") {
            ensure!(
                states
                    .as_array()
                    .is_some_and(|states| states.iter().all(Value::is_object)),
                "Dial Stack child States must be an array of objects"
            );
        }
        children.push(Entry {
            id,
            uuid: uuid.to_owned(),
            raw: action.clone(),
        });
    }
    Ok(Stack {
        children,
        current,
        fast_navigation,
        show_stack_icon,
        raw: raw.clone(),
    })
}
fn boolean(settings: &Value, key: &str, default: bool) -> Result<bool> {
    match settings.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .with_context(|| format!("Dial Stack {key} must be a boolean")),
    }
}

impl Stack {
    pub fn selected_index(&self) -> Option<usize> {
        (self.current < self.children.len()).then_some(self.current)
    }
    /// Render and touch routing use this child, even during pending navigation.
    pub fn current(&self) -> Option<&Entry> {
        self.children.get(self.current)
    }
    pub fn new_index(&self, steps: i64) -> Option<usize> {
        let index = self.selected_index()? as i64;
        let count = self.children.len() as i64;
        Some(((index + steps.rem_euclid(count)) % count) as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    Consumed,
    Forward {
        index: usize,
        ticks: i8,
        pressed: bool,
    },
}
#[derive(Default, Clone, Debug)]
pub struct Gesture {
    pending: Option<usize>,
    pressed: bool,
    cycle_on_release: bool,
}
impl Gesture {
    /// Dial down is consumed. It queues the next child without activating it.
    pub fn down(&mut self, stack: &Stack) {
        self.pending = stack.new_index(1);
        self.pressed = self.pending.is_some();
        self.cycle_on_release = self.pressed;
    }
    pub fn rotate(&mut self, stack: &Stack, ticks: i8, pressed: bool) -> Rotation {
        if ticks == 0 {
            return Rotation::Consumed;
        }
        self.cycle_on_release = false;
        if stack.fast_navigation && pressed {
            if self.pressed
                && let Some(pending) = self.pending.filter(|&index| index < stack.children.len())
            {
                let count = stack.children.len() as i64;
                // Native advances one position by sign, not magnitude. Since
                // down already queued current+1, the first CW tick queues +2.
                self.pending =
                    Some((pending as i64 + i64::from(ticks.signum())).rem_euclid(count) as usize);
            }
            Rotation::Consumed
        } else {
            match stack.selected_index() {
                Some(index) => Rotation::Forward {
                    index,
                    ticks,
                    pressed,
                },
                None => Rotation::Consumed,
            }
        }
    }
    /// Dial up is consumed. Some(index) reports a commit, even if the chosen
    /// index equals the previous one. Caller decides whether lifecycle work is needed.
    /// The caller also owns the native 300 ms post-fast-release rotation deadline.
    pub fn up(&mut self, stack: &mut Stack) -> Option<usize> {
        let selected = if self.pressed
            && stack.children.len() > 1
            && (self.cycle_on_release || stack.fast_navigation)
        {
            self.pending.filter(|&index| index < stack.children.len())
        } else {
            None
        };
        self.reset();
        if let Some(index) = selected {
            stack.current = index;
        }
        selected
    }
    /// Call when the stack leaves its parent slot or its children are replaced.
    pub fn reset(&mut self) {
        self.pending = None;
        self.pressed = false;
        self.cycle_on_release = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Value {
        json!({"ActionID":"c1f6310d-b0cc-4677-b440-3e7616dac524","UUID":UUID,"Name":"Dial Stack","Settings":{"CurrentIdx":1,"FastNavigation":false,"ShowStackIcon":true},"Actions":[
            {"ActionID":"d3335822-991a-4cc5-8aec-31efb73af9f9","UUID":"com.elgato.wave-link.channellevel","Settings":{},"State":0,"States":[{"Title":"Voice Chat"},{}],"Resources":null,"Plugin":{"Version":"3.0.0.248"}},
            {"ActionID":"a39d49ef-542a-427a-b0d9-6f11da66ad95","UUID":"com.elgato.wave-link.channellevel","Settings":{},"State":0,"States":[{"Title":"Voice Chat"},{}],"Resources":null,"Plugin":{"Version":"3.0.0.197"}}
        ]})
    }
    fn make_stack(count: usize, fast: bool) -> Stack {
        parse(&json!({"UUID":UUID,"Settings":{"FastNavigation":fast},"Actions":(0..count).map(|index|json!({"ActionID":format!("child-{index}"),"UUID":"test.encoder"})).collect::<Vec<_>>()})).unwrap()
    }
    #[test]
    fn shipped_flat_children_and_unknown_metadata_are_preserved() {
        let mut raw = fixture();
        raw["FutureField"] = json!({"keep":[1,2,3]});
        let parsed = parse(&raw).unwrap();
        assert_eq!(parsed.current, 1);
        assert!(!parsed.fast_navigation);
        assert!(parsed.show_stack_icon);
        assert_eq!(parsed.children[0].raw, raw["Actions"][0]);
        assert_eq!(parsed.raw, raw);
        assert!(parsed.children[0].id.starts_with("stack:id:"));
        assert_eq!(parsed.new_index(1), Some(0));
        assert_eq!(parsed.raw, raw);
    }
    #[test]
    fn native_position_fallback_and_defaults_do_not_rewrite_raw() {
        for position in [json!(-1), json!(2), json!(u64::MAX)] {
            let mut raw = fixture();
            raw["Settings"]["CurrentIdx"] = position;
            let parsed = parse(&raw).unwrap();
            assert_eq!(parsed.current, 0);
            assert_eq!(parsed.raw, raw);
        }
        let parsed = parse(&json!({"Actions":[{"UUID":"test.encoder"}]})).unwrap();
        assert_eq!(parsed.current, 0);
        assert!(!parsed.fast_navigation);
        assert!(parsed.show_stack_icon);
        assert_eq!(parsed.children[0].id, "stack:index:0");
        let mut raw = fixture();
        raw["Settings"]["CurrentIdx"] = json!(1.5);
        assert!(parse(&raw).is_err());
    }
    #[test]
    fn null_settings_have_effective_defaults_without_rewriting_source() {
        let mut raw = fixture();
        raw["Settings"] = Value::Null;
        raw["Actions"][0]["Settings"] = Value::Null;
        let stack = parse(&raw).unwrap();
        assert_eq!(stack.current, 0);
        assert!(!stack.fast_navigation);
        assert!(stack.show_stack_icon);
        assert_eq!(stack.raw, raw);
        assert_eq!(stack.children[0].raw["Settings"], Value::Null);
    }
    #[test]
    fn malformed_children_flags_duplicates_and_nesting_reject() {
        for raw in [
            json!({"Actions":{}}),
            json!({"Actions":[{}]}),
            json!({"Actions":[{"UUID":UUID}]}),
            json!({"Actions":[{"UUID":"com.elgato.streamdeck.multiactions.routine"}]}),
            json!({"Actions":[{"UUID":"x","ActionID":"duplicate"},{"UUID":"x","ActionID":"duplicate"}]}),
            json!({"Actions":[{"UUID":"x","States":"bad"}]}),
            json!({"Actions":[],"Settings":{"FastNavigation":1}}),
        ] {
            assert!(parse(&raw).is_err(), "{raw}");
        }
        assert!(parse(&json!({"Actions":vec![json!({"UUID":"x"});513]})).is_err());
    }
    #[test]
    fn cycling_wraps_large_and_negative_steps_and_empty_stack_is_safe() {
        let mut parsed = make_stack(3, false);
        assert_eq!(parsed.new_index(-1), Some(2));
        parsed.current = 2;
        assert_eq!(parsed.new_index(1), Some(0));
        parsed.current = 0;
        assert_eq!(
            parsed.new_index(i64::MIN),
            Some(i64::MIN.rem_euclid(3) as usize)
        );
        let mut empty = make_stack(0, false);
        assert_eq!(empty.selected_index(), None);
        assert_eq!(empty.new_index(1), None);
        let mut gesture = Gesture::default();
        gesture.down(&empty);
        assert_eq!(gesture.rotate(&empty, 1, true), Rotation::Consumed);
        assert_eq!(gesture.up(&mut empty), None);
    }
    #[test]
    fn click_cycles_only_on_release_and_normal_rotation_keeps_current_child() {
        let mut stack = make_stack(3, false);
        let mut gesture = Gesture::default();
        gesture.down(&stack);
        assert_eq!(stack.selected_index(), Some(0));
        assert_eq!(gesture.up(&mut stack), Some(1));
        gesture.down(&stack);
        assert_eq!(
            gesture.rotate(&stack, -4, true),
            Rotation::Forward {
                index: 1,
                ticks: -4,
                pressed: true
            }
        );
        assert_eq!(gesture.up(&mut stack), None);
        assert_eq!(stack.current, 1);
        assert_eq!(
            gesture.rotate(&stack, 3, false),
            Rotation::Forward {
                index: 1,
                ticks: 3,
                pressed: false
            }
        );
    }
    #[test]
    fn fast_navigation_uses_sign_and_native_first_clockwise_skip() {
        let mut stack = make_stack(4, true);
        let mut gesture = Gesture::default();
        gesture.down(&stack);
        assert_eq!(gesture.rotate(&stack, 7, true), Rotation::Consumed);
        assert_eq!(stack.selected_index(), Some(0));
        assert_eq!(stack.current().unwrap().id, "stack:id:child-0");
        assert_eq!(gesture.up(&mut stack), Some(2));
        stack.current = 0;
        gesture.down(&stack);
        assert_eq!(gesture.rotate(&stack, -9, true), Rotation::Consumed);
        assert_eq!(gesture.up(&mut stack), Some(0));
    }
    #[test]
    fn zero_rotation_preserves_click_and_reset_or_orphan_release_never_cycles() {
        let mut stack = make_stack(3, false);
        let mut gesture = Gesture::default();
        assert_eq!(gesture.up(&mut stack), None);
        gesture.down(&stack);
        assert_eq!(gesture.rotate(&stack, 0, true), Rotation::Consumed);
        assert_eq!(gesture.up(&mut stack), Some(1));
        gesture.down(&stack);
        gesture.reset();
        assert_eq!(gesture.up(&mut stack), None);
        assert_eq!(stack.current, 1);
        let mut single = make_stack(1, false);
        gesture.down(&single);
        assert_eq!(gesture.up(&mut single), None);
    }
}
