//! Read-only desktop activity. No service activation, lock, inhibit, or activity
//! simulation calls. Sources:
//! https://github.com/KDE/kscreenlocker/blob/master/interface.cpp
//! https://github.com/KDE/kidletime/blob/master/src/kidletime.h
//! https://github.com/GNOME/mutter/blob/main/src/backends/meta-idle-monitor.c
//! https://github.com/GNOME/gnome-shell/blob/main/js/ui/shellDBus.js
//! https://github.com/systemd/systemd/blob/main/man/org.freedesktop.login1.xml
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::{
    sync::oneshot,
    time::{MissedTickBehavior, timeout},
};
use zbus::{
    Connection, Proxy,
    proxy::{CacheProperties, MethodFlags},
    zvariant::{DynamicDeserialize, DynamicType, OwnedObjectPath, OwnedValue},
};

const CALL_TIMEOUT: Duration = Duration::from_millis(350);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const KDE: &str = "org.kde.screensaver";
const MUTTER: &str = "org.gnome.Mutter.IdleMonitor";
const LOGIN: &str = "org.freedesktop.login1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct State {
    pub idle_ms: Option<u64>,
    pub locked: Option<bool>,
    pub source: String,
    pub error: Option<String>,
}
impl State {
    fn unknown(error: String) -> Self {
        Self {
            idle_ms: None,
            locked: None,
            source: "unavailable".into(),
            error: Some(error),
        }
    }
}

#[derive(Default)]
struct Updates {
    last: Option<State>,
    pending: VecDeque<State>,
}
impl Updates {
    fn publish(&mut self, state: State) {
        if self.last.as_ref() == Some(&state) {
            return;
        }
        self.last = Some(state.clone());
        if self.pending.len() == 16 {
            self.pending.pop_front();
        }
        self.pending.push_back(state);
    }
}

pub struct Worker {
    updates: Arc<Mutex<Updates>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        Self::start(None, None, std::env::var("XDG_SESSION_ID").ok())
    }
    // Explicit addresses keep fixture buses isolated from the user's session.
    fn start(
        session_address: Option<String>,
        system_address: Option<String>,
        session_id: Option<String>,
    ) -> Self {
        let updates = Arc::new(Mutex::new(Updates::default()));
        let output = updates.clone();
        let (stop, stopped) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("deckforge-session-activity".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        output
                            .lock()
                            .unwrap()
                            .publish(State::unknown(format!("Desktop monitor runtime: {error}")));
                        return;
                    }
                };
                runtime.block_on(async {
                    tokio::select! {
                        biased;
                        _ = stopped => {},
                        _ = monitor(session_address, system_address, session_id, output) => {},
                    }
                });
                runtime.shutdown_timeout(Duration::from_millis(200));
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                updates
                    .lock()
                    .unwrap()
                    .publish(State::unknown(format!("Start desktop monitor: {error}")));
                None
            }
        };
        Self {
            updates,
            stop: Some(stop),
            thread,
        }
    }
    pub fn drain(&self) -> Vec<State> {
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

async fn connect(
    cached: &mut Option<Connection>,
    address: Option<&str>,
    system: bool,
) -> Result<Connection> {
    if let Some(connection) = cached.as_ref().filter(|c| !c.is_closed()) {
        return Ok(connection.clone());
    }
    *cached = None;
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address)?,
        None if system => zbus::connection::Builder::system()?,
        None => zbus::connection::Builder::session()?,
    };
    let connection = timeout(
        CONNECT_TIMEOUT,
        builder.method_timeout(CALL_TIMEOUT).build(),
    )
    .await
    .context("D-Bus connection timed out")??;
    *cached = Some(connection.clone());
    Ok(connection)
}

