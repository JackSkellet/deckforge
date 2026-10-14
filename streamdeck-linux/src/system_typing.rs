//! Clipboard-free text through the desktop's keyboard-permission portal.
//! No screens/pointer access, implicit permission requests, or persistent tokens.
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use tokio::{
    sync::{Notify, mpsc as async_mpsc},
    time::timeout,
};
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Clone, Serialize)]
pub struct Status {
    pub phase: String,
    pub message: String,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "disabled".into(),
            message: "Enable desktop typing for this app session".into(),
        }
    }
}
#[derive(Default)]
struct Inner {
    status: Status,
    generation: u64,
    shutdown: bool,
    typing: Option<u64>,
}
#[derive(Clone, Default)]
pub struct Shared {
    inner: Arc<Mutex<Inner>>,
    wake: Arc<Notify>,
}
impl Shared {
    pub fn is_typing(&self) -> bool {
        self.inner.lock().unwrap().typing.is_some()
    }
    fn finished(&self, generation: u64) {
        let mut s = self.inner.lock().unwrap();
        if s.typing == Some(generation) {
            s.typing = None;
            if s.status.phase == "stopping" {
                s.status = Status::default();
            }
        }
    }
    pub fn snapshot(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }
    fn current(&self, generation: u64) -> bool {
        let s = self.inner.lock().unwrap();
        s.generation == generation && !s.shutdown
    }
    fn set(&self, generation: u64, phase: &str, message: impl Into<String>) {
        let mut s = self.inner.lock().unwrap();
        if s.generation == generation && !s.shutdown {
            s.status = Status {
                phase: phase.into(),
                message: message.into(),
            };
        }
    }
    pub fn cancel(&self) {
        let mut s = self.inner.lock().unwrap();
        s.generation = s.generation.wrapping_add(1);
        s.status = if s.typing.is_some() {
            Status {
                phase: "stopping".into(),
                message: "Stopping typing".into(),
            }
        } else {
            Status::default()
        };
        drop(s);
        self.wake.notify_one();
    }
    async fn cancelled(&self, generation: u64) {
        loop {
            let notified = self.wake.notified();
            if !self.current(generation) {
                return;
            }
            notified.await;
        }
    }
}

pub fn keysyms(text: &str, enter: bool) -> Result<Vec<i32>> {
    ensure!(
        text.len() <= 1024 * 1024 && !text.contains('\0'),
        "Text must be at most 1 MiB without NUL characters"
    );
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut result = Vec::with_capacity(normalized.chars().count() + usize::from(enter));
    for c in normalized.chars() {
        result.push(match c {
            '\n' => 0xff0d,
            '\t' => 0xff09,
            c if c.is_control() => bail!(
                "Typing does not support control character U+{:04X}",
                c as u32
            ),
            c if (c as u32) <= 0xff => c as i32,
            c => (0x01000000 | c as u32) as i32,
        });
    }
    if enter {
        result.push(0xff0d);
    }
    ensure!(!result.is_empty(), "Enter text or enable Send Enter");
    Ok(result)
}

