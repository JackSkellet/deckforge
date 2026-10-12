//! Native PulseAudio/PipeWire volume monitoring and control, on its own worker.
//! No shell commands, media-key injection, daemon spawning, or audio playback.
use anyhow::{Context as _, Result, bail, ensure};
use libpulse_binding as pulse;
use pulse::{
    callbacks::ListResult,
    context::{Context, State as ContextState},
    mainloop::standard::{IterateResult, Mainloop},
    operation::{Operation, State as OperationState},
    volume::{ChannelVolumes, Volume},
};
use serde::Serialize;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(200);
const QUERY_TIMEOUT: Duration = Duration::from_millis(800);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1200);
const RETRY: Duration = Duration::from_secs(1);
const ERROR_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct State {
    pub sink: String,
    pub name: String,
    pub percent: f64,
    pub muted: bool,
}
enum Command {
    Wake,
    Refresh,
    Adjust(i8, f64),
    Toggle,
}
pub struct Worker {
    sender: mpsc::SyncSender<Command>,
    reports: mpsc::Receiver<Result<State>>,
    watch: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        Self::for_sink(None)
    }
    /// Some(name) targets exactly that sink, never falling back to default. The
    /// live app uses None; named targets permit isolated virtual-sink diagnostics.
    pub fn for_sink(sink: Option<String>) -> Self {
        let (sender, commands) = mpsc::sync_channel(32);
        let (reporter, reports) = mpsc::sync_channel(32);
        let watch = Arc::new(AtomicBool::new(false));
        let watched = watch.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            let mut connection: Option<Connection> = None;
            let mut next_poll = Instant::now();
            let mut was_watched = false;
            let mut last_state: Option<State> = None;
            let mut last_error: Option<Instant> = None;
            while !stopped.load(Ordering::Acquire) {
                let monitoring = watched.load(Ordering::Acquire);
                if monitoring != was_watched {
                    was_watched = monitoring;
                    next_poll = Instant::now();
                    last_state = None;
                }
                let timeout = if monitoring {
                    next_poll
                        .saturating_duration_since(Instant::now())
                        .min(POLL)
                } else {
                    POLL
                };
                let command = match commands.recv_timeout(timeout) {
                    Ok(Command::Wake) => continue,
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout)
                        if monitoring && Instant::now() >= next_poll =>
                    {
                        Command::Refresh
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let result = (|| -> Result<State> {
                    if let Some(name) = &sink {
                        validate_sink_name(name)?;
                    }
                    if connection.is_none() {
                        connection = Some(Connection::new(&stopped)?);
                    }
                    let connection = connection.as_mut().unwrap();
                    let current = connection.read_target(sink.as_deref(), &stopped)?;
                    match command {
                        Command::Adjust(ticks, step) => {
                            let volume = adjusted_volumes(&current.volumes, ticks, step)?;
                            if volume != current.volumes {
                                connection.set_volume(&current.state.sink, &volume, &stopped)?;
                                connection
                                    .read_target(sink.as_deref(), &stopped)
                                    .map(|sink| sink.state)
                            } else {
                                Ok(current.state)
                            }
                        }
                        Command::Toggle => {
                            connection.set_mute(
                                &current.state.sink,
                                !current.state.muted,
                                &stopped,
                            )?;
                            connection
                                .read_target(sink.as_deref(), &stopped)
                                .map(|sink| sink.state)
                        }
                        Command::Refresh => Ok(current.state),
                        Command::Wake => unreachable!(),
                    }
                })();
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                next_poll = Instant::now() + if result.is_ok() { POLL } else { RETRY };
                match result {
                    Ok(state) => {
                        let recovered = last_error.take().is_some();
                        if (recovered || last_state.as_ref() != Some(&state))
                            && reporter.try_send(Ok(state.clone())).is_ok()
                        {
                            last_state = Some(state);
                        }
                    }
                    Err(error) => {
                        connection = None;
                        last_state = None;
                        if last_error.is_none_or(|last| last.elapsed() >= ERROR_INTERVAL)
                            && reporter.try_send(Err(error)).is_ok()
                        {
                            last_error = Some(Instant::now());
                        }
                    }
                }
            }
            drop(connection);
        });
        Self {
            sender,
            reports,
            watch,
            stop,
            thread: Some(thread),
        }
    }
    pub fn watch(&self, enabled: bool) {
        if self.watch.swap(enabled, Ordering::AcqRel) != enabled {
            let _ = self.sender.try_send(Command::Wake);
        }
    }
    pub fn adjust(&self, ticks: i8, step: f64) -> Result<()> {
        validate_step(step)?;
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "Volume worker is stopped"
        );
        if ticks == 0 {
            return Ok(());
        }
        self.send(Command::Adjust(ticks, step))
    }
    pub fn toggle(&self) -> Result<()> {
        self.send(Command::Toggle)
    }
    fn send(&self, command: Command) -> Result<()> {
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "Volume worker is stopped"
        );
        self.sender
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Volume command queue is full or disconnected"))
    }
    pub fn drain(&self) -> Vec<Result<State>> {
        self.reports.try_iter().collect()
    }
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.try_send(Command::Wake);
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

