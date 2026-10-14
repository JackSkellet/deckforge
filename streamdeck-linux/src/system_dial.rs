//! Paced hotkey dial/tap queue, following the 7.6 native queue contract.
//! The limit includes the active stroke; direction changes remove only unstarted
//! strokes. Each stroke has a press interval and an equally long release interval.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub keys: crate::system_keyboard::Chord,
    pub gesture: u8,
    pub interval: Duration,
}
#[derive(Debug, PartialEq)]
pub enum Event {
    Down(String, crate::system_keyboard::Chord),
    Up(String),
}
enum Phase {
    Waiting,
    Pressed,
    Released,
}
struct Queue {
    strokes: VecDeque<Stroke>,
    phase: Phase,
    due: Instant,
}
#[derive(Default)]
pub struct Dials {
    queues: HashMap<String, Queue>,
}
impl Dials {
    pub fn enqueue(
        &mut self,
        context: &str,
        stroke: Stroke,
        count: u8,
        coalesce: bool,
        now: Instant,
    ) {
        if stroke.keys.is_empty() || count == 0 {
            return;
        }
        let queue = self.queues.entry(context.into()).or_insert_with(|| Queue {
            strokes: VecDeque::new(),
            phase: Phase::Waiting,
            due: now,
        });
        let active = usize::from(!matches!(queue.phase, Phase::Waiting));
        while queue.strokes.len() > active
            && queue
                .strokes
                .back()
                .is_some_and(|last| last.gesture != stroke.gesture)
        {
            queue.strokes.pop_back();
        }
        let limit: usize = if coalesce { 3 } else { 5 };
        for _ in 0..usize::from(count).min(limit.saturating_sub(queue.strokes.len())) {
            queue.strokes.push_back(stroke.clone());
        }
    }
    pub fn wait(&self, now: Instant, allow_press: bool) -> Duration {
        self.queues
            .values()
            .filter(|q| allow_press || matches!(q.phase, Phase::Pressed))
            .map(|q| q.due.saturating_duration_since(now))
            .min()
            .unwrap_or(Duration::from_millis(50))
            .min(Duration::from_millis(50))
    }
    pub fn poll(&mut self, now: Instant, allow_press: bool) -> Vec<Event> {
        let mut events = Vec::new();
        self.queues.retain(|context, q| {
            if now < q.due || (!allow_press && !matches!(q.phase, Phase::Pressed)) {
                return true;
            }
            if matches!(q.phase, Phase::Released) {
                q.strokes.pop_front();
                q.phase = Phase::Waiting;
            }
            let Some(stroke) = q.strokes.front() else {
                return false;
            };
            match q.phase {
                Phase::Waiting => {
                    events.push(Event::Down(context.clone(), stroke.keys.clone()));
                    q.phase = Phase::Pressed;
                }
                Phase::Pressed => {
                    events.push(Event::Up(context.clone()));
                    q.phase = Phase::Released;
                }
                Phase::Released => unreachable!(),
            }
            // A delayed worker never compresses a real press/release interval.
            q.due = now + stroke.interval;
            true
        });
        events
    }
    pub fn cancel(&mut self, contexts: &HashSet<String>) -> Vec<Event> {
        let mut events = Vec::new();
        self.queues.retain(|context, q| {
            if !contexts.contains(context) {
                return true;
            }
            if matches!(q.phase, Phase::Pressed) {
                events.push(Event::Up(context.clone()));
            }
            false
        });
        events
    }
    pub fn clear(&mut self) {
        self.queues.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paste_pause_releases_active_strokes_without_starting_waiting_ones() {
        let now = Instant::now();
        let mut d = Dials::default();
        d.enqueue("held", stroke(0), 2, false, now);
        assert_eq!(
            d.poll(now, true),
            vec![Event::Down("held".into(), vec![30].into())]
        );
        d.enqueue("waiting", stroke(1), 1, false, now);
        assert_eq!(
            d.poll(now + Duration::from_millis(30), false),
            vec![Event::Up("held".into())]
        );
        assert!(d.poll(now + Duration::from_secs(1), false).is_empty());
        assert_eq!(
            d.wait(now + Duration::from_secs(1), false),
            Duration::from_millis(50)
        );
        assert_eq!(d.poll(now + Duration::from_secs(1), true).len(), 2);
    }
    fn stroke(gesture: u8) -> Stroke {
        Stroke {
            keys: vec![30 + u16::from(gesture)].into(),
            gesture,
            interval: Duration::from_millis(30),
        }
    }
    #[test]
    fn limits_include_active_stroke_and_reversal_keeps_owned_release() {
        for (coalesce, limit) in [(true, 3), (false, 5)] {
            let now = Instant::now();
            let mut d = Dials::default();
            d.enqueue("a", stroke(0), 127, coalesce, now);
            assert_eq!(d.queues["a"].strokes.len(), limit);
            assert_eq!(
                d.poll(now, true),
                vec![Event::Down("a".into(), vec![30].into())]
            );
            d.enqueue("a", stroke(1), 127, coalesce, now);
            assert_eq!(d.queues["a"].strokes.len(), limit);
            assert_eq!(d.queues["a"].strokes[0].gesture, 0);
            assert!(d.queues["a"].strokes.iter().skip(1).all(|s| s.gesture == 1));
            assert!(d.poll(now + Duration::from_millis(29), true).is_empty());
            assert_eq!(
                d.poll(now + Duration::from_millis(30), true),
                vec![Event::Up("a".into())]
            );
            assert!(d.poll(now + Duration::from_millis(59), true).is_empty());
            assert_eq!(
                d.poll(now + Duration::from_millis(60), true),
                vec![Event::Down("a".into(), vec![31].into())]
            );
        }
    }
    #[test]
    fn tap_replaces_pending_turns_and_cancellation_is_context_scoped() {
        let now = Instant::now();
        let mut d = Dials::default();
        d.enqueue("a", stroke(0), 5, false, now);
        d.enqueue("a", stroke(2), 1, false, now);
        d.enqueue("b", stroke(1), 1, false, now);
        assert_eq!(d.queues["a"].strokes.len(), 1);
        assert_eq!(d.poll(now, true).len(), 2);
        assert_eq!(
            d.cancel(&HashSet::from(["a".into()])),
            vec![Event::Up("a".into())]
        );
        assert_eq!(
            d.poll(now + Duration::from_secs(1), true),
            vec![Event::Up("b".into())]
        );
        assert!(d.poll(now + Duration::from_secs(1), true).is_empty());
        assert!(d.poll(now + Duration::from_millis(1030), true).is_empty());
        assert!(d.queues.is_empty());
    }
    #[test]
    fn empty_strokes_never_allocate_and_cooling_cancellation_does_not_release_twice() {
        let now = Instant::now();
        let mut d = Dials::default();
        d.enqueue(
            "a",
            Stroke {
                keys: vec![].into(),
                ..stroke(0)
            },
            1,
            true,
            now,
        );
        d.enqueue("b", stroke(0), 0, true, now);
        assert!(d.queues.is_empty());
        d.enqueue("a", stroke(0), 1, true, now);
        d.poll(now, true);
        d.poll(now + Duration::from_millis(30), true);
        assert!(d.cancel(&HashSet::from(["a".into()])).is_empty());
        assert!(d.queues.is_empty());
    }
}
