//! Reconnecting device ownership and latest-frame delivery. No reset or firmware writes.
use crate::{
    device_model::Model,
    hardware::{Deck, InputEvent},
};
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
    Infobar(DynamicImage),
    Controls(Controls),
    /// Keep the firmware idle timer suspended while software sleep is configured.
    SleepPolicy(bool),
    Screensaver {
        generation: u64,
        image: Option<DynamicImage>,
        wake_on_input: bool,
    },
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
        model: Model,
    },
    Disconnected {
        reason: String,
    },
    Retry {
        reason: String,
    },
    Input(InputEvent),
    Wake {
        generation: u64,
    },
    /// A physical report or a successful normal image/control update. Screensaver
    /// animation frames and read-only metadata do not count as activity.
    Activity,
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

#[cfg(test)]
pub fn connect(serial: Option<&str>) -> Result<Connection> {
    connect_model(serial, None)
}

pub fn connect_model(serial: Option<&str>, model: Option<Model>) -> Result<Connection> {
    let deck = Deck::connect_with_model(serial, model)?;
    let firmware = deck.firmware_version()?;
    Ok(Connection { deck, firmware })
}

pub fn spawn_model(
    initial: Option<Connection>,
    serial: Option<String>,
    expected_model: Option<Model>,
    draw_rx: Receiver<Draw>,
    events: UnboundedSender<Event>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        worker_model(
            initial.map(|c| (c.deck, c.firmware)),
            serial,
            expected_model,
            draw_rx,
            events,
            |serial| connect_model(serial, expected_model).map(|c| (c.deck, c.firmware)),
        );
    })
}

trait Transport {
    fn model(&self) -> Model {
        Model::Plus
    }
    fn serial(&self) -> &str;
    fn read_packet(&mut self, timeout_ms: i32, decode: bool) -> Result<Option<Vec<InputEvent>>>;
    fn write(&mut self, index: usize, image: &DynamicImage) -> Result<()>;
    fn write_lcd(&mut self, image: &DynamicImage) -> Result<()>;
    fn sleep_seconds(&mut self) -> Result<u32>;
    fn set_brightness(&mut self, brightness: u8) -> Result<()>;
    fn set_sleep_seconds(&mut self, seconds: u32) -> Result<()>;
}
impl Transport for Deck {
    fn model(&self) -> Model {
        Deck::model(self)
    }
    fn sleep_seconds(&mut self) -> Result<u32> {
        Deck::sleep_seconds(self)
    }
    fn set_brightness(&mut self, brightness: u8) -> Result<()> {
        let value = if self.model() == Model::Plus {
            crate::brightness::hid_value(brightness)?
        } else {
            brightness
        };
        Deck::set_brightness(self, value)
    }
    fn set_sleep_seconds(&mut self, seconds: u32) -> Result<()> {
        Deck::set_sleep_seconds(self, seconds)
    }
    fn serial(&self) -> &str {
        self.serial()
    }
    fn read_packet(&mut self, timeout_ms: i32, decode: bool) -> Result<Option<Vec<InputEvent>>> {
        Deck::read_packet(self, timeout_ms, decode)
    }
    fn write_lcd(&mut self, image: &DynamicImage) -> Result<()> {
        if self.model().capabilities().full_lcd {
            self.set_lcd_image(image)
        } else {
            // Mini has no full-panel command. Fill each key from a software grid.
            let model = self.model();
            ensure!(model.capabilities().key_images, "device has no display");
            let frame = image.resize_exact(
                u32::from(model.columns()) * model.key_width(),
                u32::from(model.rows()) * model.key_height(),
                image::imageops::FilterType::Triangle,
            );
            for index in 0..model.key_count() {
                let tile = frame.crop_imm(
                    u32::from(index % model.columns()) * model.key_width(),
                    u32::from(index / model.columns()) * model.key_height(),
                    model.key_width(),
                    model.key_height(),
                );
                self.set_key_image(index, &tile)?;
            }
            Ok(())
        }
    }
    fn write(&mut self, index: usize, image: &DynamicImage) -> Result<()> {
        if index == usize::from(self.model().key_count()) {
            if self.model() == Model::Neo {
                self.set_infobar_image(image)
            } else {
                self.set_touchscreen_image(image)
            }
        } else {
            self.set_key_image(index as u8, image)
        }
    }
}