async fn monitor(
    session_address: Option<String>,
    system_address: Option<String>,
    session_id: Option<String>,
    output: Arc<Mutex<Updates>>,
) {
    let mut session = None;
    let mut system = None;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let (session, system) = tokio::join!(
            connect(&mut session, session_address.as_deref(), false),
            connect(&mut system, system_address.as_deref(), true),
        );
        let state = sample(
            session.as_ref().ok(),
            system.as_ref().ok(),
            session_id.as_deref(),
        )
        .await;
        output.lock().unwrap().publish(state);
    }
}

async fn call<B, R>(
    connection: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<R>
where
    B: Serialize + DynamicType,
    R: for<'a> DynamicDeserialize<'a>,
{
    timeout(CALL_TIMEOUT, async {
        let proxy = zbus::proxy::Builder::<Proxy<'_>>::new(connection)
            .destination(destination)?
            .path(path)?
            .interface(interface)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        proxy
            .call_with_flags(method, MethodFlags::NoAutoStart.into(), body)
            .await?
            .context("D-Bus method returned no reply")
    })
    .await
    .with_context(|| format!("{destination}.{method} timed out"))?
}
async fn present(connection: &Connection, name: &str) -> Result<bool> {
    call(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        &(name,),
    )
    .await
}

struct Reading<T> {
    result: Result<T>,
    source: &'static str,
}
async fn idle(session: Option<&Connection>) -> Reading<u64> {
    let mut source = "unavailable";
    let result = async {
        let connection = session.context("Session bus unavailable")?;
        let mut kde_error = None;
        if present(connection, KDE).await? {
            source = "KDE ScreenSaver";
            let value: Result<u32> = call(
                connection,
                KDE,
                "/ScreenSaver",
                "org.freedesktop.ScreenSaver",
                "GetSessionIdleTime",
                &(),
            )
            .await;
            match value {
                Ok(value) if value <= i32::MAX as u32 => return Ok(u64::from(value)),
                Ok(_) => {
                    kde_error = Some("KDE idle counter returned a negative signed value".into())
                }
                Err(error) => kde_error = Some(format!("KDE idle: {error:#}")),
            }
        }
        if present(connection, MUTTER).await? {
            source = "Mutter IdleMonitor";
            let value: u64 = call(
                connection,
                MUTTER,
                "/org/gnome/Mutter/IdleMonitor/Core",
                MUTTER,
                "GetIdletime",
                &(),
            )
            .await?;
            ensure!(
                value <= i64::MAX as u64,
                "Mutter idle counter is unsupported or negative"
            );
            return Ok(value);
        }
        bail!(kde_error.unwrap_or_else(|| "No supported desktop idle service is running".into()))
    }
    .await;
    Reading { result, source }
}

async fn locked(
    session: Option<&Connection>,
    system: Option<&Connection>,
    session_id: Option<&str>,
) -> Reading<bool> {
    if let Some(connection) = session
        && matches!(present(connection, KDE).await, Ok(true))
    {
        let result = call(
            connection,
            KDE,
            "/ScreenSaver",
            "org.freedesktop.ScreenSaver",
            "GetActive",
            &(),
        )
        .await;
        if result.is_ok() {
            return Reading {
                result,
                source: "KDE ScreenSaver",
            };
        }
    }
    // GNOME GetActive reports screenShield.active, not screenShield.locked.
    // GNOME updates logind LockedHint separately when its actual lock state changes.
    let result = async {
        let connection =
            system.context("System bus unavailable; exact session lock state unknown")?;
        let by_pid: Result<OwnedObjectPath> = call(
            connection,
            LOGIN,
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "GetSessionByPID",
            &(std::process::id(),),
        )
        .await;
        let (path, expected_id) = match by_pid {
            Ok(path) => (path, None),
            Err(error) => {
                let id = session_id
                    .filter(|id| {
                        !id.is_empty() && id.len() <= 128 && !matches!(*id, "auto" | "self")
                    })
                    .with_context(|| {
                        format!("No exact graphical session for this process: {error:#}")
                    })?;
                (
                    call(
                        connection,
                        LOGIN,
                        "/org/freedesktop/login1",
                        "org.freedesktop.login1.Manager",
                        "GetSession",
                        &(id,),
                    )
                    .await?,
                    Some(id),
                )
            }
        };
        let properties = call(
            connection,
            LOGIN,
            path.as_str(),
            "org.freedesktop.DBus.Properties",
            "GetAll",
            &("org.freedesktop.login1.Session",),
        )
        .await?;
        validate_session(properties, expected_id)
    }
    .await;
    Reading {
        result,
        source: "logind Session.LockedHint",
    }
}

fn validate_session(
    mut properties: HashMap<String, OwnedValue>,
    expected_id: Option<&str>,
) -> Result<bool> {
    let kind = String::try_from(properties.remove("Type").context("Session Type missing")?)?;
    ensure!(
        matches!(kind.as_str(), "wayland" | "x11"),
        "Current session is not graphical"
    );
    let (owner, _): (u32, OwnedObjectPath) = properties
        .remove("User")
        .context("Session User missing")?
        .try_into()?;
    // SAFETY: geteuid is read-only and takes no pointers.
    ensure!(
        owner == unsafe { libc::geteuid() },
        "Graphical session belongs to another user"
    );
    if let Some(expected) = expected_id {
        let id = String::try_from(properties.remove("Id").context("Session Id missing")?)?;
        ensure!(id == expected, "Session ID resolved to a different session");
    }
    bool::try_from(
        properties
            .remove("LockedHint")
            .context("Session LockedHint missing")?,
    )
    .context("Invalid session lock hint")
}

async fn sample(
    session: Option<&Connection>,
    system: Option<&Connection>,
    session_id: Option<&str>,
) -> State {
    let (idle, locked) = tokio::join!(idle(session), locked(session, system, session_id));
    let errors: Vec<_> = [idle.result.as_ref().err(), locked.result.as_ref().err()]
        .into_iter()
        .flatten()
        .map(|error| format!("{error:#}"))
        .collect();
    State {
        idle_ms: idle.result.ok(),
        locked: locked.result.ok(),
        source: format!("idle: {}; lock: {}", idle.source, locked.source),
        error: (!errors.is_empty()).then(|| errors.join("; ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        time::Instant,
    };
    use zbus::{
        message::{Flags, Header},
        zvariant::{ObjectPath, Value},
    };

    struct PrivateBus {
        child: Child,
        address: String,
    }
    impl PrivateBus {
        fn new() -> Self {
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("dbus-daemon fixture dependency");
            let mut address = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            assert!(address.starts_with("unix:"));
            Self {
                child,
                address: address.trim().into(),
            }
        }
    }
    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    #[derive(Default)]
    struct Fixture {
        idle: AtomicU64,
        locked: AtomicBool,
        unsupported: AtomicBool,
        delay_ms: AtomicU64,
        idle_calls: AtomicUsize,
    }
    fn readonly(header: Header<'_>) {
        assert!(header.primary().flags().contains(Flags::NoAutoStart));
        assert!(
            !header
                .primary()
                .flags()
                .contains(Flags::AllowInteractiveAuth)
        );
    }
    struct Kde(Arc<Fixture>);
    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl Kde {
        async fn get_session_idle_time(
            &self,
            #[zbus(header)] header: Header<'_>,
        ) -> zbus::fdo::Result<u32> {
            readonly(header);
            self.0.idle_calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(
                self.0.delay_ms.load(Ordering::SeqCst),
            ))
            .await;
            if self.0.unsupported.load(Ordering::SeqCst) {
                return Err(zbus::fdo::Error::NotSupported(
                    "GetSessionIdleTime is not supported on this platform".into(),
                ));
            }
            Ok(self.0.idle.load(Ordering::SeqCst) as u32)
        }
        fn get_active(&self, #[zbus(header)] header: Header<'_>) -> bool {
            readonly(header);
            self.0.locked.load(Ordering::SeqCst)
        }
    }
    struct Mutter;
    #[zbus::interface(name = "org.gnome.Mutter.IdleMonitor")]
    impl Mutter {
        fn get_idletime(&self, #[zbus(header)] header: Header<'_>) -> u64 {
            readonly(header);
            4700
        }
    }
    struct Login;
    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Login {
        fn get_session_by_pid(
            &self,
            pid: u32,
            #[zbus(header)] header: Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            readonly(header);
            assert_eq!(pid, std::process::id());
            Err(zbus::fdo::Error::Failed("NoSessionForPID".into()))
        }
        fn get_session(&self, id: &str, #[zbus(header)] header: Header<'_>) -> OwnedObjectPath {
            readonly(header);
            assert_eq!(id, "fixture");
            OwnedObjectPath::try_from("/org/freedesktop/login1/session/fixture").unwrap()
        }
    }
    struct LoginSession;
    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl LoginSession {
        #[zbus(property)]
        fn id(&self) -> &str {
            "fixture"
        }
        #[zbus(property, name = "Type")]
        fn session_type(&self) -> &str {
            "wayland"
        }
        #[zbus(property)]
        fn user(&self) -> (u32, OwnedObjectPath) {
            (
                unsafe { libc::geteuid() },
                OwnedObjectPath::try_from("/org/freedesktop/login1/user/test").unwrap(),
            )
        }
        #[zbus(property)]
        fn locked_hint(&self) -> bool {
            true
        }
    }
    async fn connection(bus: &PrivateBus) -> Connection {
        zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
    async fn fixture(bus: &PrivateBus, data: Arc<Fixture>) -> Connection {
        zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name(KDE)
            .unwrap()
            .serve_at("/ScreenSaver", Kde(data))
            .unwrap()
            .build()
            .await
            .unwrap()
    }
    async fn update(worker: &Worker) -> State {
        let end = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(state) = worker.drain().pop() {
                return state;
            }
            assert!(Instant::now() < end, "monitor did not publish");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn private_kde_reports_milliseconds_and_lock_and_deduplicates_unchanged_poll() {
        let bus = PrivateBus::new();
        let data = Arc::new(Fixture::default());
        data.idle.store(1234, Ordering::SeqCst);
        let _service = fixture(&bus, data.clone()).await;
        let worker = Worker::start(Some(bus.address.clone()), Some(bus.address.clone()), None);
        let state = update(&worker).await;
        assert_eq!(state.idle_ms, Some(1234));
        assert_eq!(state.locked, Some(false));
        assert_eq!(state.error, None);
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(worker.drain().is_empty());
        assert!(data.idle_calls.load(Ordering::SeqCst) >= 2);
        data.locked.store(true, Ordering::SeqCst);
        assert_eq!(update(&worker).await.locked, Some(true));
    }

    #[tokio::test]
    async fn unsupported_idle_is_unknown_without_losing_kde_lock_and_timeouts_are_bounded() {
        let bus = PrivateBus::new();
        let data = Arc::new(Fixture::default());
        data.unsupported.store(true, Ordering::SeqCst);
        data.locked.store(true, Ordering::SeqCst);
        let _service = fixture(&bus, data.clone()).await;
        let client = connection(&bus).await;
        let state = sample(Some(&client), None, None).await;
        assert_eq!(state.idle_ms, None);
        assert_eq!(state.locked, Some(true));
        assert!(state.error.unwrap().contains("not supported"));
        data.unsupported.store(false, Ordering::SeqCst);
        data.delay_ms.store(5000, Ordering::SeqCst);
        let started = Instant::now();
        let state = sample(Some(&client), None, None).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(state.idle_ms, None);
        assert_eq!(state.locked, Some(true));
        assert!(state.error.is_some());
    }

    #[tokio::test]
    async fn mutter_idle_and_exact_logind_session_are_independent_backends() {
        let bus = PrivateBus::new();
        let _service = zbus::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name(MUTTER)
            .unwrap()
            .name(LOGIN)
            .unwrap()
            .serve_at("/org/gnome/Mutter/IdleMonitor/Core", Mutter)
            .unwrap()
            .serve_at("/org/freedesktop/login1", Login)
            .unwrap()
            .serve_at("/org/freedesktop/login1/session/fixture", LoginSession)
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = connection(&bus).await;
        let state = sample(Some(&client), Some(&client), Some("fixture")).await;
        assert_eq!(state.idle_ms, Some(4700));
        assert_eq!(state.locked, Some(true));
        assert!(state.source.contains("Mutter") && state.source.contains("LockedHint"));
        assert_eq!(state.error, None);
        let unavailable = sample(Some(&client), Some(&client), None).await;
        assert_eq!(unavailable.idle_ms, Some(4700));
        assert_eq!(unavailable.locked, None);
    }

    #[tokio::test]
    async fn missing_services_stay_unknown_and_drop_interrupts_a_pending_reply() {
        let bus = PrivateBus::new();
        let client = connection(&bus).await;
        let state = sample(Some(&client), Some(&client), None).await;
        assert_eq!((state.idle_ms, state.locked), (None, None));
        assert!(state.error.is_some());
        assert!(!present(&client, KDE).await.unwrap());
        assert!(!present(&client, MUTTER).await.unwrap());
        let data = Arc::new(Fixture::default());
        data.delay_ms.store(5000, Ordering::SeqCst);
        let _service = fixture(&bus, data.clone()).await;
        let worker = Worker::start(Some(bus.address.clone()), Some(bus.address.clone()), None);
        let deadline = Instant::now() + Duration::from_secs(2);
        while data.idle_calls.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    fn properties() -> HashMap<String, OwnedValue> {
        HashMap::from([
            ("Type".into(), Value::from("wayland").try_into().unwrap()),
            ("Id".into(), Value::from("fixture").try_into().unwrap()),
            (
                "User".into(),
                Value::from((
                    unsafe { libc::geteuid() },
                    ObjectPath::try_from("/user/test").unwrap(),
                ))
                .try_into()
                .unwrap(),
            ),
            ("LockedHint".into(), OwnedValue::from(false)),
        ])
    }
    #[test]
    fn session_identity_types_and_absent_fields_are_validated_without_defaults() {
        assert!(!validate_session(properties(), Some("fixture")).unwrap());
        assert!(validate_session(properties(), Some("different")).is_err());
        let mut p = properties();
        p.insert("Type".into(), Value::from("tty").try_into().unwrap());
        assert!(validate_session(p, None).is_err());
        let mut p = properties();
        p.remove("LockedHint");
        assert!(validate_session(p, None).is_err());
        let mut p = properties();
        p.insert("LockedHint".into(), OwnedValue::from(0u32));
        assert!(validate_session(p, None).is_err());
        let mut p = properties();
        p.insert(
            "User".into(),
            Value::from((
                unsafe { libc::geteuid() }.wrapping_add(1),
                ObjectPath::try_from("/user/other").unwrap(),
            ))
            .try_into()
            .unwrap(),
        );
        assert!(validate_session(p, None).is_err());
    }

    #[test]
    fn notification_buffer_is_bounded_and_keeps_unknown_distinct_from_zero_false() {
        let mut updates = Updates::default();
        let unknown = State::unknown("unavailable".into());
        updates.publish(unknown.clone());
        updates.publish(unknown);
        assert_eq!(updates.pending.len(), 1);
        for idle in 0..40 {
            updates.publish(State {
                idle_ms: Some(idle),
                locked: Some(false),
                source: "fixture".into(),
                error: None,
            });
        }
        assert_eq!(updates.pending.len(), 16);
        assert_eq!(updates.pending.back().unwrap().idle_ms, Some(39));
    }

    #[test]
    #[ignore = "explicit read-only observation of the current desktop"]
    fn read_only_current_desktop_sample() {
        let worker = Worker::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(state) = worker.drain().pop() {
                eprintln!(
                    "READ_ONLY_DESKTOP_ACTIVITY {}",
                    serde_json::to_string(&state).unwrap()
                );
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(20));
        }
    }
}