struct Portal {
    connection: Connection,
    proxy: Proxy<'static>,
    session: OwnedObjectPath,
    pending: Option<OwnedObjectPath>,
    closed: Option<MessageStream>,
}
impl Portal {
    async fn new() -> Result<Self> {
        let connection = Connection::session().await?;
        let sender = connection
            .unique_name()
            .context("Portal connection has no unique name")?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let token = format!("deckforge_{}", uuid::Uuid::new_v4().simple());
        let session = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/session/{sender}/{token}"
        ))?;
        let proxy = Proxy::new_owned(
            connection.clone(),
            SERVICE,
            PATH,
            "org.freedesktop.portal.RemoteDesktop",
        )
        .await?;
        Ok(Self {
            connection,
            proxy,
            session,
            pending: None,
            closed: None,
        })
    }
    async fn request<B: Serialize + zbus::zvariant::DynamicType>(
        &mut self,
        method: &str,
        body: &B,
    ) -> Result<HashMap<String, OwnedValue>> {
        let rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)?
            .interface("org.freedesktop.portal.Request")?
            .member("Response")?
            .path_namespace(format!(
                "{PATH}/request/{}",
                self.connection
                    .unique_name()
                    .unwrap()
                    .as_str()
                    .trim_start_matches(':')
                    .replace('.', "_")
            ))?
            .build();
        let mut responses = MessageStream::for_match_rule(rule, &self.connection, Some(32)).await?;
        let path: OwnedObjectPath = self.proxy.call(method, body).await?;
        self.pending = Some(path.clone());
        while let Some(message) = responses.next().await {
            let message = message?;
            if message
                .header()
                .path()
                .is_some_and(|p| p.as_str() == path.as_str())
            {
                let (code, data): (u32, HashMap<String, OwnedValue>) =
                    message.body().deserialize()?;
                self.pending = None;
                ensure!(
                    code == 0,
                    "Desktop keyboard permission was cancelled or denied"
                );
                return Ok(data);
            }
        }
        bail!("Desktop portal connection closed")
    }
    async fn prepare(&mut self) -> Result<()> {
        let token = self.session.as_str().rsplit('/').next().unwrap().to_owned();
        let mut options = HashMap::new();
        options.insert("session_handle_token", Value::from(token));
        options.insert(
            "handle_token",
            Value::from(format!("deckforge_{}", uuid::Uuid::new_v4().simple())),
        );
        let mut response = self.request("CreateSession", &options).await?;
        let handle = response
            .remove("session_handle")
            .context("Portal returned no session")?;
        self.session = match &*handle {
            Value::Str(path) => OwnedObjectPath::try_from(path.as_str().to_owned())?,
            Value::ObjectPath(path) => OwnedObjectPath::try_from(path.as_str().to_owned())?,
            _ => bail!("Portal returned an invalid session handle type"),
        };
        let rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)?
            .path(self.session.clone())?
            .interface("org.freedesktop.portal.Session")?
            .member("Closed")?
            .build();
        self.closed = Some(MessageStream::for_match_rule(rule, &self.connection, Some(4)).await?);
        let options = HashMap::from([
            ("types", Value::from(1u32)),
            ("persist_mode", Value::from(0u32)),
        ]);
        let session = self.session.clone();
        self.request("SelectDevices", &(session, &options)).await?;
        let options = HashMap::<&str, Value>::new();
        let session = self.session.clone();
        let mut selected = self.request("Start", &(session, "", &options)).await?;
        let devices = u32::try_from(
            selected
                .remove("devices")
                .context("Portal did not grant a keyboard")?,
        )?;
        ensure!(devices & 1 != 0, "Desktop did not grant keyboard input");
        Ok(())
    }
    async fn key(&self, keysym: i32, state: u32) -> Result<()> {
        let options = HashMap::<&str, Value>::new();
        timeout(
            Duration::from_secs(2),
            self.proxy.call::<_, _, ()>(
                "NotifyKeyboardKeysym",
                &(&self.session, &options, keysym, state),
            ),
        )
        .await
        .context("Desktop keyboard request timed out")??;
        Ok(())
    }
    async fn send(&self, keys: &[i32], shared: &Shared, generation: u64) -> Result<()> {
        for &key in keys {
            ensure!(shared.current(generation), "Typing cancelled");
            let pressed = self.key(key, 1).await;
            // Always attempt release, including an uncertain or failed press.
            let released = self.key(key, 0).await;
            pressed?;
            released?;
            // Portal replies acknowledge submission, not delivery to the client.
            // Pace input so Stop cannot leave a large compositor/client backlog.
            tokio::select! {
                _=tokio::time::sleep(Duration::from_millis(5))=>{},
                _=shared.cancelled(generation)=>bail!("Typing cancelled"),
            }
        }
        ensure!(shared.current(generation), "Typing cancelled");
        Ok(())
    }
    async fn close(self) {
        if let Some(path) = self.pending {
            let _ = timeout(
                Duration::from_secs(1),
                self.connection.call_method(
                    Some(SERVICE),
                    path,
                    Some("org.freedesktop.portal.Request"),
                    "Close",
                    &(),
                ),
            )
            .await;
        }
        let _ = timeout(
            Duration::from_secs(1),
            self.connection.call_method(
                Some(SERVICE),
                self.session,
                Some("org.freedesktop.portal.Session"),
                "Close",
                &(),
            ),
        )
        .await;
        let _ = self.connection.close().await;
    }
}
enum Kind {
    Prepare,
    Type(Vec<i32>),
}
struct Job {
    generation: u64,
    context: String,
    kind: Kind,
}
pub struct Worker {
    sender: async_mpsc::Sender<Job>,
    shared: Shared,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(shared: Shared, reports: mpsc::Sender<crate::system_actions::Report>) -> Self {
        let (sender, mut receiver) = async_mpsc::channel::<Job>(8);
        let state = shared.clone();
        let thread = thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    state.set(0, "error", e.to_string());
                    return;
                }
            };
            runtime.block_on(async {
                let mut portal:Option<(u64,Portal)>=None;
                loop {
                    if state.inner.lock().unwrap().shutdown {break;}
                    tokio::select! {
                        _=state.wake.notified()=>{
                            if portal.as_ref().is_some_and(|(g,_)|!state.current(*g)) && let Some((_,p))=portal.take(){p.close().await;}
                            if state.inner.lock().unwrap().shutdown {break;}
                        }
                        _=async {if let Some((_,p))=&mut portal {p.closed.as_mut().unwrap().next().await;} else {std::future::pending::<()>().await;}}=>{
                            if let Some((generation,p))=portal.take(){state.set(generation,"disabled","Desktop keyboard permission ended; enable typing again");p.close().await;}
                        }
                        job=receiver.recv()=>{
                            let Some(job)=job else {break;};
                            if !state.current(job.generation){state.finished(job.generation);continue;}
                            let result=match job.kind {
                                Kind::Prepare=>{
                                    if let Some((_,p))=portal.take(){p.close().await;}
                                    let connected=tokio::select! {r=timeout(Duration::from_secs(5),Portal::new())=>r.context("Desktop portal did not respond").and_then(|r|r),_=state.cancelled(job.generation)=>Err(anyhow::anyhow!("Typing setup cancelled"))};
                                    match connected {
                                        Ok(mut p)=>{
                                            let result=tokio::select!{r=timeout(Duration::from_secs(120),p.prepare())=>r.context("Desktop keyboard permission request timed out").and_then(|r|r),_=state.cancelled(job.generation)=>Err(anyhow::anyhow!("Typing setup cancelled"))};
                                            if result.is_ok()&&state.current(job.generation){portal=Some((job.generation,p));state.set(job.generation,"ready","Desktop typing enabled for this app session");Ok("Desktop typing enabled".into())}
                                            else {p.close().await;result.map(|()|"Typing setup cancelled".into())}
                                        }
                                        Err(e)=>Err(e),
                                    }
                                }
                                Kind::Type(keys)=>{
                                    if let Some((_,p))=&portal {
                                        let result=p.send(&keys,&state,job.generation).await;
                                        if result.is_ok(){state.set(job.generation,"ready","Desktop typing enabled for this app session");}
                                        else if let Some((_,p))=portal.take(){p.close().await;}
                                        result.map(|()|"Text sent through desktop keyboard input".into())
                                    } else {Err(anyhow::anyhow!("Enable desktop typing in Text settings first"))}
                                }
                            };
                            state.finished(job.generation);
                            if let Err(error)=&result{state.set(job.generation,"error",format!("{error:#}"));}
                            if !state.inner.lock().unwrap().shutdown {let _=reports.send(crate::system_actions::Report{context:job.context,result,prepared_keyboard:false});}
                        }
                    }
                }
                if let Some((_,p))=portal.take(){p.close().await;}
            });
        });
        Self {
            sender,
            shared,
            thread: Some(thread),
        }
    }
    pub fn prepare(&self) -> Result<()> {
        self.submit("", Kind::Prepare)
    }
    pub fn type_text(&self, context: &str, text: &str, enter: bool) -> Result<()> {
        self.submit(context, Kind::Type(keysyms(text, enter)?))
    }
    fn submit(&self, context: &str, kind: Kind) -> Result<()> {
        let mut state = self.shared.inner.lock().unwrap();
        ensure!(!state.shutdown, "Desktop typing stopped");
        let (phase, message) = match &kind {
            Kind::Prepare => {
                ensure!(
                    !matches!(
                        state.status.phase.as_str(),
                        "requesting" | "typing" | "stopping"
                    ),
                    "Desktop typing is busy"
                );
                (
                    "requesting",
                    "Allow keyboard input in the desktop permission dialog",
                )
            }
            Kind::Type(_) => {
                ensure!(
                    state.status.phase == "ready" && state.typing.is_none(),
                    "Enable desktop typing in Text settings first, or wait for the current text to finish"
                );
                ("typing", "Sending text to the focused application")
            }
        };
        let typing = matches!(kind, Kind::Type(_));
        self.sender
            .try_send(Job {
                generation: state.generation,
                context: context.into(),
                kind,
            })
            .context("Desktop typing queue is unavailable")?;
        if typing {
            state.typing = Some(state.generation);
        }
        state.status = Status {
            phase: phase.into(),
            message: message.into(),
        };
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.cancel();
        self.shared.inner.lock().unwrap().shutdown = true;
        self.shared.wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires the owned private-compositor portal probe"]
    fn portal_shortcut_probe_on_private_wayland() -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("DECKFORGE_PRIVATE_CLIPBOARD_TEST")?);
        ensure!(
            root.parent() == Some(std::path::Path::new("/tmp"))
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("deckforge-focused-"),
            "Private fixture required"
        );
        ensure!(
            std::env::var("XDG_RUNTIME_DIR")? == root.join("runtime").to_string_lossy(),
            "Private runtime required"
        );
        let events: Vec<(String, i32, u32)> =
            serde_json::from_slice(&std::fs::read(root.join("portal-request.json"))?)?;
        ensure!(
            events.len() <= 100
                && events.iter().all(|(kind, _, state)| matches!(
                    kind.as_str(),
                    "keysym" | "keycode" | "layout"
                ) && *state <= 1),
            "Invalid probe events"
        );
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let mut portal = Portal::new().await?;
                let result = async {
                    timeout(Duration::from_secs(5), portal.prepare()).await??;
                    for (kind, key, state) in events {
                        if kind == "layout" {
                            let layout = zbus::Proxy::new(
                                &portal.connection,
                                "org.kde.keyboard",
                                "/Layouts",
                                "org.kde.KeyboardLayouts",
                            )
                            .await?;
                            let changed: bool = layout.call("setLayout", &(key as u32)).await?;
                            ensure!(changed, "Private test layout switch failed");
                        } else if kind == "keysym" {
                            portal.key(key, state).await?;
                        } else {
                            let options = HashMap::<&str, Value>::new();
                            timeout(
                                Duration::from_secs(2),
                                portal.proxy.call::<_, _, ()>(
                                    "NotifyKeyboardKeycode",
                                    &(&portal.session, &options, key, state),
                                ),
                            )
                            .await??;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                tokio::time::sleep(Duration::from_millis(150)).await;
                std::fs::copy(
                    root.join("events.json"),
                    root.join("portal-before-close.json"),
                )?;
                portal.close().await;
                result
            })
    }
    #[test]
    fn unicode_keysyms_and_original_newline_normalization() {
        assert_eq!(
            keysyms("Aæ🌍\r\nB\rC\t", true).unwrap(),
            vec![65, 230, 0x0101f30d, 0xff0d, 66, 0xff0d, 67, 0xff09, 0xff0d]
        );
        assert!(keysyms("", false).is_err());
        assert_eq!(keysyms("", true).unwrap(), vec![0xff0d]);
        assert!(keysyms("safe\u{1}unsafe", false).is_err());
        assert!(keysyms("\0", false).is_err());
    }
}
