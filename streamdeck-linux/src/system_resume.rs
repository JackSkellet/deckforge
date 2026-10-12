//! Read-only logind sleep-completion observation. No inhibit, suspend, lock,
//! unlock, device access, or service activation calls.
//! Sources: https://systemd.io/INHIBITOR_LOCKS/ and
//! https://github.com/systemd/systemd/blob/main/man/org.freedesktop.login1.xml
//! logind's false signal also covers a failed/cancelled sleep attempt; this
//! observes its paired completion notification, not physical suspend proof.
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::{
    sync::oneshot,
    time::{sleep, timeout},
};
use zbus::{
    MatchRule, Message, MessageStream, Proxy,
    message::Type,
    proxy::{CacheProperties, MethodFlags},
};

const LOGIN: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const INTERFACE: &str = "org.freedesktop.login1.Manager";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const CALL_TIMEOUT: Duration = Duration::from_millis(350);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Wake,
    Available,
    Unavailable(String),
}
#[derive(Default)]
struct Updates {
    status: Option<std::result::Result<(), String>>,
    pending: VecDeque<Event>,
    #[cfg(test)]
    signals_seen: usize,
}
impl Updates {
    fn publish(&mut self, event: Event) {
        let status = match &event {
            Event::Available => Some(Ok(())),
            Event::Unavailable(reason) => Some(Err(reason.clone())),
            Event::Wake => None,
        };
        if let Some(status) = status {
            if self.status.as_ref() == Some(&status) {
                return;
            }
            self.status = Some(status);
        }
        // Statuses deduplicate, but every genuine paired completion is retained
        // until the Host drains it; a UI pause must not silently lose wake cycles.
        self.pending.push_back(event);
    }
}
pub struct Worker {
    updates: Arc<Mutex<Updates>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        Self::start(None)
    }
    fn start(address: Option<String>) -> Self {
        let updates = Arc::new(Mutex::new(Updates::default()));
        let output = updates.clone();
        let (stop, stopped) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("deckforge-system-resume".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(r) => r,
                    Err(e) => {
                        output
                            .lock()
                            .unwrap()
                            .publish(Event::Unavailable(format!("Resume observer runtime: {e}")));
                        return;
                    }
                };
                runtime.block_on(async {
                    tokio::select! {biased;_=stopped=>{},_=monitor(address,output)=>{}}
                });
                runtime.shutdown_timeout(Duration::from_millis(200));
            });
        let thread = match thread {
            Ok(t) => Some(t),
            Err(e) => {
                updates
                    .lock()
                    .unwrap()
                    .publish(Event::Unavailable(format!("Start resume observer: {e}")));
                None
            }
        };
        Self {
            updates,
            stop: Some(stop),
            thread,
        }
    }
    pub fn drain(&self) -> Vec<Event> {
        self.updates.lock().unwrap().pending.drain(..).collect()
    }
}
impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Default)]
struct Pair {
    pending_owner: Option<String>,
}
impl Pair {
    fn reset(&mut self) {
        self.pending_owner = None;
    }
    fn observe(&mut self, owner: &str, starting: bool) -> bool {
        if starting {
            self.pending_owner = Some(owner.into());
            false
        } else {
            self.pending_owner.take().as_deref() == Some(owner)
        }
    }
}
async fn bus_call<R>(
    proxy: &Proxy<'_>,
    member: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<R>
where
    R: for<'a> zbus::zvariant::DynamicDeserialize<'a>,
{
    timeout(
        CALL_TIMEOUT,
        proxy.call_with_flags(member, MethodFlags::NoAutoStart.into(), body),
    )
    .await
    .with_context(|| format!("D-Bus {member} timed out"))??
    .context("D-Bus reply missing")
}
async fn owner(proxy: &Proxy<'_>) -> Result<Option<String>> {
    match timeout(
        CALL_TIMEOUT,
        proxy.call_with_flags::<_, _, String>(
            "GetNameOwner",
            MethodFlags::NoAutoStart.into(),
            &(LOGIN,),
        ),
    )
    .await
    .context("D-Bus owner query timed out")?
    {
        Ok(Some(owner)) => {
            ensure!(
                owner.starts_with(':'),
                "logind owner is not a unique bus name"
            );
            Ok(Some(owner))
        }
        Ok(None) => bail!("D-Bus owner query returned no reply"),
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
        {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
async fn monitor(address: Option<String>, output: Arc<Mutex<Updates>>) {
    loop {
        if let Err(e) = listen(address.as_deref(), &output).await {
            output
                .lock()
                .unwrap()
                .publish(Event::Unavailable(format!("System resume observer: {e:#}")));
        }
        sleep(RETRY).await;
    }
}
async fn listen(address: Option<&str>, output: &Arc<Mutex<Updates>>) -> Result<()> {
    let builder = match address {
        Some(a) => zbus::connection::Builder::address(a)?,
        None => zbus::connection::Builder::system()?,
    };
    let connection = timeout(
        CONNECT_TIMEOUT,
        builder.method_timeout(CALL_TIMEOUT).build(),
    )
    .await
    .context("System bus connection timed out")??;
    // One ordered stream processes BOTH signals. Separate selected streams could
    // process a completion ahead of an already-received owner-change boundary.
    let mut messages = MessageStream::from(&connection);
    messages.set_max_queued(128);
    let proxy = zbus::proxy::Builder::<Proxy<'_>>::new(&connection)
        .destination(BUS)?
        .path(BUS_PATH)?
        .interface(BUS)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let rules = [
        MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(LOGIN)?
            .path(PATH)?
            .interface(INTERFACE)?
            .member("PrepareForSleep")?
            .build(),
        MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(BUS)?
            .path(BUS_PATH)?
            .interface(BUS)?
            .member("NameOwnerChanged")?
            .add_arg(LOGIN)?
            .build(),
    ];
    for rule in rules {
        bus_call::<()>(&proxy, "AddMatch", &(rule.to_string(),)).await?;
    }
    publish_owner(output, owner(&proxy).await?.as_deref());
    let mut pair = Pair::default();
    while let Some(message) = messages.next().await {
        let message = message.context("System bus signal stream failed")?;
        consume(&message, &proxy, &mut pair, output).await?;
    }
    bail!("System bus disconnected")
}
fn publish_owner(output: &Arc<Mutex<Updates>>, owner: Option<&str>) {
    output.lock().unwrap().publish(if owner.is_some() {
        Event::Available
    } else {
        Event::Unavailable("systemd-logind is not running".into())
    });
}
async fn consume(
    message: &Message,
    proxy: &Proxy<'_>,
    pair: &mut Pair,
    output: &Arc<Mutex<Updates>>,
) -> Result<()> {
    if message.message_type() != Type::Signal {
        return Ok(());
    }
    let header = message.header();
    if header.sender().is_some_and(|s| s.as_str() == BUS)
        && header.path().is_some_and(|p| p.as_str() == BUS_PATH)
        && header.interface().is_some_and(|i| i.as_str() == BUS)
        && header
            .member()
            .is_some_and(|m| m.as_str() == "NameOwnerChanged")
    {
        let (name, _old, new): (String, String, String) = message
            .body()
            .deserialize()
            .context("Invalid D-Bus owner signal")?;
        if name == LOGIN {
            pair.reset();
            publish_owner(output, (!new.is_empty()).then_some(new.as_str()));
        }
        return Ok(());
    }
    if header.path().is_none_or(|p| p.as_str() != PATH)
        || header.interface().is_none_or(|i| i.as_str() != INTERFACE)
        || header
            .member()
            .is_none_or(|m| m.as_str() != "PrepareForSleep")
    {
        return Ok(());
    }
    // Read the current owner even for the first signal. This avoids trusting a
    // cold-start snapshot or queued signal from a service that has since left.
    let current = owner(proxy).await?;
    let Some(current) = current else {
        pair.reset();
        publish_owner(output, None);
        return Ok(());
    };
    if header
        .sender()
        .is_none_or(|sender| sender.as_str() != current)
    {
        return Ok(());
    }
    let (starting,): (bool,) = match message.body().deserialize() {
        Ok(v) => v,
        Err(e) => {
            pair.reset();
            output.lock().unwrap().publish(Event::Unavailable(format!(
                "Invalid logind sleep signal: {e}"
            )));
            return Ok(());
        }
    };
    let mut updates = output.lock().unwrap();
    updates.publish(Event::Available);
    #[cfg(test)]
    {
        updates.signals_seen += 1;
    }
    if pair.observe(&current, starting) {
        updates.publish(Event::Wake);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        time::Instant,
    };
    use zbus::Connection;
    struct PrivateBus {
        child: Option<Child>,
        address: String,
        socket: std::path::PathBuf,
        _directory: tempfile::TempDir,
    }
    impl PrivateBus {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("bus");
            let mut bus = Self {
                child: None,
                address: format!("unix:path={}", socket.display()),
                socket,
                _directory: directory,
            };
            bus.restart();
            bus
        }
        fn stop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        fn restart(&mut self) {
            self.stop();
            let _ = std::fs::remove_file(&self.socket);
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .arg(format!("--address={}", self.address))
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("dbus-daemon fixture dependency");
            let mut ready = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert!(ready.starts_with(&self.address));
            self.child = Some(child);
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            self.stop();
        }
    }
    async fn connection(bus: &PrivateBus) -> Connection {
        zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
    async fn service(bus: &PrivateBus) -> Connection {
        let c = connection(bus).await;
        c.request_name(LOGIN).await.unwrap();
        c
    }
    async fn signal(service: &Connection, start: bool) {
        service
            .emit_signal(None::<&str>, PATH, INTERFACE, "PrepareForSleep", &(start,))
            .await
            .unwrap();
    }
    async fn event(worker: &Worker, wanted: impl Fn(&Event) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut events = Vec::new();
        loop {
            events.extend(worker.drain());
            if events.iter().any(&wanted) {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "resume event fixture timed out: {events:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    async fn seen(worker: &Worker, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while worker.updates.lock().unwrap().signals_seen < count {
            assert!(Instant::now() < deadline, "sleep signal fixture timed out");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    #[test]
    fn pairing_requires_true_then_false_from_same_owner_and_resets() {
        let mut pair = Pair::default();
        assert!(!pair.observe(":1.1", false));
        assert!(!pair.observe(":1.1", true));
        assert!(!pair.observe(":1.1", true));
        assert!(pair.observe(":1.1", false));
        assert!(!pair.observe(":1.1", false));
        pair.observe(":1.1", true);
        pair.reset();
        assert!(!pair.observe(":1.1", false));
        pair.observe(":1.1", true);
        assert!(!pair.observe(":1.2", false));
    }
    #[tokio::test]
    async fn private_logind_pairs_once_and_ignores_cold_start_duplicates_and_unrelated_signals() {
        let bus = PrivateBus::new();
        let service = service(&bus).await;
        let worker = Worker::start(Some(bus.address.clone()));
        assert_eq!(
            event(&worker, |e| matches!(e, Event::Available)).await,
            vec![Event::Available]
        );
        signal(&service, false).await;
        seen(&worker, 1).await;
        assert!(worker.drain().is_empty());
        signal(&service, true).await;
        seen(&worker, 2).await;
        signal(&service, true).await;
        seen(&worker, 3).await;
        assert!(worker.drain().is_empty());
        // A foreign connection cannot complete logind's pending pair.
        let other = connection(&bus).await;
        signal(&other, false).await;
        service
            .emit_signal(
                None::<&str>,
                "/ScreenSaver",
                "org.freedesktop.ScreenSaver",
                "ActiveChanged",
                &(false,),
            )
            .await
            .unwrap();
        service
            .emit_signal(
                None::<&str>,
                PATH,
                INTERFACE,
                "PrepareForShutdown",
                &(false,),
            )
            .await
            .unwrap();
        signal(&service, false).await;
        seen(&worker, 4).await;
        assert_eq!(worker.drain(), vec![Event::Wake]);
        signal(&service, false).await;
        seen(&worker, 5).await;
        assert!(worker.drain().is_empty());
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_millis(500));
    }
    #[tokio::test]
    async fn private_owner_replacement_and_reacquisition_reset_pending_pair() {
        let bus = PrivateBus::new();
        let first = service(&bus).await;
        let worker = Worker::start(Some(bus.address.clone()));
        event(&worker, |e| matches!(e, Event::Available)).await;
        signal(&first, true).await;
        seen(&worker, 1).await;
        first.release_name(LOGIN).await.unwrap();
        assert!(
            event(&worker, |e| matches!(e, Event::Unavailable(_)))
                .await
                .iter()
                .all(|e| *e != Event::Wake)
        );
        let second = service(&bus).await;
        assert_eq!(
            event(&worker, |e| matches!(e, Event::Available)).await,
            vec![Event::Available]
        );
        signal(&first, false).await;
        signal(&second, false).await;
        seen(&worker, 2).await;
        assert!(worker.drain().is_empty());
        signal(&second, true).await;
        seen(&worker, 3).await;
        second.release_name(LOGIN).await.unwrap();
        event(&worker, |e| matches!(e, Event::Unavailable(_))).await;
        // Even the SAME unique connection losing/reacquiring the well-known
        // service name is a new epoch; it must not complete an old pair.
        second.request_name(LOGIN).await.unwrap();
        event(&worker, |e| matches!(e, Event::Available)).await;
        signal(&second, false).await;
        seen(&worker, 4).await;
        assert!(worker.drain().is_empty());
        signal(&second, true).await;
        signal(&second, false).await;
        seen(&worker, 6).await;
        assert_eq!(worker.drain(), vec![Event::Wake]);
    }
    #[tokio::test]
    async fn private_bus_restart_resets_pending_pair_and_reconnects() {
        let mut bus = PrivateBus::new();
        let first = service(&bus).await;
        let worker = Worker::start(Some(bus.address.clone()));
        event(&worker, |e| matches!(e, Event::Available)).await;
        signal(&first, true).await;
        seen(&worker, 1).await;
        bus.stop();
        let lost = event(&worker, |e| matches!(e, Event::Unavailable(_))).await;
        assert!(lost.iter().all(|e| *e != Event::Wake));
        drop(first);
        bus.restart();
        let second = service(&bus).await;
        assert!(
            event(&worker, |e| matches!(e, Event::Available))
                .await
                .iter()
                .all(|e| *e != Event::Wake)
        );
        signal(&second, false).await;
        seen(&worker, 2).await;
        assert!(worker.drain().is_empty());
        signal(&second, true).await;
        signal(&second, false).await;
        seen(&worker, 4).await;
        assert_eq!(worker.drain(), vec![Event::Wake]);
    }
    #[tokio::test]
    async fn missing_service_does_not_activate_and_malformed_signal_does_not_wake() {
        let bus = PrivateBus::new();
        let worker = Worker::start(Some(bus.address.clone()));
        let unavailable = event(&worker, |e| matches!(e, Event::Unavailable(_))).await;
        assert_eq!(unavailable.len(), 1);
        let client = connection(&bus).await;
        let proxy = Proxy::new(&client, BUS, BUS_PATH, BUS).await.unwrap();
        assert_eq!(owner(&proxy).await.unwrap(), None);
        let service = service(&bus).await;
        event(&worker, |e| matches!(e, Event::Available)).await;
        signal(&service, true).await;
        seen(&worker, 1).await;
        service
            .emit_signal(
                None::<&str>,
                PATH,
                INTERFACE,
                "PrepareForSleep",
                &("invalid",),
            )
            .await
            .unwrap();
        let malformed = event(&worker, |e| matches!(e, Event::Unavailable(_))).await;
        assert!(malformed.iter().all(|e| *e != Event::Wake));
        signal(&service, false).await;
        seen(&worker, 2).await;
        assert_eq!(worker.drain(), vec![Event::Available]);
    }
    #[test]
    fn unavailable_retry_and_connect_can_be_cancelled_without_waiting_for_retry() {
        let directory = tempfile::tempdir().unwrap();
        let worker = Worker::start(Some(format!(
            "unix:path={}",
            directory.path().join("missing").display()
        )));
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(500));
    }
    #[test]
    fn drop_cancels_a_connected_peer_that_never_finishes_dbus_handshake() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unresponsive-bus");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let worker = Worker::start(Some(format!("unix:path={}", path.display())));
        let deadline = Instant::now() + Duration::from_secs(2);
        let _peer = loop {
            match listener.accept() {
                Ok((peer, _)) => break peer,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("{e}"),
            };
            assert!(Instant::now() < deadline, "fixture peer was not reached");
            std::thread::sleep(Duration::from_millis(2));
        };
        let started = Instant::now();
        drop(worker);
        assert!(started.elapsed() < Duration::from_millis(500));
    }
    #[test]
    fn statuses_deduplicate_but_distinct_wake_cycles_do_not() {
        let mut updates = Updates::default();
        updates.publish(Event::Available);
        updates.publish(Event::Available);
        updates.publish(Event::Wake);
        updates.publish(Event::Wake);
        assert_eq!(updates.pending.len(), 3);
        updates.publish(Event::Unavailable("offline".into()));
        updates.publish(Event::Unavailable("offline".into()));
        assert_eq!(updates.pending.len(), 4);
    }
}