struct Sink {
    state: State,
    volumes: ChannelVolumes,
}
struct Connection {
    context: Context,
    mainloop: Mainloop,
}
impl Connection {
    fn new(stop: &AtomicBool) -> Result<Self> {
        let mainloop = Mainloop::new().context("Create volume event loop")?;
        let mut context =
            Context::new(&mainloop, "Deckforge volume").context("Create volume context")?;
        context
            .connect(None, pulse::context::FlagSet::NOAUTOSPAWN, None)
            .context("Connect to PulseAudio/PipeWire")?;
        let mut connection = Self { context, mainloop };
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            ensure!(!stop.load(Ordering::Acquire), "Volume connection cancelled");
            connection.iterate()?;
            if connection.context.get_state() == ContextState::Ready {
                return Ok(connection);
            }
            ensure!(Instant::now() < deadline, "Volume connection timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn iterate(&mut self) -> Result<()> {
        for _ in 0..16 {
            match self.mainloop.iterate(false) {
                IterateResult::Success(0) => break,
                IterateResult::Success(_) => {}
                _ => bail!("Volume event loop stopped"),
            }
        }
        ensure!(
            !matches!(
                self.context.get_state(),
                ContextState::Failed | ContextState::Terminated
            ),
            "PulseAudio/PipeWire volume connection was lost"
        );
        Ok(())
    }
    fn ready(&mut self, stop: &AtomicBool) -> Result<()> {
        ensure!(!stop.load(Ordering::Acquire), "Volume operation cancelled");
        self.iterate()?;
        ensure!(
            self.context.get_state() == ContextState::Ready,
            "Volume connection is not ready"
        );
        Ok(())
    }
    fn wait<C: ?Sized>(&mut self, operation: &mut Operation<C>, stop: &AtomicBool) -> Result<()> {
        let deadline = Instant::now() + QUERY_TIMEOUT;
        while operation.get_state() == OperationState::Running {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                operation.cancel();
                bail!("Volume query/update cancelled or timed out");
            }
            if let Err(error) = self.iterate() {
                operation.cancel();
                return Err(error);
            }
            if operation.get_state() == OperationState::Running {
                thread::sleep(Duration::from_millis(2));
            }
        }
        ensure!(
            operation.get_state() == OperationState::Done,
            "Volume operation was cancelled"
        );
        Ok(())
    }
    fn default_name(&mut self, stop: &AtomicBool) -> Result<String> {
        self.ready(stop)?;
        let value = Rc::new(RefCell::new(None));
        let received = value.clone();
        let mut operation = self.context.introspect().get_server_info(move |info| {
            *received.borrow_mut() = info.default_sink_name.as_ref().map(ToString::to_string);
        });
        self.wait(&mut operation, stop)?;
        let name = value
            .borrow_mut()
            .take()
            .context("Audio server has no default output")?;
        validate_sink_name(&name)?;
        Ok(name)
    }
    fn read_target(&mut self, target: Option<&str>, stop: &AtomicBool) -> Result<Sink> {
        // Resolve the default on every poll and action, never from cached state.
        let name = match target {
            Some(name) => {
                validate_sink_name(name)?;
                name.to_owned()
            }
            None => self.default_name(stop)?,
        };
        self.ready(stop)?;
        let value = Rc::new(RefCell::new(None));
        let received = value.clone();
        let failed = Rc::new(RefCell::new(false));
        let error = failed.clone();
        let mut operation = self
            .context
            .introspect()
            .get_sink_info_by_name(&name, move |result| match result {
                ListResult::Item(info) => {
                    *received.borrow_mut() = Some((
                        info.name.as_ref().map(ToString::to_string),
                        info.description.as_ref().map(ToString::to_string),
                        info.volume,
                        info.mute,
                    ));
                }
                ListResult::Error => *error.borrow_mut() = true,
                ListResult::End => {}
            });
        self.wait(&mut operation, stop)?;
        ensure!(!*failed.borrow(), "Could not query audio output {name}");
        let (sink, description, volumes, muted) = value
            .borrow_mut()
            .take()
            .with_context(|| format!("Audio output {name} was not found"))?;
        let sink = sink.context("Audio server returned an output without a name")?;
        validate_sink_name(&sink)?;
        ensure!(
            sink == name,
            "Audio server returned a different sink than requested"
        );
        let percent = volume_percent(&volumes)?;
        Ok(Sink {
            state: State {
                sink: sink.clone(),
                name: description.filter(|d| !d.is_empty()).unwrap_or(sink),
                percent,
                muted,
            },
            volumes,
        })
    }
    fn set_volume(
        &mut self,
        name: &str,
        volumes: &ChannelVolumes,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.ready(stop)?;
        validate_sink_name(name)?;
        let success = Rc::new(RefCell::new(None));
        let confirmed = success.clone();
        let mut operation = self.context.introspect().set_sink_volume_by_name(
            name,
            volumes,
            Some(Box::new(move |ok| *confirmed.borrow_mut() = Some(ok))),
        );
        self.wait(&mut operation, stop)?;
        ensure!(
            *success.borrow() == Some(true),
            "Audio server rejected the volume update"
        );
        Ok(())
    }
    fn set_mute(&mut self, name: &str, muted: bool, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        validate_sink_name(name)?;
        let success = Rc::new(RefCell::new(None));
        let confirmed = success.clone();
        let mut operation = self.context.introspect().set_sink_mute_by_name(
            name,
            muted,
            Some(Box::new(move |ok| *confirmed.borrow_mut() = Some(ok))),
        );
        self.wait(&mut operation, stop)?;
        ensure!(
            *success.borrow() == Some(true),
            "Audio server rejected the mute update"
        );
        Ok(())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.context.disconnect();
    }
}

