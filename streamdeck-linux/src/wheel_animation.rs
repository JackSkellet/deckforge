//! Action Wheel carousel geometry recovered from the shipped 7.6 native animator.
use crate::action_wheel::{MAX_CHILDREN, Movement};
use std::time::Duration;

pub const FRAME_INTERVAL: Duration = Duration::from_millis(40);
const LEFT: [f64; 4] = [20., 14., 56., 56.];
const FRONT: [f64; 4] = [64., 6., 72., 72.];
const RIGHT: [f64; 4] = [124., 14., 56., 56.];
const FAR_LEFT: [f64; 4] = [10., 20., 44., 44.];
const FAR_RIGHT: [f64; 4] = [146., 20., 44., 44.];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub index: usize,
    pub rect: [f64; 4],
    pub opacity: f64,
    pub depth: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Carousel {
    pub count: usize,
    pub target: usize,
    from: usize,
    backwards: bool,
    started: Duration,
}
impl Carousel {
    pub fn new(count: usize, from: usize, movement: Movement, started: Duration) -> Option<Self> {
        if !(2..=MAX_CHILDREN).contains(&count)
            || from >= count
            || movement.index >= count
            || movement.delta == 0
        {
            return None;
        }
        Some(Self {
            count,
            from,
            target: movement.index,
            backwards: movement.delta < 0,
            started,
        })
    }
    pub fn duration(self) -> Duration {
        Duration::from_millis(if self.count == 2 { 160 } else { 320 })
    }
    pub fn end(self) -> Duration {
        self.started.saturating_add(self.duration())
    }
    pub fn next_frame(self, now: Duration) -> Option<Duration> {
        if now >= self.end() {
            return None;
        }
        let elapsed = now.saturating_sub(self.started).as_millis() as u64;
        Some(
            self.started
                .saturating_add(Duration::from_millis(
                    (elapsed / FRAME_INTERVAL.as_millis() as u64 + 1)
                        * FRAME_INTERVAL.as_millis() as u64,
                ))
                .min(self.end()),
        )
    }
    /// Frozen image indices in the original animator's object order. Duplicates
    /// on short wheels share a snapshot but remain separate moving cards.
    pub fn indices(self) -> Vec<usize> {
        if self.count == 2 {
            return vec![0, 1];
        }
        let offsets = if self.backwards {
            [-1, 0, 1, 2]
        } else {
            [-2, -1, 0, 1]
        };
        offsets
            .into_iter()
            .map(|offset| self.index(offset))
            .collect()
    }
    fn index(self, offset: i32) -> usize {
        (self.target as i32 + offset).rem_euclid(self.count as i32) as usize
    }
    pub fn cards(self, now: Duration) -> Option<Vec<Card>> {
        if now < self.started || now >= self.end() {
            return None;
        }
        let t =
            ease(now.saturating_sub(self.started).as_secs_f64() / self.duration().as_secs_f64());
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        let card = |index, from: [f64; 4], to: [f64; 4], opacity: [f64; 2], depth: [f64; 2]| Card {
            index,
            rect: std::array::from_fn(|i| lerp(from[i], to[i])),
            opacity: lerp(opacity[0], opacity[1]),
            depth: lerp(depth[0], depth[1]),
        };
        let mut cards = if self.count == 2 {
            let a = [42., 6., 72., 72.];
            let b = [42., 14., 56., 56.];
            let c = [102., 14., 56., 56.];
            let d = [86., 6., 72., 72.];
            // Native Animation2 chooses its direction from the old selection,
            // even when a multi-step batch wrapped back to that same index.
            if self.from == 0 {
                vec![
                    card(0, a, b, [1., 0.6], [1., 0.]),
                    card(1, c, d, [0.6, 1.], [0., 1.]),
                ]
            } else {
                vec![
                    card(0, b, a, [0.6, 1.], [0., 1.]),
                    card(1, d, c, [1., 0.6], [1., 0.]),
                ]
            }
        } else if self.backwards {
            vec![
                card(self.index(-1), FAR_LEFT, LEFT, [0., 0.6], [0., 1.]),
                card(self.target, LEFT, FRONT, [0.6, 1.], [1., 2.]),
                card(self.index(1), FRONT, RIGHT, [1., 0.6], [2., 1.]),
                card(self.index(2), RIGHT, FAR_RIGHT, [0.6, 0.], [1., 0.]),
            ]
        } else {
            vec![
                card(self.index(-2), LEFT, FAR_LEFT, [0.6, 0.], [1., 0.]),
                card(self.index(-1), FRONT, LEFT, [1., 0.6], [2., 1.]),
                card(self.target, RIGHT, FRONT, [0.6, 1.], [1., 2.]),
                card(self.index(1), FAR_RIGHT, RIGHT, [0., 0.6], [0., 1.]),
            ]
        };
        // Qt's stable ascending depth sort retains object order at ties.
        cards.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        Some(cards)
    }
}
/// Cubic Bézier (0,0), (.3,.8), (.3,.8), (1,1), evaluated by time (x).
pub fn ease(time: f64) -> f64 {
    if time <= 0. {
        return 0.;
    }
    if time >= 1. {
        return 1.;
    }
    let (mut low, mut high) = (0., 1.);
    for _ in 0..32 {
        let u = (low + high) / 2.;
        let x = 0.9 * u - 0.9 * u * u + u * u * u;
        if x < time {
            low = u;
        } else {
            high = u;
        }
    }
    let u = (low + high) / 2.;
    2.4 * u - 2.4 * u * u + u * u * u
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_durations_curve_and_last_deadline_are_preserved() {
        let two = Carousel::new(2, 0, Movement { index: 1, delta: 1 }, Duration::ZERO).unwrap();
        let many = Carousel::new(6, 0, Movement { index: 4, delta: 4 }, Duration::ZERO).unwrap();
        assert_eq!(two.duration(), Duration::from_millis(160));
        assert_eq!(many.duration(), Duration::from_millis(320));
        assert_eq!(
            many.next_frame(Duration::from_millis(319)),
            Some(Duration::from_millis(320))
        );
        assert_eq!(many.next_frame(Duration::from_millis(320)), None);
        assert!(many.cards(Duration::from_millis(320)).is_none());
        // At Bézier parameter .5, x=.35 and y=.725. This also guards against
        // treating the control-point parameter as elapsed time.
        assert!((ease(0.35) - 0.725).abs() < 1e-8);
        assert_eq!(ease(0.), 0.);
        assert_eq!(ease(1.), 1.);
    }
    #[test]
    fn two_cards_change_size_depth_and_opacity() {
        let motion = Carousel::new(2, 0, Movement { index: 1, delta: 1 }, Duration::ZERO).unwrap();
        let first = motion.cards(Duration::ZERO).unwrap();
        assert_eq!(first.last().unwrap().index, 0);
        assert_eq!(first.last().unwrap().rect, [42., 6., 72., 72.]);
        let middle = motion.cards(Duration::from_millis(80)).unwrap();
        assert_eq!(middle.last().unwrap().index, 1);
        let finish = motion.cards(Duration::from_millis(159)).unwrap();
        assert!((finish.last().unwrap().rect[0] - 86.).abs() < 0.1);
        assert!(finish.last().unwrap().opacity > 0.99);
    }
    #[test]
    fn larger_batches_use_target_neighbors_and_reverse_has_its_own_templates() {
        let forward = Carousel::new(6, 0, Movement { index: 4, delta: 4 }, Duration::ZERO).unwrap();
        let reverse = Carousel::new(
            6,
            0,
            Movement {
                index: 5,
                delta: -1,
            },
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(forward.indices(), vec![2, 3, 4, 5]);
        assert_eq!(reverse.indices(), vec![4, 5, 0, 1]);
        let first = forward.cards(Duration::ZERO).unwrap();
        assert_eq!(first.last().unwrap().index, 3);
        assert_eq!(first.last().unwrap().rect, FRONT);
        let last = reverse.cards(Duration::from_millis(319)).unwrap();
        assert_eq!(last.last().unwrap().index, 5);
        assert!((last.last().unwrap().rect[0] - 64.).abs() < 0.1);
    }
}