const RETRY: Duration = Duration::from_secs(1);
struct Saver {
    generation: u64,
    image: Arc<DynamicImage>,
    wake_on_input: bool,
}
enum Written {
    None,
    Normal,
    Saver,
}
#[derive(Default)]
struct Timer {
    policy: bool,
    stopping: bool,
    baseline: Option<u32>,
    owned: bool,
    // A failed disable and a failed restore are distinct operations even when
    // their value is zero. Animation frames never reset this attempt marker.
    attempted: Option<(u32, bool)>,
}
struct Session<T> {
    device: Option<(T, String)>,
    serial: Option<String>,
    announce: bool,
    model: Model,
    desired: Vec<Option<Arc<DynamicImage>>>,
    sent: Vec<Option<Arc<DynamicImage>>>,
    cursor: usize,
    next_retry: Duration,
    retry_reason: Option<String>,
    desired_controls: Controls,
    attempted_brightness: Option<u8>,
    observed_controls: Controls,
    control_errors: [Option<String>; 2],
    control_cursor: usize,
    saver: Option<Saver>,
    saver_sent: Option<Arc<DynamicImage>>,
    closed_generation: Option<u64>,
    draining: Option<u64>,
    wake_notification: bool,
    timer: Timer,
}
impl<T: Transport> Session<T> {
    #[cfg(test)]
    fn new(initial: Option<(T, String)>, serial: Option<String>) -> Self {
        Self::new_model(initial, serial, None)
    }
    fn new_model(
        initial: Option<(T, String)>,
        serial: Option<String>,
        expected: Option<Model>,
    ) -> Self {
        let model = initial
            .as_ref()
            .map(|(deck, _)| deck.model())
            .or(expected)
            .unwrap_or_default();
        let frames = usize::from(model.key_count()) + usize::from(model.capabilities().strip);
        Self {
            model,
            device: initial,
            serial,
            announce: true,
            desired: vec![None; frames],
            sent: vec![None; frames],
            cursor: 0,
            next_retry: Duration::ZERO,
            retry_reason: None,
            desired_controls: Controls::default(),
            attempted_brightness: None,
            observed_controls: Controls::default(),
            control_errors: [None, None],
            control_cursor: 0,
            saver: None,
            saver_sent: None,
            closed_generation: None,
            draining: None,
            wake_notification: false,
            timer: Timer::default(),
        }
    }

