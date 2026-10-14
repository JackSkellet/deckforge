//! Nonblocking PulseAudio transport, also supported by PipeWire's Pulse server.
//! The mixer and device connection live off the host/editor thread.
use crate::soundboard::{Config, Mixer};
use anyhow::{Context as _, Result, bail, ensure};
use libpulse_binding as pulse;
use pulse::{
    callbacks::ListResult,
    context::{Context, State as ContextState},
    def::BufferAttr,
    mainloop::standard::{IterateResult, Mainloop},
    sample::{Format, Spec},
    stream::{SeekMode, State as StreamState, Stream},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const PLAY: &str = "com.elgato.streamdeck.soundboard.playaudio";
pub const STOP: &str = "com.elgato.streamdeck.soundboard.stopaudioplay";
pub fn supported(uuid: &str) -> bool {
    matches!(uuid, PLAY | STOP)
}
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            PLAY,
            "Play audio",
            json!({"path":"","actionType":1,"volume":50,"fadeType":0,"fadeLen":1,"outputType":""}),
        ),
        (STOP, "Stop audio", json!({})),
    ]
}
pub fn defaults(uuid: &str) -> Option<Value> {
    definitions()
        .into_iter()
        .find(|(id, _, _)| *id == uuid)
        .map(|(_, _, settings)| settings)
}

