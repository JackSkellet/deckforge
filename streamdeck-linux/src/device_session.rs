//! Reconnecting device ownership and latest-frame delivery. No reset or firmware writes.
use crate::hardware::{Deck, InputEvent};
use anyhow::{Result, ensure};
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub enum Draw {
    Key(u8, DynamicImage),
    Strip(DynamicImage),
    Controls(Controls),
    Stop,
}

/// Explicit per-device overrides. None never causes a default value to be sent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Controls {
    pub brightness: Option<u8>,
    pub sleep_seconds: Option<u32>,
}

#[derive(Debug)]
pub enum Event {
    Connected {
        serial: String,
        firmware: String,
    },
    Disconnected {
        reason: String,
    },
    Retry {
        reason: String,
    },
    Input(InputEvent),
    /// Sleep is getter-verified; brightness means the HID setter was accepted,
    /// not physical brightness measurement or a readback that the protocol lacks.
    Controls {
        controls: Controls,
        error: Option<String>,
    },
}

pub struct Connection {
    pub deck: Deck,
    pub firmware: String,
}

pub fn connect(serial: Option<&str>) -> Result<Connection> {
    let deck = Deck::connect(serial)?;
    let firmware = deck.firmware_version()?;
    Ok(Connection { deck, firmware })
}

pub fn spawn(
    initial: Option<Connection>,
    serial: Option<String>,
    draw_rx: Receiver<Draw>,
    events: UnboundedSender<Event>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        worker(
            initial.map(|c| (c.deck, c.firmware)),
            serial,
            draw_rx,
            events,
            |serial| connect(serial).map(|c| (c.deck, c.firmware)),
        );
    })
}

trait Transport {
    fn serial(&self) -> &str;
    fn read(&mut self, timeout_ms: i32) -> Result<Vec<InputEvent>>;
    fn write(&mut self, index: usize, image: &DynamicImage) -> Result<()>;
    fn sleep_seconds(&mut self) -> Result<u32>;
    fn set_brightness(&mut self, brightness: u8) -> Result<()>;
    fn set_sleep_seconds(&mut self, seconds: u32) -> Result<()>;
}
impl Transport for Deck {
    fn sleep_seconds(&mut self) -> Result<u32> {
        Deck::sleep_seconds(self)
    }
    fn set_brightness(&mut self, brightness: u8) -> Result<()> {
        Deck::set_brightness(self, brightness)
    }
    fn set_sleep_seconds(&mut self, seconds: u32) -> Result<()> {
        Deck::set_sleep_seconds(self, seconds)
    }
    fn serial(&self) -> &str {
        self.serial()
    }
    fn read(&mut self, timeout_ms: i32) -> Result<Vec<InputEvent>> {
        self.read_events(timeout_ms)
    }
    fn write(&mut self, index: usize, image: &DynamicImage) -> Result<()> {
        if index == 8 {
            self.set_touchscreen_image(image)
        } else {
            self.set_key_image(index as u8, image)
        }
    }
}

const RETRY: Duration = Duration::from_secs(1);
struct Session<T> {
    device: Option<(T, String)>,
    serial: Option<String>,
    announce: bool,
    desired: [Option<Arc<DynamicImage>>; 9],
    sent: [Option<Arc<DynamicImage>>; 9],
    cursor: usize,
    next_retry: Duration,
    retry_reason: Option<String>,
    desired_controls: Controls,
    attempted_controls: Controls,
    observed_controls: Controls,
    control_errors: [Option<String>; 2],
    control_cursor: usize,
}
impl<T: Transport> Session<T> {
    fn new(initial: Option<(T, String)>, serial: Option<String>) -> Self {
        Self {
            device: initial,
            serial,
            announce: true,
            desired: std::array::from_fn(|_| None),
            sent: std::array::from_fn(|_| None),
            cursor: 0,
            next_retry: Duration::ZERO,
            retry_reason: None,
            desired_controls: Controls::default(),
            attempted_controls: Controls::default(),
            observed_controls: Controls::default(),
            control_errors: [None, None],
            control_cursor: 0,
        }
    }