    // Reject controls that the selected model cannot display.
    fn cache(&mut self, draw: Draw) -> bool {
        let (index, image) = match draw {
            Draw::SleepPolicy(enabled) => {
                self.timer.policy = enabled;
                self.retry_timer();
                return false;
            }
            Draw::Screensaver {
                generation,
                image,
                wake_on_input,
            } => {
                if !self.model.has_lcd() {
                    return false;
                }
                if let Some(image) = image {
                    if self
                        .closed_generation
                        .is_some_and(|closed| generation <= closed)
                        || self
                            .saver
                            .as_ref()
                            .is_some_and(|active| generation < active.generation)
                    {
                        return false;
                    }
                    self.saver = Some(Saver {
                        generation,
                        image: Arc::new(image),
                        wake_on_input,
                    });
                } else {
                    if let Some(active) = &self.saver
                        && active.generation <= generation
                    {
                        self.draining = Some(active.generation);
                        self.wake_notification = false;
                    }
                    self.close_saver(generation);
                }
                return false;
            }
            Draw::Controls(controls) => {
                // A new command can retry a failed field. Successful duplicates
                // remain coalesced; None withdraws an override without resetting it.
                if controls.brightness == self.attempted_brightness
                    && controls.brightness != self.observed_controls.brightness
                {
                    self.attempted_brightness = None;
                }
                self.desired_controls = controls;
                self.retry_timer();
                return false;
            }
            Draw::Stop => return true,
            Draw::Key(index, image)
                if index < self.model.key_count() && self.model.capabilities().key_images =>
            {
                (usize::from(index), image)
            }
            Draw::Strip(image) if self.model.dial_count() > 0 => {
                (usize::from(self.model.key_count()), image)
            }
            Draw::Infobar(image) if self.model == Model::Neo => {
                (usize::from(self.model.key_count()), image)
            }
            Draw::Key(_, _) | Draw::Strip(_) | Draw::Infobar(_) => return false,
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
                    device.model() == self.model,
                    "connected device model does not match selected layout"
                );
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
                model: self.model,
            });
            self.announce = false;
            self.retry_reason = None;
            self.observed_controls = Controls::default();
            self.attempted_brightness = None;
            self.timer.attempted = None;
            self.control_errors = [None, None];
            if self.model.capabilities().sleep {
                match self.device.as_mut().unwrap().0.sleep_seconds() {
                    Ok(seconds) if seconds <= i32::MAX as u32 => {
                        self.observed_controls.sleep_seconds = Some(seconds);
                        if !self.timer.owned {
                            self.timer.baseline = Some(seconds);
                        }
                    }
                    Ok(_) => {
                        self.control_errors[1] =
                            Some("device sleep duration exceeds signed 32-bit seconds".into())
                    }
                    Err(error) => self.control_errors[1] = Some(format!("{error:#}")),
                }
            }
            events.push(self.controls_event());
        }
        // Poll before each image, rather than holding input behind a full replay.
        let timeout = if self.display_dirty() { 0 } else { 20 };
        let mut activity = match self.poll_input(timeout, &mut events) {
            Ok(activity) => activity,
            Err(error) => {
                events.push(self.disconnect(now, error));
                return events;
            }
        };
        // At most one feature update per poll, with independent image progress.
        // Metadata failures do not evict a functioning input/image transport.
        if let Some((event, success)) = self.write_control() {
            events.push(event);
            activity |= success;
        }
        match self.write_one() {
            Ok(Written::Normal) => activity = true,
            Ok(_) => {}
            Err(error) => events.push(self.disconnect(now, error)),
        }
        if activity {
            events.push(Event::Activity);
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
        self.saver_sent = None;
        self.attempted_brightness = None;
        self.timer.attempted = None;
        if !self.timer.owned {
            self.timer.baseline = None;
        }
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
    fn timer_target(&self) -> Option<(u32, bool)> {
        let restore = self.desired_controls.sleep_seconds.or(self.timer.baseline);
        let suspend = !self.timer.stopping
            && (self.timer.policy || self.saver.is_some() || self.draining.is_some())
            && restore.is_some_and(|seconds| seconds <= i32::MAX as u32);
        if suspend {
            Some((0, true))
        } else if self.timer.owned {
            restore.map(|seconds| (seconds, false))
        } else {
            self.desired_controls
                .sleep_seconds
                .map(|seconds| (seconds, false))
        }
    }
    fn retry_timer(&mut self) {
        if self.timer_target() == self.timer.attempted
            && self
                .timer_target()
                .is_some_and(|(seconds, _)| self.observed_controls.sleep_seconds != Some(seconds))
        {
            self.timer.attempted = None;
        }
    }
    fn write_control(&mut self) -> Option<(Event, bool)> {
        let timer_target = self.timer_target();
        let (device, _) = self.device.as_mut()?;
        for offset in 0..2 {
            let field = (self.control_cursor + offset) % 2;
            if field == 0 {
                let Some(brightness) = self.desired_controls.brightness else {
                    continue;
                };
                if self.attempted_brightness == Some(brightness) {
                    continue;
                }
                self.attempted_brightness = Some(brightness);
                let result = (|| {
                    ensure!(
                        self.model.capabilities().brightness,
                        "device has no brightness control"
                    );
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
                let Some((seconds, suspend)) = timer_target else {
                    continue;
                };
                if suspend && !self.timer.owned {
                    // The last getter/verified setter is the pre-override value.
                    // If that is unknown, only an explicit user value supplies a
                    // safe restore target; remember it even if later withdrawn.
                    self.timer.baseline = self
                        .observed_controls
                        .sleep_seconds
                        .or(self.timer.baseline)
                        .or(self.desired_controls.sleep_seconds);
                    self.timer.owned = true;
                }
                if self.observed_controls.sleep_seconds == Some(seconds) {
                    self.timer.attempted = Some((seconds, suspend));
                    if !suspend {
                        self.timer.owned = false;
                        self.timer.baseline = Some(seconds);
                    }
                    continue;
                }
                if self.timer.attempted == Some((seconds, suspend)) {
                    continue;
                }
                self.timer.attempted = Some((seconds, suspend));
                let result = (|| {
                    ensure!(
                        seconds <= i32::MAX as u32,
                        "sleep duration exceeds signed 32-bit seconds"
                    );
                    ensure!(
                        self.model.capabilities().sleep,
                        "device has no sleep control"
                    );
                    device.set_sleep_seconds(seconds)
                })();
                match result {
                    Ok(()) => {
                        self.observed_controls.sleep_seconds = Some(seconds);
                        self.control_errors[1] = None;
                        if !suspend {
                            self.timer.owned = false;
                            self.timer.baseline = Some(seconds);
                        }
                    }
                    Err(error) => {
                        self.observed_controls.sleep_seconds = None;
                        self.control_errors[1] = Some(format!("{error:#}"));
                    }
                }
            }
            self.control_cursor = (field + 1) % 2;
            return Some((self.controls_event(), self.control_errors[field].is_none()));
        }
        None
    }
    fn dirty(&self) -> Option<usize> {
        (0..self.desired.len())
            .map(|offset| (self.cursor + offset) % self.desired.len())
            .find(|&index| {
                self.desired[index].as_ref().is_some_and(|wanted| {
                    self.sent[index]
                        .as_ref()
                        .is_none_or(|sent| !Arc::ptr_eq(sent, wanted) && !same(sent, wanted))
                })
            })
    }
    fn close_saver(&mut self, generation: u64) {
        self.closed_generation = Some(
            self.closed_generation
                .map_or(generation, |old| old.max(generation)),
        );
        if self
            .saver
            .as_ref()
            .is_some_and(|active| active.generation <= generation)
        {
            self.saver = None;
            self.saver_sent = None;
            self.sent.fill(None);
        }
    }
    fn display_dirty(&self) -> bool {
        if self.draining.is_some() {
            return false;
        }
        if let Some(saver) = &self.saver {
            return self
                .saver_sent
                .as_ref()
                .is_none_or(|sent| !same(sent, &saver.image));
        }
        self.dirty().is_some()
    }
    fn poll_input(&mut self, timeout: i32, events: &mut Vec<Event>) -> Result<bool> {
        let (device, _) = self.device.as_mut().unwrap();
        if self.saver.is_none() && self.draining.is_none() {
            return match device.read_packet(timeout, true)? {
                Some(input) => {
                    events.extend(input.into_iter().map(Event::Input));
                    Ok(true)
                }
                None => Ok(false),
            };
        }
        let mut activity = false;
        // Consume the wake report and queued reports without touching decoder
        // state. Resume across ticks if a burst exceeds this bounded batch.
        for index in 0..256 {
            let wait = if index == 0 && self.draining.is_none() {
                timeout
            } else {
                0
            };
            match device.read_packet(wait, false)? {
                Some(_) => {
                    activity = true;
                    if self.draining.is_none()
                        && let Some(saver) = &self.saver
                        && saver.wake_on_input
                    {
                        self.draining = Some(saver.generation);
                        self.wake_notification = true;
                        self.closed_generation = Some(
                            self.closed_generation
                                .map_or(saver.generation, |old| old.max(saver.generation)),
                        );
                    }
                }
                None => {
                    if let Some(generation) = self.draining.take() {
                        self.close_saver(generation);
                        if self.wake_notification {
                            events.push(Event::Wake { generation });
                        }
                        self.wake_notification = false;
                    }
                    break;
                }
            }
        }
        Ok(activity)
    }
    fn write_one(&mut self) -> Result<Written> {
        if self.draining.is_some() {
            return Ok(Written::None);
        }
        if let Some(saver) = &self.saver {
            if self
                .saver_sent
                .as_ref()
                .is_some_and(|sent| same(sent, &saver.image))
            {
                return Ok(Written::None);
            }
            let Some((device, _)) = self.device.as_mut() else {
                return Ok(Written::None);
            };
            device.write_lcd(&saver.image)?;
            self.saver_sent = Some(saver.image.clone());
            return Ok(Written::Saver);
        }
        let Some(index) = self.dirty() else {
            return Ok(Written::None);
        };
        let Some((device, _)) = self.device.as_mut() else {
            return Ok(Written::None);
        };
        let image = self.desired[index].as_ref().unwrap();
        device.write(index, image)?;
        self.sent[index] = Some(image.clone());
        self.cursor = (index + 1) % self.desired.len();
        Ok(Written::Normal)
    }
    fn stop(&mut self, now: Duration) -> Vec<Event> {
        let mut events = Vec::new();
        self.timer.stopping = true;
        // Shutdown makes one final bounded restoration attempt, even when an
        // earlier normal wake restoration failed. Never reconnect just to do it.
        if self.timer.owned {
            self.timer.attempted = None;
        }
        if let Some(generation) = self.draining.take() {
            self.close_saver(generation);
            self.wake_notification = false;
        }
        while let Some((event, _)) = self.write_control() {
            events.push(event);
        }
        loop {
            match self.write_one() {
                Ok(Written::Normal | Written::Saver) => {}
                Ok(Written::None) => return events,
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

#[cfg(test)]
fn worker<T: Transport>(
    initial: Option<(T, String)>,
    serial: Option<String>,
    draw_rx: Receiver<Draw>,
    events: UnboundedSender<Event>,
    open: impl FnMut(Option<&str>) -> Result<(T, String)>,
) {
    worker_model(initial, serial, None, draw_rx, events, open)
}

fn worker_model<T: Transport>(
    initial: Option<(T, String)>,
    serial: Option<String>,
    expected_model: Option<Model>,
    draw_rx: Receiver<Draw>,
    events: UnboundedSender<Event>,
    mut open: impl FnMut(Option<&str>) -> Result<(T, String)>,
) {
    let start = Instant::now();
    let mut session = Session::new_model(initial, serial, expected_model);
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
        decoding: Vec<bool>,
    }
    struct Fake {
        model: Model,
        serial: String,
        calls: Arc<Mutex<Calls>>,
        read_error: bool,
        write_error: bool,
        inputs: Vec<InputEvent>,
        packets: VecDeque<Vec<InputEvent>>,
        sleep: u32,
        sleep_read_error: bool,
        brightness_error: bool,
        sleep_write_error: bool,
        on_sleep_set: Option<Box<dyn FnMut(u32) + Send>>,
    }
    impl Transport for Fake {
        fn model(&self) -> Model {
            self.model
        }
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
            if let Some(callback) = &mut self.on_sleep_set {
                callback(seconds);
            }
            ensure!(!self.sleep_write_error, "sleep readback mismatch");
            self.sleep = seconds;
            self.sleep_seconds()?;
            Ok(())
        }
        fn serial(&self) -> &str {
            &self.serial
        }
        fn read_packet(&mut self, _: i32, decode: bool) -> Result<Option<Vec<InputEvent>>> {
            {
                let mut calls = self.calls.lock().unwrap();
                calls.reads += 1;
                calls.decoding.push(decode);
            }
            ensure!(!self.read_error, "read failed");
            let packet = if !self.inputs.is_empty() {
                Some(std::mem::take(&mut self.inputs))
            } else {
                self.packets.pop_front()
            };
            Ok(packet.map(|events| if decode { events } else { Vec::new() }))
        }
        fn write_lcd(&mut self, image: &DynamicImage) -> Result<()> {
            self.write(9, image)
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
                model: Model::Plus,
                serial: serial.into(),
                calls: calls.clone(),
                read_error: false,
                write_error: false,
                inputs: Vec::new(),
                packets: VecDeque::new(),
                sleep: 600,
                sleep_read_error: false,
                brightness_error: false,
                sleep_write_error: false,
                on_sleep_set: None,
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
    fn model_caches_cover_all_keys_and_only_supported_extra_displays() {
        for &model in Model::all() {
            let (mut deck, calls) = fake("A");
            deck.model = model;
            let mut session = Session::new_model(Some((deck, "1".into())), None, Some(model));
            for index in 0..=model.key_count() {
                session.cache(Draw::Key(index, frame(index)));
            }
            session.cache(Draw::Strip(frame(100)));
            session.cache(Draw::Infobar(frame(101)));
            for _ in 0..40 {
                session.tick(Duration::ZERO, &mut never);
            }
            let writes = &calls.lock().unwrap().writes;
            let expected = if model.capabilities().key_images {
                usize::from(model.key_count())
            } else {
                0
            } + usize::from(model.capabilities().strip);
            assert_eq!(writes.len(), expected, "{model:?}");
            for index in 0..if model.capabilities().key_images {
                model.key_count()
            } else {
                0
            } {
                assert!(
                    writes.contains(&(usize::from(index), index)),
                    "{model:?} missing key {index}"
                );
            }
            if model.capabilities().strip {
                assert_eq!(
                    writes.last(),
                    Some(&(
                        usize::from(model.key_count()),
                        if model == Model::Neo { 101 } else { 100 }
                    ))
                );
            }
        }
    }

    #[test]
    fn pedal_never_reads_or_writes_display_features() {
        let (mut deck, calls) = fake("A");
        deck.model = Model::Pedal;
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Screensaver {
            generation: 1,
            image: Some(frame(1)),
            wake_on_input: true,
        });
        session.cache(Draw::Controls(Controls {
            brightness: Some(20),
            sleep_seconds: Some(60),
        }));
        for _ in 0..3 {
            session.tick(Duration::ZERO, &mut never);
        }
        let calls = calls.lock().unwrap();
        assert_eq!(calls.sleep_queries, 0);
        assert!(calls.features.is_empty());
        assert!(calls.writes.is_empty());
        assert!(calls.reads > 0);
    }

    #[test]
    fn wrong_model_reconnection_cannot_replay_cached_frames() {
        let mut session = Session::<Fake>::new_model(None, Some("A".into()), Some(Model::Xl));
        session.cache(Draw::Key(31, frame(1)));
        let (deck, calls) = fake("A");
        let mut device = Some(deck);
        let events = session.tick(Duration::ZERO, &mut |_| {
            Ok((device.take().unwrap(), "1".into()))
        });
        assert!(matches!(&events[..], [Event::Retry { reason }] if reason.contains("model")));
        assert!(calls.lock().unwrap().writes.is_empty());
        assert_eq!(session.desired[31].as_ref().unwrap().as_bytes()[0], 1);
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
            matches!(&events[..], [Event::Connected { serial, .. }, Event::Controls { error: None, .. }, Event::Input(_), Event::Activity] if serial == "A")
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
                matches!(&session.tick(RETRY, &mut open)[..], [Event::Connected { firmware, .. }, Event::Controls { .. }, Event::Activity] if firmware == "2")
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
            matches!(&events[..], [Event::Controls { controls: observed, error: None }, Event::Activity] if *observed == controls)
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

    fn saver(generation: u64, value: u8, wake_on_input: bool) -> Draw {
        Draw::Screensaver {
            generation,
            image: Some(frame(value)),
            wake_on_input,
        }
    }
    fn clear_saver(generation: u64) -> Draw {
        Draw::Screensaver {
            generation,
            image: None,
            wake_on_input: true,
        }
    }

    #[test]
    fn screensaver_coalesces_full_frames_hides_normal_updates_and_replays_all_nine_on_clear() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        for key in 0..8 {
            session.cache(Draw::Key(key, frame(key)));
        }
        session.cache(Draw::Strip(frame(8)));
        for _ in 0..9 {
            session.tick(Duration::ZERO, &mut never);
        }
        calls.lock().unwrap().writes.clear();
        session.cache(saver(1, 200, true));
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [Event::Controls { .. }, Event::Activity]
        ));
        session.cache(Draw::Key(0, frame(99)));
        session.cache(Draw::Strip(frame(77)));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        session.cache(saver(1, 201, true));
        session.cache(saver(1, 202, true));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        session.cache(saver(1, 202, true));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert_eq!(calls.lock().unwrap().writes, [(9, 200), (9, 202)]);
        session.cache(clear_saver(1));
        for index in 0..9 {
            let events = session.tick(Duration::ZERO, &mut never);
            assert_eq!(events.len(), if index == 0 { 2 } else { 1 });
            assert!(matches!(events.last(), Some(Event::Activity)));
        }
        let writes = &calls.lock().unwrap().writes;
        assert_eq!(writes.len(), 11);
        assert!(writes[2..].contains(&(0, 99)) && writes[2..].contains(&(8, 77)));
        for key in 1..8 {
            assert!(writes[2..].contains(&(key, key as u8)));
        }
    }

    #[test]
    fn wake_consumes_first_packet_and_bounded_queued_reports_before_decoding_resumes() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Key(0, frame(4)));
        session.tick(Duration::ZERO, &mut never);
        session.cache(saver(7, 200, true));
        session.tick(Duration::ZERO, &mut never);
        session.device.as_mut().unwrap().0.packets = (0..300)
            .map(|index| {
                vec![InputEvent::Key {
                    index: 0,
                    pressed: index % 2 == 0,
                }]
            })
            .collect();
        let events = session.tick(Duration::ZERO, &mut never);
        assert!(matches!(&events[..], [Event::Activity]));
        assert_eq!(session.draining, Some(7));
        assert_eq!(calls.lock().unwrap().writes, [(0, 4), (9, 200)]);
        session.cache(saver(7, 201, true)); // Queued animation must not undo waking.
        let events = session.tick(Duration::ZERO, &mut never);
        assert!(matches!(
            &events[..],
            [
                Event::Wake { generation: 7 },
                Event::Controls { .. },
                Event::Activity
            ]
        ));
        assert!(session.saver.is_none() && session.draining.is_none());
        assert_eq!(calls.lock().unwrap().writes, [(0, 4), (9, 200), (0, 4)]);
        assert!(
            calls.lock().unwrap().decoding[1..]
                .iter()
                .all(|decode| !decode)
        );
        session
            .device
            .as_mut()
            .unwrap()
            .0
            .packets
            .push_back(vec![InputEvent::Key {
                index: 0,
                pressed: true,
            }]);
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [
                Event::Input(InputEvent::Key { pressed: true, .. }),
                Event::Activity
            ]
        ));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
    }

    #[test]
    fn empty_decoded_packet_wakes_but_locked_policy_consumes_without_waking() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(saver(10, 200, false));
        session.tick(Duration::ZERO, &mut never);
        session
            .device
            .as_mut()
            .unwrap()
            .0
            .packets
            .push_back(Vec::new());
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [Event::Activity]
        ));
        assert_eq!(session.saver.as_ref().unwrap().generation, 10);
        session.cache(saver(10, 201, true));
        session
            .device
            .as_mut()
            .unwrap()
            .0
            .packets
            .push_back(Vec::new());
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [
                Event::Wake { generation: 10 },
                Event::Controls { .. },
                Event::Activity
            ]
        ));
        assert!(session.saver.is_none());
        session.cache(saver(10, 202, true));
        session.cache(saver(9, 203, true));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert_eq!(calls.lock().unwrap().writes, [(9, 200)]);
    }

    #[test]
    fn explicit_wake_discards_sleeping_backlog_without_an_input_wake_notification() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Key(0, frame(4)));
        session.cache(saver(11, 200, false));
        session.tick(Duration::ZERO, &mut never);
        session.device.as_mut().unwrap().0.packets = (0..300)
            .map(|_| {
                vec![InputEvent::Key {
                    index: 0,
                    pressed: true,
                }]
            })
            .collect();
        session.cache(clear_saver(11));
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [Event::Activity]
        ));
        assert!(session.draining.is_some());
        assert_eq!(calls.lock().unwrap().writes, [(9, 200)]);
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [Event::Controls { .. }, Event::Activity]
        ));
        assert!(session.draining.is_none());
        assert_eq!(calls.lock().unwrap().writes, [(9, 200), (0, 4)]);
    }

    #[test]
    fn old_clear_cannot_cancel_new_generation_and_reconnect_replays_active_layer_only() {
        let (deck, _) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Key(0, frame(4)));
        session.cache(saver(1, 200, true));
        session.tick(Duration::ZERO, &mut never);
        session.cache(saver(2, 201, true));
        session.cache(clear_saver(1));
        session.cache(saver(1, 202, true));
        session.tick(Duration::ZERO, &mut never);
        assert_eq!(session.saver.as_ref().unwrap().generation, 2);
        session.device.as_mut().unwrap().0.read_error = true;
        assert!(matches!(
            &session.tick(Duration::ZERO, &mut never)[..],
            [Event::Disconnected { .. }]
        ));
        let (replacement, calls) = fake("A");
        let mut replacement = Some(replacement);
        let events = session.tick(RETRY, &mut |_| {
            Ok((replacement.take().unwrap(), "1".into()))
        });
        assert!(matches!(
            &events[..],
            [
                Event::Connected { .. },
                Event::Controls { .. },
                Event::Controls { .. },
                Event::Activity
            ]
        ));
        assert_eq!(calls.lock().unwrap().writes, [(9, 201)]);
        session.cache(clear_saver(2));
        session.tick(RETRY, &mut never);
        assert_eq!(calls.lock().unwrap().writes, [(9, 201), (0, 4)]);
    }

    #[test]
    fn fullscreen_write_failure_disconnects_and_stop_flushes_only_the_selected_layer() {
        let (mut deck, _) = fake("A");
        deck.write_error = true;
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Key(0, frame(4)));
        session.cache(saver(4, 200, true));
        let events = session.tick(Duration::ZERO, &mut never);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Disconnected { .. }))
        );
        let (replacement, calls) = fake("A");
        let mut replacement = Some(replacement);
        session.tick(RETRY, &mut |_| {
            Ok((replacement.take().unwrap(), "1".into()))
        });
        session.cache(saver(4, 201, true));
        assert!(matches!(&session.stop(RETRY)[..], [Event::Controls { .. }]));
        assert_eq!(calls.lock().unwrap().writes, [(9, 200), (9, 201)]);
        session.cache(clear_saver(4));
        session.stop(RETRY);
        assert_eq!(calls.lock().unwrap().writes, [(9, 200), (9, 201), (0, 4)]);
    }

    #[test]
    fn timer_baseline_survives_rapid_sleep_wake_cycles_without_host_acknowledgements() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.tick(Duration::ZERO, &mut never);
        session.cache(saver(1, 200, true));
        session.tick(Duration::ZERO, &mut never); // Its Controls(0) event is deliberately ignored.
        session.cache(clear_saver(1));
        session.cache(saver(2, 201, true));
        session.tick(Duration::ZERO, &mut never);
        assert_eq!(session.timer.baseline, Some(600));
        assert_eq!(calls.lock().unwrap().features, [(1, 0)]);
        session.cache(clear_saver(2));
        session.tick(Duration::ZERO, &mut never);
        assert!(!session.timer.owned);
        assert_eq!(session.observed_controls.sleep_seconds, Some(600));
        session.cache(saver(3, 202, true));
        session.tick(Duration::ZERO, &mut never);
        session.cache(clear_saver(3));
        session.stop(Duration::ZERO); // Restore even before the ordinary clear tick.
        assert_eq!(
            calls.lock().unwrap().features,
            [(1, 0), (1, 600), (1, 0), (1, 600)]
        );
    }

    #[test]
    fn wake_queued_inside_zero_setter_then_stop_restores_in_worker_order() {
        let (mut deck, calls) = fake("A");
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Draw::SleepPolicy(true)).unwrap();
        tx.send(Draw::Key(0, frame(4))).unwrap();
        deck.on_sleep_set = Some(Box::new(move |seconds| {
            if seconds == 0 {
                tx.send(Draw::SleepPolicy(false)).unwrap();
                tx.send(Draw::Stop).unwrap();
            }
        }));
        let (events, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        worker(Some((deck, "1".into())), None, rx, events, never);
        assert_eq!(calls.lock().unwrap().features, [(1, 0), (1, 600)]);
        let observations: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok())
            .filter_map(|event| {
                if let Event::Controls {
                    controls,
                    error: None,
                } = event
                {
                    controls.sleep_seconds
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(observations, [600, 0, 600]);
    }

    #[test]
    fn configured_software_policy_restores_explicit_timer_on_stop_without_uncovering_locked_layer()
    {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::Controls(Controls {
            brightness: None,
            sleep_seconds: Some(900),
        }));
        session.cache(Draw::SleepPolicy(true));
        session.cache(Draw::Key(0, frame(4)));
        session.tick(Duration::ZERO, &mut never);
        assert_eq!(calls.lock().unwrap().features, [(1, 0)]);
        assert!(session.timer.owned);
        session.cache(saver(1, 200, false));
        session.tick(Duration::ZERO, &mut never);
        session.cache(saver(1, 201, false));
        session.stop(Duration::ZERO);
        assert!(!session.timer.owned);
        assert!(session.saver.is_some());
        assert_eq!(calls.lock().unwrap().features, [(1, 0), (1, 900)]);
        assert_eq!(calls.lock().unwrap().writes, [(0, 4), (9, 200), (9, 201)]);
    }

    #[test]
    fn already_zero_timer_deduplicates_disable_and_restore_without_waiting_for_an_event() {
        let (mut deck, calls) = fake("A");
        deck.sleep = 0;
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::SleepPolicy(true));
        session.tick(Duration::ZERO, &mut never);
        assert!(session.timer.owned);
        session.cache(Draw::SleepPolicy(false));
        assert!(session.tick(Duration::ZERO, &mut never).is_empty());
        assert!(!session.timer.owned);
        assert!(calls.lock().unwrap().features.is_empty());
    }

    #[test]
    fn stop_restores_baseline_even_while_software_policy_remains_enabled_and_awake() {
        let (deck, calls) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::SleepPolicy(true));
        session.tick(Duration::ZERO, &mut never);
        assert!(session.saver.is_none());
        assert!(session.timer.policy && session.timer.owned);
        session.stop(Duration::ZERO);
        assert!(!session.timer.owned);
        assert_eq!(calls.lock().unwrap().features, [(1, 0), (1, 600)]);
    }

    #[test]
    fn unknown_baseline_defers_zero_and_animation_does_not_retry_failed_feature_writes() {
        let (mut deck, calls) = fake("A");
        deck.sleep_read_error = true;
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::SleepPolicy(true));
        session.cache(saver(1, 200, true));
        session.tick(Duration::ZERO, &mut never);
        assert!(calls.lock().unwrap().features.is_empty());
        assert!(!session.timer.owned);
        session.cache(Draw::Controls(Controls {
            brightness: None,
            sleep_seconds: Some(600),
        }));
        session.tick(Duration::ZERO, &mut never); // Explicit target permits zero; readback still fails.
        assert!(session.timer.owned);
        assert_eq!(session.timer.baseline, Some(600));
        for value in 201..210 {
            session.cache(saver(1, value, true));
            session.tick(Duration::ZERO, &mut never);
        }
        assert_eq!(calls.lock().unwrap().features, [(1, 0)]);
        session.cache(clear_saver(1));
        session.cache(Draw::SleepPolicy(false));
        session.tick(Duration::ZERO, &mut never);
        assert!(session.timer.owned); // Failed restore does not relinquish ownership.
        assert_eq!(calls.lock().unwrap().features, [(1, 0), (1, 600)]);
        session.device.as_mut().unwrap().0.sleep_read_error = false;
        session.cache(Draw::Controls(Controls::default())); // Removing override cannot lose its fallback.
        session.tick(Duration::ZERO, &mut never);
        assert!(!session.timer.owned);
        assert_eq!(calls.lock().unwrap().features, [(1, 0), (1, 600), (1, 600)]);
    }

    #[test]
    fn reconnect_preserves_owned_baseline_but_refreshes_it_after_restoration() {
        let (deck, _) = fake("A");
        let mut session = Session::new(Some((deck, "1".into())), None);
        session.cache(Draw::SleepPolicy(true));
        session.tick(Duration::ZERO, &mut never);
        session.device.as_mut().unwrap().0.read_error = true;
        session.tick(Duration::ZERO, &mut never);
        let (mut replacement, calls) = fake("A");
        replacement.sleep = 0;
        let mut replacement = Some(replacement);
        session.tick(RETRY, &mut |_| {
            Ok((replacement.take().unwrap(), "1".into()))
        });
        assert_eq!(session.timer.baseline, Some(600));
        assert!(calls.lock().unwrap().features.is_empty());
        session.cache(Draw::SleepPolicy(false));
        session.tick(RETRY, &mut never);
        assert_eq!(calls.lock().unwrap().features, [(1, 600)]);
        assert!(!session.timer.owned);
        session.device.as_mut().unwrap().0.read_error = true;
        session.tick(RETRY, &mut never);
        let (mut replacement, calls) = fake("A");
        replacement.sleep = 900;
        let mut replacement = Some(replacement);
        session.tick(RETRY * 2, &mut |_| {
            Ok((replacement.take().unwrap(), "1".into()))
        });
        assert_eq!(session.timer.baseline, Some(900));
        assert!(calls.lock().unwrap().features.is_empty());
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
            fn read_packet(
                &mut self,
                timeout_ms: i32,
                decode: bool,
            ) -> Result<Option<Vec<InputEvent>>> {
                ensure!(
                    !self.counts.fail_first_read.swap(false, Ordering::SeqCst),
                    CONTROLLED_ERROR
                );
                // Inputs go only to the test's event receiver. No host executes them.
                self.deck.read_packet(timeout_ms, decode)
            }
            fn write_lcd(&mut self, _: &DynamicImage) -> Result<()> {
                self.counts.writes.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("probe forbids all LCD writes")
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
                Ok(Event::Activity) => {}
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
            Event::Connected { serial: first_serial, firmware: first_firmware, .. },
            Event::Disconnected { reason },
            Event::Connected { serial: second_serial, firmware: second_firmware, .. },
        ] if first_serial == &serial && second_serial == &serial
            && first_firmware == &firmware && second_firmware == &firmware
            && reason == CONTROLLED_ERROR),
            "expected Connected→controlled Disconnected→Connected within 10 seconds; got {transitions:?}"
        );
        assert_eq!((opens, drops, writes), (2, 2, 0));
    }
}