pub enum Report {
    Active(HashMap<String, usize>),
    Outputs(Vec<(String, String)>),
    Status(String),
}
enum Command {
    StopContext(String),
    Play(String, Config),
    Refresh,
}
struct Job {
    generation: u64,
    command: Command,
}
pub struct Worker {
    sender: mpsc::SyncSender<Job>,
    reports: mpsc::Receiver<Report>,
    stop: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(32);
        let (reporter, reports) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let epoch = generation.clone();
        let thread = thread::spawn(move || {
            let mut mixer = Mixer::new();
            let mut connection: Option<Connection> = None;
            let mut current = 0;
            let mut previous = HashMap::new();
            while !done.load(Ordering::Acquire) {
                let next = epoch.load(Ordering::Acquire);
                if next != current {
                    current = next;
                    mixer.stop_all();
                    if let Some(connection) = &mut connection {
                        connection.clear();
                    }
                    let _ = reporter.send(Report::Status("Soundboard stopped".into()));
                }
                // Bound work per cycle so a burst of presses cannot starve playback.
                for _ in 0..8 {
                    let Ok(job) = receiver.try_recv() else {
                        break;
                    };
                    let observed = epoch.load(Ordering::Acquire);
                    if observed != current {
                        current = observed;
                        mixer.stop_all();
                        if let Some(connection) = &mut connection {
                            connection.clear();
                        }
                    }
                    if job.generation != current {
                        continue;
                    }
                    let result = (|| -> Result<()> {
                        if connection.is_none() {
                            connection = Some(Connection::new(&done)?);
                        }
                        let connection = connection.as_mut().unwrap();
                        match job.command {
                            Command::StopContext(context) => mixer.stop_context(&context),
                            Command::Refresh => {
                                let outputs = connection.outputs(&done, &mut mixer, &reporter)?;
                                let _ = reporter.send(Report::Outputs(outputs));
                            }
                            Command::Play(context, config) => {
                                connection.prepare(&config.output, &done, &mut mixer, &reporter)?;
                                // A Stop-all arriving during connection cannot be overtaken.
                                if epoch.load(Ordering::Acquire) != current
                                    || done.load(Ordering::Acquire)
                                {
                                    return Ok(());
                                }
                                mixer.trigger(&context, config)?;
                                let _ = reporter
                                    .send(Report::Status("Soundboard playback updated".into()));
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        let _ =
                            reporter.send(Report::Status(format!("Soundboard failed: {error:#}")));
                    }
                }
                if let Some(active) = &mut connection
                    && let Err(error) = active.pump(&mut mixer, &reporter)
                {
                    mixer.stop_all();
                    connection = None;
                    let _ = reporter.send(Report::Status(format!(
                        "Audio connection lost: {error:#}. Press again to reconnect."
                    )));
                }
                let active = mixer.active();
                if active != previous {
                    previous = active.clone();
                    let _ = reporter.send(Report::Active(active));
                }
                thread::sleep(Duration::from_millis(5));
            }
            mixer.stop_all();
            if let Some(connection) = &mut connection {
                connection.clear();
            }
        });
        Self {
            sender,
            reports,
            stop,
            generation,
            thread: Some(thread),
        }
    }
    fn send(&self, command: Command) -> Result<()> {
        ensure!(!self.stop.load(Ordering::Acquire), "Audio worker stopped");
        self.sender
            .try_send(Job {
                generation: self.generation.load(Ordering::Acquire),
                command,
            })
            .map_err(|_| anyhow::anyhow!("Soundboard queue is full; press Stop audio to clear it"))
    }
    pub fn play(&self, context: &str, config: Config) -> Result<()> {
        self.send(Command::Play(context.into(), config))
    }
    pub fn stop_context(&self, context: &str) -> Result<()> {
        self.send(Command::StopContext(context.into()))
    }
    pub fn refresh(&self) -> Result<()> {
        self.send(Command::Refresh)
    }
    pub fn stop_all(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    pub fn drain(&self) -> Vec<Report> {
        self.reports.try_iter().collect()
    }
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Output {
    stream: Stream,
    drain: Option<pulse::operation::Operation<dyn FnMut(bool)>>,
    touched: Instant,
}
struct Connection {
    streams: HashMap<String, Output>,
    context: Context,
    mainloop: Mainloop,
}
impl Connection {
    fn new(stop: &AtomicBool) -> Result<Self> {
        let mainloop = Mainloop::new().context("Create audio event loop")?;
        let mut context =
            Context::new(&mainloop, "Deckforge soundboard").context("Create audio context")?;
        context.connect(None, pulse::context::FlagSet::NOAUTOSPAWN, None)?;
        let mut this = Self {
            streams: HashMap::new(),
            context,
            mainloop,
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            this.iterate()?;
            match this.context.get_state() {
                ContextState::Ready => return Ok(this),
                ContextState::Failed | ContextState::Terminated => {
                    bail!("Cannot connect to PulseAudio/PipeWire")
                }
                _ => {}
            }
            ensure!(
                !stop.load(Ordering::Acquire) && Instant::now() < deadline,
                "Audio connection cancelled or timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn iterate(&mut self) -> Result<()> {
        for _ in 0..16 {
            match self.mainloop.iterate(false) {
                IterateResult::Success(0) => break,
                IterateResult::Success(_) => {}
                _ => bail!("Audio event loop stopped"),
            }
        }
        ensure!(
            !matches!(
                self.context.get_state(),
                ContextState::Failed | ContextState::Terminated
            ),
            "Audio server disconnected"
        );
        Ok(())
    }
    fn outputs(
        &mut self,
        stop: &AtomicBool,
        mixer: &mut Mixer,
        reports: &mpsc::Sender<Report>,
    ) -> Result<Vec<(String, String)>> {
        let values = Rc::new(RefCell::new(Vec::new()));
        let items = values.clone();
        let error = Rc::new(RefCell::new(false));
        let failed = error.clone();
        let mut operation =
            self.context
                .introspect()
                .get_sink_info_list(move |result| match result {
                    ListResult::Item(info) => {
                        if let Some(name) = &info.name {
                            items.borrow_mut().push((
                                name.to_string(),
                                info.description.as_deref().unwrap_or(name).to_string(),
                            ));
                        }
                    }
                    ListResult::Error => *failed.borrow_mut() = true,
                    ListResult::End => {}
                });
        let deadline = Instant::now() + Duration::from_secs(3);
        while operation.get_state() == pulse::operation::State::Running {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                operation.cancel();
                bail!("Output-device query cancelled or timed out");
            }
            self.pump(mixer, reports)?;
            thread::sleep(Duration::from_millis(2));
        }
        ensure!(!*error.borrow(), "Could not list audio outputs");
        let mut result = values.borrow().clone();
        result.sort();
        Ok(result)
    }
    fn prepare(
        &mut self,
        output: &str,
        stop: &AtomicBool,
        mixer: &mut Mixer,
        reports: &mpsc::Sender<Report>,
    ) -> Result<()> {
        ensure!(!output.contains('\0'), "Invalid output name");
        if let Some(existing) = self.streams.get_mut(output) {
            if let Some(mut drain) = existing.drain.take() {
                drain.cancel();
            }
            existing.touched = Instant::now();
            return Ok(());
        }
        ensure!(self.streams.len() < 16, "Too many audio outputs in use");
        let spec = Spec {
            format: Format::FLOAT32NE,
            channels: 2,
            rate: 48000,
        };
        let mut stream = Stream::new(&mut self.context, "Soundboard", &spec, None)
            .context("Create output stream")?;
        // 40ms server queue, 10ms mixer blocks. No system/device volume mutation.
        let attr = BufferAttr {
            maxlength: 38400,
            tlength: 15360,
            prebuf: 0,
            minreq: 3840,
            fragsize: u32::MAX,
        };
        stream.connect_playback(
            if output.is_empty() {
                None
            } else {
                Some(output)
            },
            Some(&attr),
            if output.is_empty() {
                pulse::stream::FlagSet::ADJUST_LATENCY
            } else {
                pulse::stream::FlagSet::ADJUST_LATENCY | pulse::stream::FlagSet::DONT_MOVE
            },
            None,
            None,
        )?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            self.pump(mixer, reports)?;
            match stream.get_state() {
                StreamState::Ready => break,
                StreamState::Failed | StreamState::Terminated => {
                    bail!("Selected audio output is unavailable")
                }
                _ => {}
            }
            ensure!(
                !stop.load(Ordering::Acquire) && Instant::now() < deadline,
                "Output connection cancelled or timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
        // PipeWire/Pulse policy may accept an unknown requested sink and move the
        // stream elsewhere. Never submit samples until the explicit route matches.
        if !output.is_empty() && stream.get_device_name().as_deref() != Some(output) {
            let _ = stream.disconnect();
            bail!("Selected audio output is unavailable; refused fallback routing");
        }
        self.streams.insert(
            output.into(),
            Output {
                stream,
                drain: None,
                touched: Instant::now(),
            },
        );
        Ok(())
    }
    fn pump(&mut self, mixer: &mut Mixer, reports: &mpsc::Sender<Report>) -> Result<()> {
        self.iterate()?;
        let mut remove = Vec::new();
        let playing = mixer.outputs();
        for (name, output) in &mut self.streams {
            if !name.is_empty() && output.stream.get_device_name().as_deref() != Some(name.as_str())
            {
                mixer.remove_output(name);
                remove.push(name.clone());
                let _ = reports.send(Report::Status(format!(
                    "Selected output {name} changed; soundboard stopped instead of rerouting"
                )));
                continue;
            }
            if output.stream.get_state() != StreamState::Ready {
                mixer.remove_output(name);
                remove.push(name.clone());
                let _ = reports.send(Report::Status(format!(
                    "Soundboard output {name} disconnected"
                )));
                continue;
            }
            if !playing.contains(name) {
                if output.drain.is_none() {
                    output.drain = Some(output.stream.drain(None));
                    output.touched = Instant::now();
                }
                if output
                    .drain
                    .as_ref()
                    .is_some_and(|op| op.get_state() != pulse::operation::State::Running)
                    || output.touched.elapsed() > Duration::from_secs(3)
                {
                    remove.push(name.clone());
                }
                continue;
            }
            // Refill only requested frames. A suspended sink never blocks others.
            let bytes = output.stream.writable_size().unwrap_or(0).min(15360) / 8 * 8;
            if bytes == 0 {
                if output.touched.elapsed() > Duration::from_secs(5) {
                    mixer.remove_output(name);
                    remove.push(name.clone());
                    let _ = reports.send(Report::Status(format!(
                        "Audio output {name} stopped consuming samples"
                    )));
                }
                continue;
            }
            let mut samples = vec![0f32; bytes / 4];
            mixer.mix(name, &mut samples);
            let bytes: Vec<u8> = samples.into_iter().flat_map(f32::to_ne_bytes).collect();
            if let Err(error) = output.stream.write_copy(&bytes, 0, SeekMode::Relative) {
                mixer.remove_output(name);
                remove.push(name.clone());
                let _ = reports.send(Report::Status(format!("Audio write failed: {error}")));
            } else {
                output.touched = Instant::now();
            }
        }
        for name in remove {
            if let Some(mut output) = self.streams.remove(&name) {
                let _ = output.stream.disconnect();
            }
        }
        Ok(())
    }
    fn clear(&mut self) {
        for (_, mut output) in self.streams.drain() {
            if let Some(mut drain) = output.drain.take() {
                drain.cancel();
            }
            let _ = output.stream.disconnect();
        }
        let _ = self.iterate();
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.clear();
        self.context.disconnect();
    }
}
