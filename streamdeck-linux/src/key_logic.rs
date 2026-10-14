//! Key Logic's native 300/500 ms gesture timers. Actual key ownership and the
//! single-press 60/15 ms pulse queue remain in the existing host adapter.
use std::{collections::BTreeMap, time::Duration};
pub const UUID: &str = "com.elgato.streamdeck.keys.logic";
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Single(String),
    Down(String, usize),
    Up(String, usize),
}
#[derive(Default)]
struct Gesture {
    pressed: bool,
    single: Option<Duration>,
    hold: Option<Duration>,
    active: Option<usize>,
}
#[derive(Default)]
pub struct Gestures {
    pending: BTreeMap<String, Gesture>,
}
impl Gestures {
    pub fn input(&mut self, parent: &str, pressed: bool, now: Duration) -> Vec<Event> {
        let mut events = self.advance_one(parent, now);
        if !pressed && !self.pending.contains_key(parent) {
            return events;
        }
        let gesture = self.pending.entry(parent.into()).or_default();
        if gesture.pressed == pressed {
            self.prune();
            return events;
        }
        gesture.pressed = pressed;
        if pressed {
            if gesture.single.is_some() && gesture.hold.is_some() {
                gesture.single = None;
                gesture.hold = None;
                gesture.active = Some(1);
                events.push(Event::Down(parent.into(), 1));
            } else if gesture.single.is_none() && gesture.hold.is_none() {
                gesture.single = Some(now + Duration::from_millis(300));
                gesture.hold = Some(now + Duration::from_millis(500));
            }
        } else if let Some(index) = gesture.active.take() {
            events.push(Event::Up(parent.into(), index));
        }
        self.prune();
        events
    }
    fn advance_one(&mut self, parent: &str, now: Duration) -> Vec<Event> {
        let Some(g) = self.pending.get_mut(parent) else {
            return vec![];
        };
        let mut events = Vec::new();
        if g.single.is_some_and(|due| now >= due) {
            g.single = None;
            if !g.pressed && g.hold.is_some() {
                g.hold = None;
                events.push(Event::Single(parent.into()));
            }
        }
        if g.hold.is_some_and(|due| now >= due) {
            g.hold = None;
            if g.pressed {
                g.single = None;
                g.active = Some(2);
                events.push(Event::Down(parent.into(), 2));
            }
        }
        events
    }
    fn prune(&mut self) {
        self.pending.retain(|_, g| {
            g.pressed || g.single.is_some() || g.hold.is_some() || g.active.is_some()
        });
    }
    pub fn poll(&mut self, now: Duration) -> Vec<Event> {
        let parents: Vec<_> = self.pending.keys().cloned().collect();
        let events = parents
            .into_iter()
            .flat_map(|p| self.advance_one(&p, now))
            .collect();
        self.prune();
        events
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.pending
            .values()
            .flat_map(|g| g.single.into_iter().chain(g.hold))
            .min()
    }
    pub fn parents(&self) -> impl Iterator<Item = &String> {
        self.pending.keys()
    }
    pub fn cancel(&mut self, parent: &str) {
        self.pending.remove(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    #[test]
    fn single_double_and_hold_dispatch_only_their_original_position() {
        let mut g = Gestures::default();
        assert!(g.input("a", true, ms(0)).is_empty());
        assert!(g.input("a", false, ms(20)).is_empty());
        assert!(g.poll(ms(299)).is_empty());
        assert_eq!(g.poll(ms(300)), vec![Event::Single("a".into())]);
        assert!(g.next_due().is_none());
        g.input("a", true, ms(1000));
        g.input("a", false, ms(1020));
        assert_eq!(
            g.input("a", true, ms(1100)),
            vec![Event::Down("a".into(), 1)]
        );
        assert_eq!(
            g.input("a", false, ms(1150)),
            vec![Event::Up("a".into(), 1)]
        );
        assert!(g.poll(ms(1600)).is_empty());
        g.input("a", true, ms(2000));
        assert!(g.poll(ms(2300)).is_empty());
        assert_eq!(g.poll(ms(2500)), vec![Event::Down("a".into(), 2)]);
        assert_eq!(
            g.input("a", false, ms(2600)),
            vec![Event::Up("a".into(), 2)]
        );
    }
    #[test]
    fn native_medium_press_gap_cancellation_and_context_isolation() {
        let mut g = Gestures::default();
        g.input("a", true, ms(0));
        g.input("b", true, ms(0));
        assert!(g.input("a", false, ms(400)).is_empty());
        g.cancel("b");
        assert!(g.poll(ms(500)).is_empty());
        assert!(g.parents().next().is_none());
        assert!(g.input("a", false, ms(700)).is_empty());
        g.input("a", true, ms(1000));
        assert!(g.input("a", true, ms(1100)).is_empty());
        assert_eq!(
            g.input("a", false, ms(1500)),
            vec![Event::Down("a".into(), 2), Event::Up("a".into(), 2)]
        );
    }
}
