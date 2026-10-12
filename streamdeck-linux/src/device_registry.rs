//! One fixed-model worker per serial, with exclusive HID-path admission.
//! Discovery reads metadata only. Each existing device worker owns its frames,
//! input state, reconnect pacing and ordered Stop behavior.
use crate::{
    device_model::Model,
    device_session::{self, Connection, Draw},
    hardware::Deck,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    ffi::CString,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Key {
    pub serial: String,
    pub model: Model,
}
#[derive(Clone, Debug)]
pub struct Handle {
    pub key: Key,
    pub generation: u64,
    pub draw: Sender<Draw>,
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub serial: Option<String>,
    pub model: Option<Model>,
}
#[derive(Debug)]
pub enum Event {
    Forgotten {
        key: Key,
    },
    Added {
        handle: Handle,
    },
    Tagged {
        key: Key,
        generation: u64,
        event: device_session::Event,
    },
    Diagnostic {
        error: String,
    },
}
pub enum Command {
    Forget(Key),
    Stop,
}
impl Filter {
    fn accepts(&self, key: &Key) -> bool {
        self.serial.as_ref().is_none_or(|s| s == &key.serial)
            && self.model.is_none_or(|m| m == key.model)
    }
    fn validate(&self) -> Result<()> {
        if let Some(serial) = &self.serial {
            validate_serial(serial)?;
        }
        Ok(())
    }
    fn candidate(&self, candidate: &Candidate) -> bool {
        if !candidate.model.capabilities().transport_supported
            || self.model.is_some_and(|model| model != candidate.model)
        {
            return false;
        }
        // A missing USB descriptor is not an identity mismatch. Read metadata
        // under an exclusive path lease, then verify the actual serial before
        // starting a worker or exposing input/image/control operations.
        self.serial.as_ref().is_none_or(|wanted| {
            candidate
                .serial
                .as_ref()
                .is_none_or(|found| found == wanted)
        })
    }
}
fn validate_serial(serial: &str) -> Result<()> {
    ensure!(
        !serial.is_empty() && serial.len() <= 256 && !serial.chars().any(char::is_control),
        "device serial must be nonempty, at most 256 bytes and contain no control characters"
    );
    Ok(())
}

pub const MAX_WORKERS: usize = 32;

pub struct Registry {
    inner: Core<System>,
}
impl Registry {
    /// No enumeration or USB access until refresh/watch is called.
    pub fn new(events: UnboundedSender<Event>, filter: Filter) -> Self {
        Self {
            inner: Core::new(Arc::new(System), events, filter),
        }
    }
    pub fn watch(&mut self, serial: String, model: Model) -> Result<Handle> {
        self.inner.watch(Key { serial, model })
    }
    /// Run outside the Host lock. Losing a path never removes its worker or frames.
    pub fn refresh(&mut self) -> Result<()> {
        self.inner.refresh()
    }
    /// Retire outside the Host lock. Acknowledge only after the worker has joined;
    /// any subsequently discovered device receives a fresh generation.
    pub fn forget(&mut self, key: Key) -> Result<()> {
        self.inner.forget(key)
    }
    /// Sends all Stop barriers before joining. Call outside the Host lock.
    pub fn shutdown(mut self) -> Result<()> {
        self.inner.stop_all()
    }
}

#[derive(Clone)]
struct Candidate {
    path: CString,
    serial: Option<String>,
    model: Model,
}
trait Backend: Send + Sync + 'static {
    type Connection: Send + 'static;
    fn enumerate(&self) -> Result<Vec<Candidate>>;
    fn open(&self, candidate: &Candidate) -> Result<Self::Connection>;
    fn identity(connection: &Self::Connection) -> (Key, Vec<u8>);
    fn spawn(
        &self,
        initial: Option<Self::Connection>,
        key: Key,
        draws: Receiver<Draw>,
        events: TaggedSink,
        open: Box<dyn FnMut() -> Result<Self::Connection> + Send>,
    ) -> JoinHandle<()>;
}
struct System;
impl Backend for System {
    type Connection = Connection;
    fn enumerate(&self) -> Result<Vec<Candidate>> {
        Ok(Deck::enumerate()?
            .into_iter()
            .map(|d| Candidate {
                path: d.raw_path,
                serial: d.serial,
                model: d.model,
            })
            .collect())
    }
    fn open(&self, candidate: &Candidate) -> Result<Connection> {
        let deck = Deck::connect_path(
            &candidate.path,
            candidate.serial.as_deref(),
            Some(candidate.model),
        )?;
        let firmware = deck.firmware_version()?;
        Ok(Connection { deck, firmware })
    }
    fn identity(connection: &Connection) -> (Key, Vec<u8>) {
        (
            Key {
                serial: connection.deck.serial().into(),
                model: connection.deck.model(),
            },
            connection.deck.path().to_bytes().to_vec(),
        )
    }
    fn spawn(
        &self,
        initial: Option<Connection>,
        key: Key,
        draws: Receiver<Draw>,
        events: TaggedSink,
        mut open: Box<dyn FnMut() -> Result<Connection> + Send>,
    ) -> JoinHandle<()> {
        device_session::spawn_with_connector(
            initial,
            Some(key.serial),
            Some(key.model),
            draws,
            events,
            move |_| open(),
        )
    }
}
struct TaggedSink {
    key: Key,
    generation: u64,
    events: UnboundedSender<Event>,
}
impl device_session::EventSink for TaggedSink {
    // Registry ownership, not observer lifetime, controls worker shutdown. This
    // keeps Stop/restoration available even if the UI drops its event receiver.
    fn is_closed(&self) -> bool {
        false
    }
    fn send(&self, event: device_session::Event) -> bool {
        let _ = self.events.send(Event::Tagged {
            key: self.key.clone(),
            generation: self.generation,
            event,
        });
        true
    }
}
#[derive(Default)]
struct Shared {
    leases: HashMap<Vec<u8>, u64>,
    next_lease: u64,
    known: HashMap<Vec<u8>, Key>,
    watched: HashSet<Key>,
}
struct Lease {
    shared: Arc<Mutex<Shared>>,
    path: Vec<u8>,
    token: u64,
}
impl Lease {
    fn claim(shared: &Arc<Mutex<Shared>>, path: &[u8]) -> Result<Option<Self>> {
        let mut state = shared.lock().unwrap();
        if state.leases.contains_key(path) {
            return Ok(None);
        }
        state.next_lease = state
            .next_lease
            .checked_add(1)
            .context("device lease counter exhausted")?;
        let token = state.next_lease;
        state.leases.insert(path.to_vec(), token);
        Ok(Some(Self {
            shared: shared.clone(),
            path: path.to_vec(),
            token,
        }))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.shared.lock().unwrap();
        if state.leases.get(&self.path) == Some(&self.token) {
            state.leases.remove(&self.path);
        }
    }
}
struct Connector<B: Backend> {
    backend: Arc<B>,
    shared: Arc<Mutex<Shared>>,
    filter: Filter,
    key: Key,
    held: Option<Lease>,
}
impl<B: Backend> Connector<B> {
    fn open(&mut self) -> Result<B::Connection> {
        // Session calls the connector only after dropping its previous device.
        // Retaining its lease until this point prevents discovery racing the close.
        self.held.take();
        let mut candidates = self.backend.enumerate()?;
        {
            let state = self.shared.lock().unwrap();
            candidates
                .sort_by_key(|c| u8::from(state.known.get(c.path.as_bytes()) != Some(&self.key)));
        }
        let mut error = None;
        for candidate in candidates {
            if candidate.model != self.key.model {
                continue;
            }
            if !self.filter.candidate(&candidate)
                || candidate
                    .serial
                    .as_ref()
                    .is_some_and(|serial| serial != &self.key.serial)
            {
                continue;
            }
            let Some(lease) = Lease::claim(&self.shared, candidate.path.as_bytes())? else {
                continue;
            };
            match self.backend.open(&candidate) {
                Ok(connection) => {
                    let (key, path) = B::identity(&connection);
                    self.shared
                        .lock()
                        .unwrap()
                        .known
                        .insert(path.clone(), key.clone());
                    if key == self.key && path == candidate.path.as_bytes() {
                        self.held = Some(lease);
                        return Ok(connection);
                    }
                    // Close first, release reservation second. No unmatched
                    // connection can reach Session's image/control/input loop.
                    drop(connection);
                    drop(lease);
                }
                Err(e) => {
                    error.get_or_insert(e);
                    drop(lease);
                }
            }
        }
        if let Some(error) = error {
            return Err(error.context(format!("reconnect {}", self.key.serial)));
        }
        bail!(
            "selected {} ({}) is unavailable or its path is already owned",
            self.key.serial,
            self.key.model.id()
        )
    }
}
struct Worker {
    handle: Handle,
    join: JoinHandle<()>,
}
struct Core<B: Backend> {
    backend: Arc<B>,
    events: UnboundedSender<Event>,
    filter: Filter,
    shared: Arc<Mutex<Shared>>,
    workers: HashMap<Key, Worker>,
    generation: u64,
    diagnostics: HashMap<Vec<u8>, String>,
}
impl<B: Backend> Core<B> {
    fn new(backend: Arc<B>, events: UnboundedSender<Event>, filter: Filter) -> Self {
        Self {
            backend,
            events,
            filter,
            shared: Arc::new(Mutex::new(Shared::default())),
            workers: HashMap::new(),
            generation: 0,
            diagnostics: HashMap::new(),
        }
    }
    fn validate_key(&self, key: &Key) -> Result<()> {
        self.filter.validate()?;
        validate_serial(&key.serial)?;
        ensure!(
            self.filter.accepts(key),
            "device is excluded by the explicit serial/model filter"
        );
        ensure!(
            key.model.capabilities().transport_supported,
            "device transport is unsupported"
        );
        Ok(())
    }
    fn watch(&mut self, key: Key) -> Result<Handle> {
        self.validate_key(&key)?;
        if let Some(worker) = self.workers.get(&key) {
            return Ok(worker.handle.clone());
        }
        self.start(key, None, None)
    }
    fn start(
        &mut self,
        key: Key,
        initial: Option<B::Connection>,
        lease: Option<Lease>,
    ) -> Result<Handle> {
        if self.workers.len() >= MAX_WORKERS {
            drop(initial);
            drop(lease);
            bail!(
                "device worker limit reached ({MAX_WORKERS}); existing device workers are retained"
            );
        }
        self.generation = self
            .generation
            .checked_add(1)
            .context("device generation counter exhausted")?;
        let (draw, draws) = mpsc::channel();
        let handle = Handle {
            key: key.clone(),
            generation: self.generation,
            draw,
        };
        if self
            .events
            .send(Event::Added {
                handle: handle.clone(),
            })
            .is_err()
        {
            drop(initial);
            drop(lease);
            bail!("device observer is closed");
        }
        self.shared.lock().unwrap().watched.insert(key.clone());
        let mut connector = Connector {
            backend: self.backend.clone(),
            shared: self.shared.clone(),
            filter: self.filter.clone(),
            key: key.clone(),
            held: lease,
        };
        let sink = TaggedSink {
            key: key.clone(),
            generation: self.generation,
            events: self.events.clone(),
        };
        let join = self.backend.spawn(
            initial,
            key.clone(),
            draws,
            sink,
            Box::new(move || connector.open()),
        );
        self.workers.insert(
            key,
            Worker {
                handle: handle.clone(),
                join,
            },
        );
        Ok(handle)
    }
    fn diagnostic(&mut self, path: Vec<u8>, error: String) {
        if self.diagnostics.get(&path) != Some(&error) {
            self.diagnostics.insert(path, error.clone());
            let _ = self.events.send(Event::Diagnostic { error });
        }
    }
    fn refresh(&mut self) -> Result<()> {
        self.filter.validate()?;
        let candidates = match self.backend.enumerate() {
            Ok(c) => {
                self.diagnostics.remove(&Vec::new());
                c
            }
            Err(e) => {
                self.diagnostic(Vec::new(), format!("device enumeration failed: {e:#}"));
                return Err(e);
            }
        };
        let present: HashSet<Vec<u8>> = candidates
            .iter()
            .map(|c| c.path.as_bytes().to_vec())
            .collect();
        self.shared
            .lock()
            .unwrap()
            .known
            .retain(|path, _| present.contains(path));
        for candidate in candidates {
            let path = candidate.path.as_bytes().to_vec();
            {
                let state = self.shared.lock().unwrap();
                if !self.filter.candidate(&candidate) {
                    continue;
                }
                let key = candidate
                    .serial
                    .as_ref()
                    .map(|serial| Key {
                        serial: serial.clone(),
                        model: candidate.model,
                    })
                    .or_else(|| {
                        state
                            .known
                            .get(&path)
                            .filter(|key| key.model == candidate.model)
                            .cloned()
                    });
                if key.as_ref().is_some_and(|key| state.watched.contains(key)) {
                    continue;
                }
            }
            if self.workers.len() >= MAX_WORKERS {
                self.diagnostic(
                    path,
                    format!(
                        "device worker limit reached ({MAX_WORKERS}); excess device was not opened"
                    ),
                );
                continue;
            }
            let Some(lease) = Lease::claim(&self.shared, &path)? else {
                continue;
            };
            match self.backend.open(&candidate) {
                Ok(connection) => {
                    let (key, actual_path) = B::identity(&connection);
                    self.shared
                        .lock()
                        .unwrap()
                        .known
                        .insert(actual_path.clone(), key.clone());
                    if !self.filter.accepts(&key) {
                        drop(connection);
                        drop(lease);
                        continue;
                    }
                    let validation = self.validate_key(&key).and_then(|()| {
                        ensure!(
                            actual_path == path && key.model == candidate.model,
                            "opened device identity changed"
                        );
                        Ok(())
                    });
                    if let Err(error) = validation {
                        drop(connection);
                        drop(lease);
                        self.diagnostic(path, format!("device admission failed: {error:#}"));
                        continue;
                    }
                    self.diagnostics.remove(&path);
                    if self.workers.contains_key(&key) {
                        drop(connection);
                        drop(lease);
                        continue;
                    }
                    self.start(key, Some(connection), Some(lease))?;
                }
                Err(error) => {
                    drop(lease);
                    self.diagnostic(path, format!("device discovery failed: {error:#}"));
                }
            }
        }
        Ok(())
    }
    fn remove(&mut self, key: &Key) -> Result<()> {
        if let Some(worker) = self.workers.remove(key) {
            let _ = worker.handle.draw.send(Draw::Stop);
            let result = worker.join.join();
            let mut shared = self.shared.lock().unwrap();
            shared.watched.remove(key);
            shared.known.retain(|_, remembered| remembered != key);
            ensure!(result.is_ok(), "device worker panicked during shutdown");
        }
        Ok(())
    }
    fn forget(&mut self, key: Key) -> Result<()> {
        let result = self.remove(&key);
        let _ = self.events.send(Event::Forgotten { key });
        result
    }
    fn stop_all(&mut self) -> Result<()> {
        for worker in self.workers.values() {
            let _ = worker.handle.draw.send(Draw::Stop);
        }
        let mut panicked = false;
        for (key, worker) in self.workers.drain() {
            panicked |= worker.join.join().is_err();
            self.shared.lock().unwrap().watched.remove(&key);
        }
        ensure!(!panicked, "device worker panicked during shutdown");
        Ok(())
    }
}
impl<B: Backend> Drop for Core<B> {
    fn drop(&mut self) {
        let _ = self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{device_session::EventSink, hardware::InputEvent};
    use std::{
        sync::Condvar,
        time::{Duration, Instant},
    };
    #[derive(Clone)]
    struct Device {
        candidate: Candidate,
        key: Key,
    }
    #[derive(Default)]
    struct State {
        devices: Vec<Device>,
        active: HashSet<Vec<u8>>,
        opens: Vec<Vec<u8>>,
        closed: Vec<Vec<u8>>,
        frames: Vec<(String, u8)>,
        stops: HashSet<Key>,
        expected_stops: usize,
        enumeration_error: bool,
    }
    #[derive(Default)]
    struct Fake {
        state: Mutex<State>,
        stopped: Condvar,
    }
    struct FakeConnection {
        key: Key,
        path: Vec<u8>,
        backend: Arc<Fake>,
    }
    impl Drop for FakeConnection {
        fn drop(&mut self) {
            let mut s = self.backend.state.lock().unwrap();
            assert!(s.active.remove(&self.path));
            s.closed.push(self.path.clone());
        }
    }
    // Backend owns only fixture data. Its small channel worker models lifecycle
    // boundaries; the actual Session reconnect/frame behavior has separate tests.
    struct FixtureBackend(Arc<Fake>);
    impl Backend for FixtureBackend {
        type Connection = FakeConnection;
        fn enumerate(&self) -> Result<Vec<Candidate>> {
            let state = self.0.state.lock().unwrap();
            ensure!(!state.enumeration_error, "controlled inventory failure");
            Ok(state.devices.iter().map(|d| d.candidate.clone()).collect())
        }
        fn open(&self, candidate: &Candidate) -> Result<FakeConnection> {
            let mut state = self.0.state.lock().unwrap();
            let device = state
                .devices
                .iter()
                .find(|d| d.candidate.path == candidate.path)
                .context("fixture unplugged")?
                .clone();
            let path = candidate.path.as_bytes().to_vec();
            ensure!(
                state.active.insert(path.clone()),
                "fixture device opened twice concurrently"
            );
            state.opens.push(path.clone());
            Ok(FakeConnection {
                key: device.key,
                path,
                backend: self.0.clone(),
            })
        }
        fn identity(c: &FakeConnection) -> (Key, Vec<u8>) {
            (c.key.clone(), c.path.clone())
        }
        fn spawn(
            &self,
            initial: Option<FakeConnection>,
            key: Key,
            draws: Receiver<Draw>,
            events: TaggedSink,
            mut open: Box<dyn FnMut() -> Result<FakeConnection> + Send>,
        ) -> JoinHandle<()> {
            let state = self.0.clone();
            std::thread::spawn(move || {
                let connection = match initial {
                    Some(c) => Some(c),
                    None => match open() {
                        Ok(c) => Some(c),
                        Err(e) => {
                            events.send(device_session::Event::Retry {
                                reason: e.to_string(),
                            });
                            None
                        }
                    },
                };
                if let Some(c) = &connection {
                    events.send(device_session::Event::Connected {
                        serial: c.key.serial.clone(),
                        model: c.key.model,
                        firmware: "fixture".into(),
                    });
                    events.send(device_session::Event::Input(InputEvent::Key {
                        index: 0,
                        pressed: true,
                    }));
                }
                while let Ok(draw) = draws.recv() {
                    match draw {
                        Draw::Stop => {
                            let mut s = state.state.lock().unwrap();
                            s.stops.insert(key.clone());
                            state.stopped.notify_all();
                            let deadline = Instant::now() + Duration::from_secs(1);
                            while s.stops.len() < s.expected_stops {
                                let left = deadline.saturating_duration_since(Instant::now());
                                assert!(
                                    !left.is_zero(),
                                    "Stop was not sent to all workers before joining"
                                );
                                s = state.stopped.wait_timeout(s, left).unwrap().0;
                            }
                            break;
                        }
                        Draw::Key(index, _) => state
                            .state
                            .lock()
                            .unwrap()
                            .frames
                            .push((key.serial.clone(), index)),
                        _ => {}
                    }
                }
                drop(connection); // Exact ownership boundary required before connector lease drops.
                drop(open);
            })
        }
    }
    fn key(serial: &str) -> Key {
        Key {
            serial: serial.into(),
            model: Model::Plus,
        }
    }
    fn device(path: &str, serial: &str, descriptor: bool) -> Device {
        Device {
            candidate: Candidate {
                path: CString::new(path).unwrap(),
                serial: descriptor.then(|| serial.into()),
                model: Model::Plus,
            },
            key: key(serial),
        }
    }
    fn setup(
        devices: Vec<Device>,
        filter: Filter,
    ) -> (
        Core<FixtureBackend>,
        Arc<Fake>,
        tokio::sync::mpsc::UnboundedReceiver<Event>,
    ) {
        let fake = Arc::new(Fake::default());
        fake.state.lock().unwrap().devices = devices;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Core::new(Arc::new(FixtureBackend(fake.clone())), tx, filter),
            fake,
            rx,
        )
    }
    fn wait(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !ready() {
            assert!(Instant::now() < deadline, "fixture timeout");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn worker_limit_preserves_known_workers_and_does_not_open_excess_devices() {
        let devices = (0..=MAX_WORKERS)
            .map(|i| device(&format!("path-{i}"), &format!("serial-{i}"), false))
            .collect();
        let (mut core, fake, mut rx) = setup(devices, Filter::default());
        core.refresh().unwrap();
        assert_eq!(core.workers.len(), MAX_WORKERS);
        assert_eq!(fake.state.lock().unwrap().opens.len(), MAX_WORKERS);
        let known = core.workers[&key("serial-0")].handle.clone();
        assert_eq!(
            core.watch(known.key.clone()).unwrap().generation,
            known.generation
        );
        assert!(
            core.watch(key("new-excess"))
                .unwrap_err()
                .to_string()
                .contains("32")
        );
        core.refresh().unwrap();
        assert_eq!(fake.state.lock().unwrap().opens.len(), MAX_WORKERS);
        let mut added = 0;
        let mut diagnostics = 0;
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Added { .. } => added += 1,
                Event::Diagnostic { error } => {
                    assert!(error.contains("limit"));
                    diagnostics += 1;
                }
                _ => {}
            }
        }
        assert_eq!(added, MAX_WORKERS);
        assert_eq!(diagnostics, 1);
        core.remove(&known.key).unwrap();
        // Remaining workers retain their exact identity/generation. A freed slot
        // can admit a newly observed device; no live device is displaced.
        fake.state
            .lock()
            .unwrap()
            .devices
            .retain(|d| d.key != known.key);
        core.refresh().unwrap();
        assert_eq!(core.workers.len(), MAX_WORKERS);
        assert!(
            core.workers
                .contains_key(&key(&format!("serial-{MAX_WORKERS}")))
        );
        assert_eq!(fake.state.lock().unwrap().opens.len(), MAX_WORKERS + 1);
        core.stop_all().unwrap();
        assert!(fake.state.lock().unwrap().active.is_empty());
    }
    #[test]
    fn same_model_descriptorless_discovery_is_exclusive_idempotent_and_tagged() {
        let (mut core, fake, mut rx) = setup(
            vec![device("A", "one", false), device("B", "two", false)],
            Filter::default(),
        );
        core.refresh().unwrap();
        core.refresh().unwrap();
        assert_eq!(core.workers.len(), 2);
        assert_eq!(fake.state.lock().unwrap().opens.len(), 2);
        let mut added = HashMap::new();
        let mut inputs = HashSet::new();
        wait(|| {
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::Added { handle } => {
                        added.insert(handle.key, handle.generation);
                    }
                    Event::Tagged {
                        key,
                        generation,
                        event,
                    } => {
                        assert_eq!(added.get(&key), Some(&generation));
                        if matches!(event, device_session::Event::Input(_)) {
                            inputs.insert(key);
                        }
                    }
                    Event::Diagnostic { error } => panic!("{error}"),
                    Event::Forgotten { .. } => panic!("Unexpected device retirement"),
                }
            }
            inputs.len() == 2
        });
        for worker in core.workers.values() {
            worker
                .handle
                .draw
                .send(Draw::Key(7, image::DynamicImage::new_rgb8(1, 1)))
                .unwrap();
        }
        wait(|| fake.state.lock().unwrap().frames.len() == 2);
        assert!(
            fake.state
                .lock()
                .unwrap()
                .frames
                .contains(&("one".into(), 7))
        );
        assert!(
            fake.state
                .lock()
                .unwrap()
                .frames
                .contains(&("two".into(), 7))
        );
        fake.state.lock().unwrap().expected_stops = 2;
        // Closing the observer does not bypass explicit worker cleanup.
        drop(rx);
        core.stop_all().unwrap();
        assert!(fake.state.lock().unwrap().active.is_empty());
        assert!(core.shared.lock().unwrap().leases.is_empty());
    }
    #[test]
    fn explicit_absent_serial_only_probes_unknown_metadata_and_closes_mismatch() {
        let mut other_model = device("C", "absent", false);
        other_model.candidate.model = Model::Mini;
        other_model.key.model = Model::Mini;
        let (mut core, fake, mut rx) = setup(
            vec![
                device("A", "real", false),
                device("B", "other", true),
                other_model,
            ],
            Filter {
                serial: Some("absent".into()),
                model: Some(Model::Plus),
            },
        );
        core.refresh().unwrap();
        let state = fake.state.lock().unwrap();
        assert_eq!(state.opens, vec![b"A".to_vec()]);
        assert_eq!(state.closed, vec![b"A".to_vec()]);
        assert!(state.active.is_empty());
        assert!(state.frames.is_empty());
        drop(state);
        assert!(core.workers.is_empty());
        assert!(core.shared.lock().unwrap().leases.is_empty());
        // No worker means no input/frame/control path was exposed, even briefly.
        assert!(rx.try_recv().is_err());
        assert!(core.watch(key("real")).is_err());
        core.stop_all().unwrap();
    }
    #[test]
    fn reconnect_skips_other_live_lease_and_wrong_serial_then_finds_same_identity() {
        let fake = Arc::new(Fake::default());
        fake.state.lock().unwrap().devices = vec![
            device("B", "two", false),
            device("wrong", "other", false),
            device("A", "one", false),
        ];
        let backend = Arc::new(FixtureBackend(fake.clone()));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let blease = Lease::claim(&shared, b"B").unwrap().unwrap();
        let b = backend.open(&device("B", "two", false).candidate).unwrap();
        shared
            .lock()
            .unwrap()
            .watched
            .extend([key("one"), key("two")]);
        let mut connector = Connector {
            backend: backend.clone(),
            shared: shared.clone(),
            filter: Filter::default(),
            key: key("one"),
            held: None,
        };
        let a = connector.open().unwrap();
        assert_eq!(a.key, key("one"));
        assert_eq!(
            fake.state
                .lock()
                .unwrap()
                .opens
                .iter()
                .filter(|p| p.as_slice() == b"B")
                .count(),
            1
        );
        assert!(
            fake.state
                .lock()
                .unwrap()
                .closed
                .contains(&b"wrong".to_vec())
        );
        assert!(
            !shared
                .lock()
                .unwrap()
                .leases
                .contains_key(b"wrong".as_slice())
        );
        drop(a);
        fake.state.lock().unwrap().devices =
            vec![device("B", "two", false), device("A-new", "one", false)];
        let reconnected = connector.open().unwrap();
        assert_eq!(reconnected.path, b"A-new");
        assert_eq!(
            fake.state
                .lock()
                .unwrap()
                .opens
                .iter()
                .filter(|p| p.as_slice() == b"B")
                .count(),
            1
        );
        assert!(!shared.lock().unwrap().leases.contains_key(b"A".as_slice()));
        drop(reconnected);
        drop(connector);
        drop(b);
        drop(blease);
        assert!(fake.state.lock().unwrap().active.is_empty());
        assert!(shared.lock().unwrap().leases.is_empty());
    }
    #[test]
    fn inventory_failure_keeps_workers_and_readmission_uses_new_generation() {
        let (mut core, fake, mut rx) = setup(vec![device("A", "one", true)], Filter::default());
        core.refresh().unwrap();
        let first = core.workers[&key("one")].handle.clone();
        fake.state.lock().unwrap().enumeration_error = true;
        assert!(core.refresh().is_err());
        assert!(core.refresh().is_err());
        assert_eq!(
            core.workers[&key("one")].handle.generation,
            first.generation
        );
        core.remove(&first.key).unwrap();
        assert!(first.draw.send(Draw::SleepPolicy(false)).is_err());
        fake.state.lock().unwrap().enumeration_error = false;
        core.refresh().unwrap();
        let second = core.workers[&key("one")].handle.clone();
        assert!(second.generation > first.generation);
        let mut generations = HashSet::new();
        let mut diagnostics = 0;
        wait(|| {
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::Tagged {
                        generation,
                        event: device_session::Event::Connected { .. },
                        ..
                    } => {
                        generations.insert(generation);
                    }
                    Event::Diagnostic { .. } => diagnostics += 1,
                    _ => {}
                }
            }
            generations.len() == 2
        });
        assert_eq!(diagnostics, 1);
        assert!(generations.contains(&first.generation));
        assert!(generations.contains(&second.generation));
        core.stop_all().unwrap();
    }
    #[test]
    fn forget_acknowledges_join_before_rediscovery_and_keeps_other_workers() {
        let (mut core, fake, mut rx) = setup(
            vec![device("A", "one", true), device("B", "two", true)],
            Filter::default(),
        );
        core.refresh().unwrap();
        let first = core.workers[&key("one")].handle.clone();
        let second = core.workers[&key("two")].handle.clone();
        core.forget(first.key.clone()).unwrap();
        assert!(first.draw.send(Draw::SleepPolicy(false)).is_err());
        assert_eq!(
            core.workers[&key("two")].handle.generation,
            second.generation
        );
        assert!(!core.shared.lock().unwrap().watched.contains(&first.key));
        assert!(!fake.state.lock().unwrap().active.contains(b"A".as_slice()));
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let retired = events
            .iter()
            .position(|e| matches!(e, Event::Forgotten { key: k } if k == &first.key))
            .unwrap();
        assert!(
            !events[retired + 1..]
                .iter()
                .any(|e| matches!(e, Event::Tagged { key: k, .. } if k == &first.key))
        );
        core.refresh().unwrap();
        let next = core.workers[&key("one")].handle.clone();
        assert!(next.generation > first.generation);
        assert_eq!(
            core.workers[&key("two")].handle.generation,
            second.generation
        );
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok()).any(
                |e| matches!(e, Event::Added { handle } if handle.generation == next.generation)
            )
        );
    }
    #[test]
    fn explicit_serial_identifies_first_use_and_reconnects_after_new_registry_restart() {
        let filter = Filter {
            serial: Some("one".into()),
            model: Some(Model::Plus),
        };
        let (mut core, fake, mut rx) = setup(
            vec![
                device("wrong", "other", false),
                device("A", "one", false),
                device("known-other", "other", true),
            ],
            filter.clone(),
        );
        core.refresh().unwrap();
        assert_eq!(core.workers.len(), 1);
        assert!(core.workers.contains_key(&key("one")));
        wait(|| {
            while let Ok(event) = rx.try_recv() {
                if let Event::Tagged {
                    key,
                    event: device_session::Event::Connected { serial, .. },
                    ..
                } = event
                {
                    assert_eq!(key.serial, "one");
                    assert_eq!(serial, "one");
                    return true;
                }
            }
            false
        });
        core.stop_all().unwrap();
        drop(core);
        assert!(fake.state.lock().unwrap().active.is_empty());
        // New registry has no identity cache. The saved serial can reconnect on
        // a new path by metadata identification, before any input is delivered.
        fake.state.lock().unwrap().devices = vec![
            device("A-new", "one", false),
            device("known-other", "other", true),
        ];
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut restarted = Core::new(Arc::new(FixtureBackend(fake.clone())), tx, filter);
        assert!(restarted.shared.lock().unwrap().known.is_empty());
        restarted.watch(key("one")).unwrap();
        wait(|| {
            while let Ok(event) = rx.try_recv() {
                if let Event::Tagged {
                    key,
                    event: device_session::Event::Connected { serial, .. },
                    ..
                } = event
                {
                    assert_eq!(key.serial, "one");
                    assert_eq!(serial, "one");
                    return true;
                }
            }
            false
        });
        assert!(
            fake.state
                .lock()
                .unwrap()
                .opens
                .contains(&b"A-new".to_vec())
        );
        assert!(
            !fake
                .state
                .lock()
                .unwrap()
                .opens
                .contains(&b"known-other".to_vec())
        );
        restarted.stop_all().unwrap();
        assert!(fake.state.lock().unwrap().active.is_empty());
    }
    #[test]
    fn changed_model_metadata_is_closed_without_worker_admission() {
        let (mut core, fake, _rx) = setup(vec![device("changed", "one", true)], Filter::default());
        fake.state.lock().unwrap().devices[0].key.model = Model::Mini;
        core.refresh().unwrap();
        assert!(core.workers.is_empty());
        assert!(fake.state.lock().unwrap().active.is_empty());
        assert!(core.shared.lock().unwrap().leases.is_empty());
    }
}