fn validate_sink_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 4096 && !name.contains('\0'),
        "Invalid PulseAudio sink name"
    );
    Ok(())
}
fn validate_step(step: f64) -> Result<()> {
    ensure!(
        step.is_finite() && (0.1..=100.0).contains(&step),
        "Volume step must be between 0.1 and 100 percent"
    );
    Ok(())
}
fn volume_percent(volumes: &ChannelVolumes) -> Result<f64> {
    ensure!(
        volumes.is_valid() && volumes.len() > 0,
        "Audio server returned invalid channel volumes"
    );
    Ok(f64::from(volumes.max().0) * 100.0 / f64::from(Volume::NORMAL.0))
}
fn adjusted_volumes(volumes: &ChannelVolumes, ticks: i8, step: f64) -> Result<ChannelVolumes> {
    validate_step(step)?;
    let current = volume_percent(volumes)?;
    if ticks == 0 {
        return Ok(*volumes);
    }
    // Never introduce boost. Existing boost can only stay fixed or decrease by
    // the requested delta; reducing 130% by 2% yields 128%, not an abrupt 100%.
    let target = (current + f64::from(ticks) * step).clamp(0.0, current.max(100.0));
    let target = Volume((target * f64::from(Volume::NORMAL.0) / 100.0).round() as u32);
    let mut changed = *volumes;
    changed
        .scale(target)
        .context("Could not preserve the output's channel balance")?;
    ensure!(changed.is_valid(), "Calculated channel volumes are invalid");
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn volumes(left: f64, right: f64) -> ChannelVolumes {
        let mut result = ChannelVolumes::default();
        result.set(2, Volume::NORMAL);
        result.get_mut()[0] = Volume((left / 100.0 * f64::from(Volume::NORMAL.0)).round() as u32);
        result.get_mut()[1] = Volume((right / 100.0 * f64::from(Volume::NORMAL.0)).round() as u32);
        result
    }
    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.003,
            "actual={actual},expected={expected}"
        );
    }
    #[test]
    fn percentage_and_scaling_use_max_channel_and_preserve_balance() {
        let initial = volumes(80.0, 40.0);
        close(volume_percent(&initial).unwrap(), 80.0);
        let result = adjusted_volumes(&initial, 2, 5.0).unwrap();
        close(volume_percent(&result).unwrap(), 90.0);
        close(
            f64::from(result.get()[1].0) * 100.0 / f64::from(Volume::NORMAL.0),
            45.0,
        );
        assert_eq!(adjusted_volumes(&initial, 0, 5.0).unwrap(), initial);
    }
    #[test]
    fn bounds_and_preexisting_boost_do_not_jump() {
        close(
            volume_percent(&adjusted_volumes(&volumes(95.0, 95.0), 2, 5.0).unwrap()).unwrap(),
            100.0,
        );
        close(
            volume_percent(&adjusted_volumes(&volumes(130.0, 65.0), 1, 2.0).unwrap()).unwrap(),
            130.0,
        );
        close(
            volume_percent(&adjusted_volumes(&volumes(130.0, 65.0), -1, 2.0).unwrap()).unwrap(),
            128.0,
        );
        close(
            volume_percent(&adjusted_volumes(&volumes(130.0, 65.0), -10, 5.0).unwrap()).unwrap(),
            80.0,
        );
        close(
            volume_percent(&adjusted_volumes(&volumes(5.0, 5.0), i8::MIN, 100.0).unwrap()).unwrap(),
            0.0,
        );
    }
    #[test]
    fn zero_volume_stays_stable_and_can_increase() {
        let silent = volumes(0.0, 0.0);
        assert_eq!(adjusted_volumes(&silent, 0, 1.0).unwrap(), silent);
        let increased = adjusted_volumes(&silent, 1, 0.1).unwrap();
        close(volume_percent(&increased).unwrap(), 0.1);
        assert_eq!(increased.get()[0], increased.get()[1]);
    }
    #[test]
    fn malformed_values_are_rejected_without_pulse_operations() {
        for step in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            0.09,
            100.1,
        ] {
            assert!(validate_step(step).is_err());
        }
        for name in ["", "bad\0name"] {
            assert!(validate_sink_name(name).is_err());
        }
        assert!(volume_percent(&ChannelVolumes::default()).is_err());
        let mut invalid = volumes(100.0, 100.0);
        invalid.get_mut()[0] = Volume(u32::MAX);
        assert!(volume_percent(&invalid).is_err());
    }
    #[test]
    fn idle_worker_zero_ticks_and_invalid_steps_do_not_connect() {
        let worker = Worker::for_sink(Some("unused-test-sink".into()));
        assert!(worker.drain().is_empty());
        assert!(worker.adjust(0, 1.0).is_ok());
        assert!(worker.adjust(1, f64::NAN).is_err());
        assert!(worker.drain().is_empty());
        let start = Instant::now();
        worker.cancel();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
