//! Reconnecting device ownership and latest-frame delivery. No reset or firmware writes.
use crate::hardware::{Deck, InputEvent};
use anyhow::{Result, ensure};
use image::DynamicImage;
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
    Stop,
}

#[derive(Debug)]
pub enum Event {
    Connected { serial: String, firmware: String },
    Disconnected { reason: String },
    Retry { reason: String },
    Input(InputEvent),
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
}
impl Transport for Deck {
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
        }
    }

    // Invalid key indices cannot enter the nine-frame cache.
    fn cache(&mut self, draw: Draw) -> bool {
        let (index, image) = match draw {
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
        self.next_retry = now + RETRY;
        Event::Disconnected {
            reason: format!("{error:#}"),
        }
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
        loop {
            match self.write_one() {
                Ok(true) => {}
                Ok(false) => return Vec::new(),
                Err(error) => return vec![self.disconnect(now, error)],
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
    }
    struct Fake {
        serial: String,
        calls: Arc<Mutex<Calls>>,
        read_error: bool,
        write_error: bool,
        inputs: Vec<InputEvent>,
    }
    impl Transport for Fake {
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
            matches!(&events[..], [Event::Connected { serial, .. }, Event::Input(_)] if serial == "A")
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
                matches!(&session.tick(RETRY, &mut open)[..], [Event::Connected { firmware, .. }] if firmware == "2")
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