    // Invalid key indices cannot enter the nine-frame cache.
    fn cache(&mut self, draw: Draw) -> bool {
        let (index, image) = match draw {
            Draw::Controls(controls) => {
                // A new command can retry a failed field. Successful duplicates
                // remain coalesced; None withdraws an override without resetting it.
                if controls.brightness == self.attempted_controls.brightness
                    && controls.brightness != self.observed_controls.brightness
                {
                    self.attempted_controls.brightness = None;
                }
                if controls.sleep_seconds == self.attempted_controls.sleep_seconds
                    && controls.sleep_seconds != self.observed_controls.sleep_seconds
                {
                    self.attempted_controls.sleep_seconds = None;
                }
                self.desired_controls = controls;
                return false;
            }
            Draw::Stop => return true,
            Draw::Key(index, image) if index < 8 => (usize::from(index), image),
            Draw::Strip(image) => (8, image),
            Draw::Key(_, _) => return false,
        };
        if !self.desired[index]
            .as_ref()
            .is_some_and(|old| same(old, &image))
        {
            self.desired[index] = Some(Arc::new(image));
        }
        false
    }

    fn tick(
        &mut self,
        now: Duration,
        open: &mut impl FnMut(Option<&str>) -> Result<(T, String)>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        if self.device.is_none() {
            if now < self.next_retry {
                return events;
            }
            match open(self.serial.as_deref()) {
                Ok(connection) => {
                    self.device = Some(connection);
                    self.announce = true;
                }
                Err(error) => {
                    self.retry(now, error, &mut events);
                    return events;
                }
            }
        }
        if self.announce {
            let (device, firmware) = self.device.as_ref().unwrap();
            let serial = device.serial().to_owned();
            let valid = (|| {
                ensure!(
                    !serial.is_empty(),
                    "connected device has no serial; cannot safely reconnect"
                );
                ensure!(
                    self.serial
                        .as_ref()
                        .is_none_or(|selected| selected == &serial),
                    "connected device serial does not match selected device"
                );
                Ok(())
            })();
            if let Err(error) = valid {
                self.device = None;
                self.retry(now, error, &mut events);
                return events;
            }
            self.serial = Some(serial.clone());
            events.push(Event::Connected {
                serial,
                firmware: firmware.clone(),
            });
            self.announce = false;
            self.retry_reason = None;
            self.observed_controls = Controls::default();
            self.attempted_controls = Controls::default();
            self.control_errors = [None, None];
            match self.device.as_mut().unwrap().0.sleep_seconds() {
                Ok(seconds) if seconds <= i32::MAX as u32 => {
                    self.observed_controls.sleep_seconds = Some(seconds)
                }
                Ok(_) => {
                    self.control_errors[1] =
                        Some("device sleep duration exceeds signed 32-bit seconds".into())
                }
                Err(error) => self.control_errors[1] = Some(format!("{error:#}")),
            }
            events.push(self.controls_event());
        }
        // Poll before each image, rather than holding input behind a full replay.
        let timeout = if self.dirty().is_some() { 0 } else { 20 };
        match self.device.as_mut().unwrap().0.read(timeout) {
            Ok(input) => events.extend(input.into_iter().map(Event::Input)),
            Err(error) => {
                events.push(self.disconnect(now, error));
                return events;
            }
        }
        // At most one feature update per poll, with independent image progress.
        // Metadata failures do not evict a functioning input/image transport.
        if let Some(event) = self.write_control() {
            events.push(event);
        }
        if let Err(error) = self.write_one() {
            events.push(self.disconnect(now, error));
        }
        events
    }

    fn retry(&mut self, now: Duration, error: anyhow::Error, events: &mut Vec<Event>) {
        self.next_retry = now + RETRY;
        let reason = format!("{error:#}");
        if self.retry_reason.as_ref() != Some(&reason) {
            events.push(Event::Retry {
                reason: reason.clone(),
            });
            self.retry_reason = Some(reason);
        }
    }
    fn disconnect(&mut self, now: Duration, error: anyhow::Error) -> Event {
        self.device = None;
        self.sent.fill(None);
        self.attempted_controls = Controls::default();
        self.next_retry = now + RETRY;
        Event::Disconnected {
            reason: format!("{error:#}"),
        }
    }
    fn controls_event(&self) -> Event {
        let errors: Vec<_> = self
            .control_errors
            .iter()
            .flatten()
            .map(String::as_str)
            .collect();
        Event::Controls {
            controls: self.observed_controls,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
        }
    }
    fn write_control(&mut self) -> Option<Event> {
        let (device, _) = self.device.as_mut()?;
        for offset in 0..2 {
            let field = (self.control_cursor + offset) % 2;
            if field == 0 {
                let Some(brightness) = self.desired_controls.brightness else {
                    continue;
                };
                if self.attempted_controls.brightness == Some(brightness) {
                    continue;
                }
                self.attempted_controls.brightness = Some(brightness);
                let result = (|| {
                    ensure!(brightness <= 100, "brightness must be between 0 and 100");
                    device.set_brightness(brightness)
                })();
                match result {
                    Ok(()) => {
                        self.observed_controls.brightness = Some(brightness);
                        self.control_errors[0] = None;
                    }
                    Err(error) => {
                        self.observed_controls.brightness = None;
                        self.control_errors[0] = Some(format!("{error:#}"));
                    }
                }
            } else {
                let Some(seconds) = self.desired_controls.sleep_seconds else {
                    continue;
                };
                if self.attempted_controls.sleep_seconds == Some(seconds) {
                    continue;
                }
                self.attempted_controls.sleep_seconds = Some(seconds);
                let result = (|| {
                    ensure!(
                        seconds <= i32::MAX as u32,
                        "sleep duration exceeds signed 32-bit seconds"
                    );
                    device.set_sleep_seconds(seconds)
                })();
                match result {
                    Ok(()) => {
                        self.observed_controls.sleep_seconds = Some(seconds);
                        self.control_errors[1] = None;
                    }
                    Err(error) => {
                        self.observed_controls.sleep_seconds = None;
                        self.control_errors[1] = Some(format!("{error:#}"));
                    }
                }
            }
            self.control_cursor = (field + 1) % 2;
            return Some(self.controls_event());
        }
        None
    }
    fn dirty(&self) -> Option<usize> {
        (0..9)
            .map(|offset| (self.cursor + offset) % 9)
            .find(|&index| {
                self.desired[index].as_ref().is_some_and(|wanted| {
                    self.sent[index]
                        .as_ref()
                        .is_none_or(|sent| !Arc::ptr_eq(sent, wanted) && !same(sent, wanted))
                })
            })
    }
    fn write_one(&mut self) -> Result<bool> {
        let Some(index) = self.dirty() else {
            return Ok(false);
        };
        let Some((device, _)) = self.device.as_mut() else {
            return Ok(false);
        };
        let image = self.desired[index].as_ref().unwrap();
        device.write(index, image)?;
        self.sent[index] = Some(image.clone());
        self.cursor = (index + 1) % 9;
        Ok(true)
    }
    fn stop(&mut self, now: Duration) -> Vec<Event> {
        let mut events = Vec::new();
        while let Some(event) = self.write_control() {
            events.push(event);
        }
        loop {
            match self.write_one() {
                Ok(true) => {}
                Ok(false) => return events,
                Err(error) => {
                    events.push(self.disconnect(now, error));
                    return events;
                }
            }
        }
    }
}
fn same(a: &DynamicImage, b: &DynamicImage) -> bool {
    a.width() == b.width()
        && a.height() == b.height()
        && a.color() == b.color()
        && a.as_bytes() == b.as_bytes()
}

fn worker<T: Transport>(
    initial: Option<(T, String)>,
    serial: Option<String>,
    draw_rx: Receiver<Draw>,
    events: UnboundedSender<Event>,
    mut open: impl FnMut(Option<&str>) -> Result<(T, String)>,
) {
    let start = Instant::now();
    let mut session = Session::new(initial, serial);
    loop {
        if events.is_closed() {
            return;
        }
        // Bounded draining coalesces bursts without letting a continuous producer
        // starve HID input. Stop is an ordered barrier for all preceding frames.
        for _ in 0..256 {
            match draw_rx.try_recv() {
                Ok(draw) => {
                    if session.cache(draw) {
                        for event in session.stop(start.elapsed()) {
                            let _ = events.send(event);
                        }
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let now = start.elapsed();
        let attempted = session.device.is_some() || now >= session.next_retry;
        let outgoing = session.tick(now, &mut open);
        if attempted && session.device.is_none() {
            // Measure the retry gap after a potentially slow HID open/read/write,
            // so repeated failures cannot turn into immediate connection attempts.
            session.next_retry = start.elapsed() + RETRY;
        }
        for event in outgoing {
            if events.send(event).is_err() {
                return;
            }
        }
        if session.device.is_none() {
            let wait = session.next_retry.saturating_sub(start.elapsed());
            match draw_rx.recv_timeout(wait) {
                Ok(draw) => {
                    if session.cache(draw) {
                        return;
                    } // No reconnect on Stop.
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::{collections::VecDeque, sync::Mutex};
    #[derive(Default)]
    struct Calls {
        writes: Vec<(usize, u8)>,
        reads: usize,
        features: Vec<(u8, u32)>,
        sleep_queries: usize,
    }
    struct Fake {
        serial: String,
        calls: Arc<Mutex<Calls>>,
        read_error: bool,
        write_error: bool,
        inputs: Vec<InputEvent>,
        sleep: u32,
        sleep_read_error: bool,
        brightness_error: bool,
        sleep_write_error: bool,
    }
    impl Transport for Fake {
        fn sleep_seconds(&mut self) -> Result<u32> {
            self.calls.lock().unwrap().sleep_queries += 1;
            ensure!(!self.sleep_read_error, "sleep getter failed");
            Ok(self.sleep)
        }
        fn set_brightness(&mut self, brightness: u8) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .features
                .push((0, u32::from(brightness)));
            ensure!(!self.brightness_error, "brightness setter failed");
            Ok(())
        }
        fn set_sleep_seconds(&mut self, seconds: u32) -> Result<()> {
            self.calls.lock().unwrap().features.push((1, seconds));
            ensure!(!self.sleep_write_error, "sleep readback mismatch");
            self.sleep = seconds;
            self.sleep_seconds()?;
            Ok(())
        }
        fn serial(&self) -> &str {
            &self.serial
        }
        fn read(&mut self, _: i32) -> Result<Vec<InputEvent>> {
            self.calls.lock().unwrap().reads += 1;
            ensure!(!self.read_error, "read failed");
            Ok(std::mem::take(&mut self.inputs))
        }
        fn write(&mut self, index: usize, image: &DynamicImage) -> Result<()> {
            ensure!(!self.write_error, "write failed");
            self.calls
                .lock()
                .unwrap()
                .writes
                .push((index, image.as_bytes()[0]));
            Ok(())
        }
    }
    fn fake(serial: &str) -> (Fake, Arc<Mutex<Calls>>) {
        let calls = Arc::new(Mutex::new(Calls::default()));
        (
            Fake {
                serial: serial.into(),
                calls: calls.clone(),
                read_error: false,
                write_error: false,
                inputs: Vec::new(),
                sleep: 600,
                sleep_read_error: false,
                brightness_error: false,
                sleep_write_error: false,
            },
            calls,
        )
    }
    fn frame(value: u8) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([value, 0, 0, 255])))
    }
    fn never(_: Option<&str>) -> Result<(Fake, String)> {
        panic!("unexpected reconnect")
    }

    #[test]
    fn initial_connection_precedes_input_and_bursts_keep_latest_unique_frame() {
        let (mut deck, calls) = fake("A");
        deck.inputs.push(InputEvent::Key {
            index: 0,
            pressed: true,
        });
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Key(0, frame(1)));
        session.cache(Draw::Key(0, frame(2)));
        session.cache(Draw::Strip(frame(3)));
        let events = session.tick(Duration::ZERO, &mut never);
        assert!(
            matches!(&events[..], [Event::Connected { serial, .. }, Event::Controls { error: None, .. }, Event::Input(_)] if serial == "A")
        );
        session.tick(Duration::ZERO, &mut never);
        session.cache(Draw::Key(0, frame(2)));
        session.tick(Duration::ZERO, &mut never);
        assert_eq!(calls.lock().unwrap().writes, [(0, 2), (8, 3)]);
        assert_eq!(calls.lock().unwrap().reads, 3);
    }

    #[test]
    fn failed_startup_is_rate_limited_and_repeated_errors_are_quiet() {
        let mut session = Session::<Fake>::new(None, Some("A".into()));
        let mut attempts = 0;
        let mut open = |serial: Option<&str>| {
            assert_eq!(serial, Some("A"));
            attempts += 1;
            anyhow::bail!("absent")
        };
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut open)[..],
            [Event::Retry { .. }]
        ));
        assert!(
            session
                .tick(Duration::from_millis(999), &mut open)
                .is_empty()
        );
        assert!(session.tick(RETRY, &mut open).is_empty());
        assert_eq!(attempts, 2);
        assert!(
            matches!(&session.tick(RETRY * 2, &mut |_| anyhow::bail!("permission"))[..], [Event::Retry { reason }] if reason == "permission")
        );
    }

    #[test]
    fn read_or_write_failure_disconnects_once_and_replays_latest_to_same_serial() {
        for read_failure in [false, true] {
            let (deck, calls) = fake("A");
            let mut session = Session::new(Some((deck, "1".into())), None);
            for index in 0..8 {
                session.cache(Draw::Key(index, frame(index)));
            }
            session.cache(Draw::Strip(frame(9)));
            session.tick(Duration::ZERO, &mut never);
            session.device.as_mut().unwrap().0.read_error = read_failure;
            session.device.as_mut().unwrap().0.write_error = !read_failure;
            assert!(matches!(
                &session.tick(Duration::ZERO, &mut never)[..],
                [Event::Disconnected { .. }]
            ));
            assert!(
                session
                    .tick(Duration::from_millis(500), &mut never)
                    .is_empty()
            );
            session.cache(Draw::Key(0, frame(42)));
            let (replacement, writes) = fake("A");
            let mut replacements = VecDeque::from([(replacement, "2".into())]);
            let mut open = |serial: Option<&str>| {
                assert_eq!(serial, Some("A"));
                Ok(replacements.pop_front().unwrap())
            };
            assert!(
                matches!(&session.tick(RETRY, &mut open)[..], [Event::Connected { firmware, .. }, Event::Controls { .. }] if firmware == "2")
            );
            for _ in 0..8 {
                session.tick(RETRY, &mut never);
            }
            let written = &writes.lock().unwrap().writes;
            assert_eq!(written.len(), 9);
            assert!(written.contains(&(0, 42)) && written.contains(&(8, 9)));
            assert_eq!(calls.lock().unwrap().writes, [(0, 0)]);
        }
    }

    #[test]
    fn other_serial_is_never_accepted_and_stop_does_not_reconnect() {
        let mut session = Session::new(None, Some("A".into()));
        session.cache(Draw::Key(0, frame(7)));
        let (wrong, calls) = fake("B");
        let mut wrong = Some(wrong);
        let events = session.tick(Duration::ZERO, &mut |_| {
            Ok((wrong.take().unwrap(), "1".into()))
        });
        assert!(matches!(&events[..], [Event::Retry { .. }]));
        assert!(session.stop(RETRY).is_empty());
        assert_eq!(calls.lock().unwrap().reads, 0);
        assert!(calls.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn stop_flushes_latest_frames_but_failure_does_not_retry() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Key(0, frame(1)));
        session.cache(Draw::Key(0, frame(2)));
        session.cache(Draw::Strip(frame(3)));
        assert!(session.stop(Duration::ZERO).is_empty());
        assert_eq!(calls.lock().unwrap().writes, [(0, 2), (8, 3)]);
        session.cache(Draw::Key(1, frame(4)));
        session.device.as_mut().unwrap().0.write_error = true;
        assert!(matches!(
            &session.stop(Duration::ZERO)[..],
            [Event::Disconnected { .. }]
        ));
        assert!(session.stop(RETRY).is_empty());
    }

    #[test]
    fn queued_offline_stop_and_closed_channel_never_open_a_device() {
        for send_stop in [false, true] {
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(Draw::Key(0, frame(1))).unwrap();
            if send_stop {
                tx.send(Draw::Stop).unwrap();
            }
            drop(tx);
            let (events, _rx) = tokio::sync::mpsc::unbounded_channel();
            worker(None, None, rx, events, never);
        }
    }

    #[test]
    fn stop_queued_during_failed_open_wakes_offline_worker_without_retry() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (events, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut attempts = 0;
        worker::<Fake>(None, None, rx, events, |_| {
            attempts += 1;
            tx.send(Draw::Key(0, frame(1))).unwrap();
            tx.send(Draw::Stop).unwrap();
            anyhow::bail!("absent")
        });
        assert_eq!(attempts, 1);
        assert!(matches!(event_rx.try_recv(), Ok(Event::Retry { .. })));
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn default_controls_only_query_sleep_and_do_not_invent_brightness_or_write_defaults() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Controls(Controls::default()));
        let events = session.tick(Duration::ZERO, &mut never);
        assert!(matches!(
            &events[..],
            [
                Event::Connected { .. },
                Event::Controls {
                    controls: Controls {
                        brightness: None,
                        sleep_seconds: Some(600)
                    },
                    error: None,
                }
            ]
        ));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert!(session.stop(Duration::ZERO).is_empty());
        assert_eq!(calls.lock().unwrap().sleep_queries, 1);
        assert!(calls.lock().unwrap().features.is_empty());
        assert_eq!(
            serde_json::from_str::<Controls>("{}").unwrap(),
            Controls::default()
        );
    }

    #[test]
    fn control_bursts_coalesce_without_starving_input_or_images_and_success_is_deduplicated() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        for brightness in [10, 20, 30] {
            session.cache(Draw::Controls(Controls {
                brightness: Some(brightness),
                sleep_seconds: Some(0),
            }));
        }
        session.cache(Draw::Key(0, frame(1)));
        session.cache(Draw::Strip(frame(2)));
        session.tick(Duration::ZERO, &mut never);
        assert_eq!(calls.lock().unwrap().features, [(0, 30)]);
        assert_eq!(calls.lock().unwrap().writes, [(0, 1)]);
        // An arriving brightness update must not starve the pending sleep field.
        let final_controls = Controls {
            brightness: Some(40),
            sleep_seconds: Some(0),
        };
        session.cache(Draw::Controls(final_controls));
        session.tick(Duration::ZERO, &mut never);
        assert_eq!(calls.lock().unwrap().features, [(0, 30), (1, 0)]);
        session.tick(Duration::ZERO, &mut never);
        session.cache(Draw::Controls(final_controls));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert_eq!(calls.lock().unwrap().features, [(0, 30), (1, 0), (0, 40)]);
        assert_eq!(calls.lock().unwrap().writes, [(0, 1), (8, 2)]);
        assert_eq!(calls.lock().unwrap().reads, 4);
        assert_eq!(session.observed_controls, final_controls);
        session.cache(Draw::Controls(Controls::default()));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert_eq!(calls.lock().unwrap().features.len(), 3);
        session.cache(Draw::Controls(final_controls));
        session.cache(Draw::Controls(Controls {
            brightness: Some(50),
            ..final_controls
        }));
        session.cache(Draw::Controls(final_controls));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert_eq!(calls.lock().unwrap().features.len(), 3);
    }

    #[test]
    fn explicit_overrides_replay_after_reconnect_but_withdrawn_fields_do_not() {
        let (deck, _) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Controls(Controls {
            brightness: Some(37),
            sleep_seconds: Some(900),
        }));
        session.tick(Duration::ZERO, &mut never);
        session.tick(Duration::ZERO, &mut never);
        session.device.as_mut().unwrap().0.read_error = true;
        session.tick(Duration::ZERO, &mut never);
        let (replacement, calls) = fake("A");
        let mut replacement = Some(replacement);
        let events = session.tick(RETRY, &mut |_| {
            Ok((replacement.take().unwrap(), "1".into()))
        });
        assert!(matches!(
            &events[1],
            Event::Controls {
                controls: Controls {
                    brightness: None,
                    sleep_seconds: Some(600)
                },
                error: None
            }
        ));
        session.tick(RETRY, &mut never);
        assert_eq!(calls.lock().unwrap().features, [(0, 37), (1, 900)]);
        session.cache(Draw::Controls(Controls {
            brightness: None,
            sleep_seconds: Some(900),
        }));
        session.device.as_mut().unwrap().0.read_error = true;
        session.tick(RETRY, &mut never);
        let (replacement, calls) = fake("A");
        let mut replacement = Some(replacement);
        session.tick(RETRY * 2, &mut |_| {
            Ok((replacement.take().unwrap(), "1".into()))
        });
        session.tick(RETRY * 2, &mut never);
        assert_eq!(calls.lock().unwrap().features, [(1, 900)]);
    }

    #[test]
    fn metadata_and_setter_failures_keep_images_alive_and_wait_for_fresh_commands() {
        let (mut deck, calls) = fake("A");
        deck.sleep_read_error = true;
        deck.brightness_error = true;
        deck.sleep_write_error = true;
        let mut session = Session::new(Some((deck, "1".into())), None);
        let controls = Controls {
            brightness: Some(37),
            sleep_seconds: Some(900),
        };
        session.cache(Draw::Controls(controls));
        session.cache(Draw::Key(0, frame(1)));
        let events = session.tick(Duration::ZERO, &mut never);
        assert!(matches!(
            &events[1],
            Event::Controls {
                controls: Controls {
                    brightness: None,
                    sleep_seconds: None
                },
                error: Some(_)
            }
        ));
        session.tick(Duration::ZERO, &mut never);
        assert!(session.tick(RETRY * 20, &mut never).is_empty());
        assert!(session.device.is_some());
        assert_eq!(calls.lock().unwrap().features, [(0, 37), (1, 900)]);
        assert_eq!(calls.lock().unwrap().writes, [(0, 1)]);
        let deck = &mut session.device.as_mut().unwrap().0;
        deck.sleep_read_error = false;
        deck.brightness_error = false;
        deck.sleep_write_error = false;
        session.cache(Draw::Controls(controls));
        session.tick(RETRY * 20, &mut never);
        let events = session.tick(RETRY * 20, &mut never);
        assert!(
            matches!(&events[..], [Event::Controls { controls: observed, error: None }] if *observed == controls)
        );
        assert_eq!(
            calls.lock().unwrap().features,
            [(0, 37), (1, 900), (0, 37), (1, 900)]
        );
    }

    #[test]
    fn invalid_control_bounds_never_reach_transport_and_stop_flushes_only_explicit_values() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Controls(Controls {
            brightness: Some(101),
            sleep_seconds: Some(u32::MAX),
        }));
        session.tick(Duration::ZERO, &mut never);
        session.tick(Duration::ZERO, &mut never);
        assert!(session.device.is_some());
        assert!(calls.lock().unwrap().features.is_empty());
        session.cache(Draw::Controls(Controls {
            brightness: Some(0),
            sleep_seconds: Some(i32::MAX as u32),
        }));
        assert_eq!(session.stop(Duration::ZERO).len(), 2);
        assert_eq!(
            calls.lock().unwrap().features,
            [(0, 0), (1, i32::MAX as u32)]
        );
        assert!(session.stop(Duration::ZERO).is_empty());
        let mut offline = Session::<Fake>::new(None, None);
        offline.cache(Draw::Controls(Controls {
            brightness: Some(0),
            sleep_seconds: Some(0),
        }));
        assert!(offline.stop(Duration::ZERO).is_empty());
    }

    /// Explicitly run only after the normal host has released this device.
    /// Proves owned HID handle close/reopen, not physical unplug or re-enumeration.
    #[test]
    #[ignore = "requires exclusive real-device ownership and DECKFORGE_REAL_DEVICE_RECONNECT_SERIAL"]
    fn real_device_controlled_close_reopen() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let Ok(serial) = std::env::var("DECKFORGE_REAL_DEVICE_RECONNECT_SERIAL") else {
            eprintln!(
                "REAL_HID_CLOSE_REOPEN_PROBE skipped: explicit serial environment variable absent"
            );
            return;
        };
        assert!(!serial.trim().is_empty(), "probe serial must not be empty");
        const CONTROLLED_ERROR: &str = "controlled close/reopen probe: first read failure";
        #[derive(Default)]
        struct Counts {
            opens: AtomicUsize,
            drops: AtomicUsize,
            writes: AtomicUsize,
            fail_first_read: AtomicBool,
        }
        struct Probe {
            deck: Deck,
            counts: Arc<Counts>,
        }
        impl Transport for Probe {
            fn sleep_seconds(&mut self) -> Result<u32> {
                self.deck.sleep_seconds()
            }
            fn set_brightness(&mut self, _: u8) -> Result<()> {
                self.counts.writes.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("probe forbids feature writes")
            }
            fn set_sleep_seconds(&mut self, _: u32) -> Result<()> {
                self.counts.writes.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("probe forbids feature writes")
            }
            fn serial(&self) -> &str {
                self.deck.serial()
            }
            fn read(&mut self, timeout_ms: i32) -> Result<Vec<InputEvent>> {
                ensure!(
                    !self.counts.fail_first_read.swap(false, Ordering::SeqCst),
                    CONTROLLED_ERROR
                );
                // Inputs go only to the test's event receiver. No host executes them.
                self.deck.read_events(timeout_ms)
            }
            fn write(&mut self, _: usize, _: &DynamicImage) -> Result<()> {
                self.counts.writes.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("probe forbids all image writes")
            }
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                self.counts.drops.fetch_add(1, Ordering::SeqCst);
            }
        }
        // Unwinding an assertion or timeout still stops and joins the owned worker.
        struct Cleanup {
            stop: std::sync::mpsc::Sender<Draw>,
            thread: Option<JoinHandle<()>>,
        }
        impl Cleanup {
            fn finish(&mut self) -> thread::Result<()> {
                let _ = self.stop.send(Draw::Stop);
                self.thread.take().map_or(Ok(()), JoinHandle::join)
            }
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.finish();
            }
        }

        eprintln!(
            "REAL_HID_CLOSE_REOPEN_PROBE beginning for serial={serial}; no image/reset/firmware writes; no input actions"
        );
        let initial = connect(Some(&serial)).expect("initial read-only HID connection");
        let firmware = initial.firmware.clone();
        let counts = Arc::new(Counts::default());
        counts.opens.store(1, Ordering::SeqCst);
        counts.fail_first_read.store(true, Ordering::SeqCst);
        let initial = (
            Probe {
                deck: initial.deck,
                counts: counts.clone(),
            },
            initial.firmware,
        );
        let (draw_tx, draw_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let selected = serial.clone();
        let child_counts = counts.clone();
        let handle = thread::spawn(move || {
            worker(
                Some(initial),
                Some(selected.clone()),
                draw_rx,
                event_tx,
                move |requested| {
                    ensure!(
                        requested == Some(selected.as_str()),
                        "probe reconnect requested a different serial"
                    );
                    let connection = connect(requested)?;
                    child_counts.opens.fetch_add(1, Ordering::SeqCst);
                    Ok((
                        Probe {
                            deck: connection.deck,
                            counts: child_counts.clone(),
                        },
                        connection.firmware,
                    ))
                },
            );
        });
        let mut cleanup = Cleanup {
            stop: draw_tx,
            thread: Some(handle),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut transitions = Vec::new();
        let mut input_count = 0;
        while Instant::now() < deadline {
            match event_rx.try_recv() {
                Ok(Event::Input(_)) => input_count += 1,
                Ok(Event::Controls { controls, error }) => eprintln!(
                    "REAL_HID_CLOSE_REOPEN_PROBE read-only metadata={controls:?}, error={error:?}"
                ),
                Ok(event) => {
                    eprintln!("REAL_HID_CLOSE_REOPEN_PROBE event={event:?}");
                    let complete =
                        matches!(&event, Event::Connected { .. }) && !transitions.is_empty();
                    transitions.push(event);
                    if complete {
                        break;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let joined = cleanup.finish();
        let opens = counts.opens.load(Ordering::SeqCst);
        let drops = counts.drops.load(Ordering::SeqCst);
        let writes = counts.writes.load(Ordering::SeqCst);
        eprintln!(
            "REAL_HID_CLOSE_REOPEN_PROBE opens={opens} drops={drops} forbidden_write_calls={writes} discarded_inputs={input_count}; proves handle close/reopen only, NOT physical unplug/re-enumeration"
        );
        assert!(joined.is_ok(), "probe worker panicked");
        assert!(
            matches!(&transitions[..], [
            Event::Connected { serial: first_serial, firmware: first_firmware },
            Event::Disconnected { reason },
            Event::Connected { serial: second_serial, firmware: second_firmware },
        ] if first_serial == &serial && second_serial == &serial
            && first_firmware == &firmware && second_firmware == &firmware
            && reason == CONTROLLED_ERROR),
            "expected Connected→controlled Disconnected→Connected within 10 seconds; got {transitions:?}"
        );
        assert_eq!((opens, drops, writes), (2, 2, 0));
    }
}
