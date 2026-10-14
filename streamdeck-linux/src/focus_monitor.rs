//! Read-only KDE focus subscription. No titles, window contents or activation.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::oneshot;

const PATH: &str = "/org/deckforge/FocusObserver";
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Focus {
    pub pid: u32,
    pub desktop_id: String,
    pub window_class: String,
    pub executable: Option<PathBuf>,
}
impl Focus {
    pub fn same_application(&self, other: &Self) -> bool {
        self.desktop_id == other.desktop_id
            && self.window_class == other.window_class
            && self.executable == other.executable
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub editor_minimized: Option<bool>,
    pub focus: Option<Focus>,
    pub recent: Vec<Focus>,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            editor_minimized: None,
            focus: None,
            recent: vec![],
            error: Some("Starting KDE focus observer".into()),
        }
    }
}
fn executable(pid: u32) -> Option<PathBuf> {
    let proc = PathBuf::from(format!("/proc/{pid}"));
    // SAFETY: geteuid has no arguments or side effects.
    if fs::metadata(&proc).ok()?.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let mut path = fs::read_link(proc.join("exe")).ok()?;
    // Linux annotates an unlinked running image after an app update. Only strip
    // that annotation when the mapped inode has no links; a real filename may
    // itself end in " (deleted)".
    if fs::metadata(proc.join("exe")).ok()?.nlink() == 0
        && let Some(original) = path.to_str()?.strip_suffix(" (deleted)")
    {
        path = PathBuf::from(original);
    }
    let text = path.to_str()?;
    if text.len() > 4096 || text.chars().any(char::is_control) {
        return None;
    }
    Some(path)
}
fn field(value: &str) -> bool {
    value.len() <= 1024 && !value.chars().any(char::is_control)
}

struct Receiver {
    nonce: String,
    latest: Arc<Mutex<Snapshot>>,
    ready: Mutex<Option<oneshot::Sender<()>>>,
}
#[zbus::interface(name = "org.deckforge.FocusObserver")]
impl Receiver {
    fn report(
        &self,
        nonce: &str,
        pid: &str,
        desktop_id: &str,
        window_class: &str,
        minimized: &str,
    ) -> bool {
        if nonce != self.nonce {
            return false;
        }
        if !field(desktop_id) || !field(window_class) {
            publish_error(
                &self.latest,
                "Focused application identifier is unsupported".into(),
            );
            return false;
        }
        let Ok(pid) = pid.parse::<u32>() else {
            publish_error(&self.latest, "Focused application PID is invalid".into());
            return false;
        };
        let editor_minimized = match minimized {
            "none" => None,
            "true" => Some(true),
            "false" => Some(false),
            _ => {
                publish_error(&self.latest, "Editor window state is invalid".into());
                return false;
            }
        };
        let focus = if pid == 0 && desktop_id.is_empty() && window_class.is_empty() {
            None
        } else {
            Some(Focus {
                pid,
                desktop_id: desktop_id.to_owned(),
                window_class: window_class.to_owned(),
                executable: if pid == 0 { None } else { executable(pid) },
            })
        };
        let mut latest = self.latest.lock().unwrap();
        if latest.focus != focus
            || latest.editor_minimized != editor_minimized
            || latest.error.is_some()
        {
            latest.revision = latest.revision.saturating_add(1);
            if let Some(focus) = &focus {
                latest.recent.retain(|old| !old.same_application(focus));
                latest.recent.insert(0, focus.clone());
                latest.recent.truncate(32);
            }
            latest.focus = focus;
            latest.editor_minimized = editor_minimized;
            latest.error = None;
        }
        if let Some(ready) = self.ready.lock().unwrap().take() {
            let _ = ready.send(());
        }
        true
    }
}

pub struct Worker {
    latest: Arc<Mutex<Snapshot>>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    name: String,
}
impl Worker {
    pub fn new() -> Self {
        Self::start(format!("deckforge-focus-{}", uuid::Uuid::new_v4()), None)
    }
    /// Caller must own the state's session lock. A stable, private scope name
    /// lets the next owner remove a script left by an abruptly terminated host.
    pub fn for_state(state: &Path) -> Result<Self> {
        use sha2::{Digest, Sha256};
        use std::os::unix::ffi::OsStrExt;
        let canonical = state.canonicalize()?;
        Ok(Self::start(
            format!(
                "deckforge-focus-{:x}",
                Sha256::digest(canonical.as_os_str().as_bytes())
            ),
            Some(canonical),
        ))
    }
    fn start(name: String, source_dir: Option<PathBuf>) -> Self {
        let latest = Arc::new(Mutex::new(Snapshot::default()));
        let output = latest.clone();
        let (stop, mut stopped) = oneshot::channel();
        let script_name = name.clone();
        let thread = thread::Builder::new()
            .name("deckforge-focus".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else {
                    publish_error(&output, "Could not start focus observer runtime".into());
                    return;
                };
                runtime.block_on(async {
                    loop {
                        match observe(
                            output.clone(),
                            &script_name,
                            source_dir.as_deref(),
                            &mut stopped,
                        )
                        .await
                        {
                            Ok(()) => break,
                            Err(error) => {
                                publish_error(&output, format!("KDE focus observer: {error:#}"))
                            }
                        }
                        tokio::select! {_=&mut stopped=>break,_=tokio::time::sleep(TIMEOUT)=>{}}
                    }
                });
            });
        let thread = match thread {
            Ok(t) => Some(t),
            Err(error) => {
                publish_error(&latest, format!("Focus observer thread: {error}"));
                None
            }
        };
        Self {
            latest,
            stop: Some(stop),
            thread,
            name,
        }
    }
    pub fn request_stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut value = self.latest.lock().unwrap().clone();
        if value.error.is_none() && self.thread.as_ref().is_none_or(JoinHandle::is_finished) {
            value.error = Some("Focus observer stopped unexpectedly".into());
        }
        value
    }
    pub fn script_name(&self) -> &str {
        &self.name
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
fn publish_error(latest: &Mutex<Snapshot>, error: String) {
    let mut state = latest.lock().unwrap();
    state.revision = state.revision.saturating_add(1);
    state.error = Some(error);
}
struct ScriptSource {
    path: PathBuf,
    temporary: Option<tempfile::NamedTempFile>,
}
impl Drop for ScriptSource {
    fn drop(&mut self) {
        if self.temporary.is_none() {
            let _ = fs::remove_file(&self.path);
        }
    }
}
async fn observe(
    latest: Arc<Mutex<Snapshot>>,
    name: &str,
    source_dir: Option<&Path>,
    stop: &mut oneshot::Receiver<()>,
) -> Result<()> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let (ready, received) = oneshot::channel();
    let connection = tokio::time::timeout(
        TIMEOUT,
        zbus::connection::Builder::session()?
            .method_timeout(TIMEOUT)
            .serve_at(
                PATH,
                Receiver {
                    nonce: nonce.clone(),
                    latest,
                    ready: Mutex::new(Some(ready)),
                },
            )?
            .build(),
    )
    .await
    .context("Session bus connection timed out")??;
    let service = connection
        .unique_name()
        .context("Session bus has no unique name")?;
    let bus = zbus::Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?;
    let owner: String = bus
        .call("GetNameOwner", &("org.kde.KWin",))
        .await
        .context("KWin is unavailable on this desktop")?;
    let scripting = zbus::Proxy::new(
        &connection,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )
    .await?;
    if source_dir.is_some() {
        let _: bool = scripting.call("unloadScript", &(name,)).await?;
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix("deckforge-focus-").suffix(".js");
    let mut source = if let Some(dir) = source_dir {
        builder.tempfile_in(dir)?
    } else {
        builder.tempfile()?
    };
    write!(
        source,
        "function report() {{ const w=workspace.activeWindow; let own=0,shown=0; for(const v of workspace.windowList()){{if(v&&v.pid==={}&&v.normalWindow){{own++;if(!v.minimized)shown++;}}}} callDBus({}, {}, 'org.deckforge.FocusObserver', 'Report', {}, String(w?w.pid:0), String(w?w.desktopFileName:''), String(w?w.resourceClass:''), own===0?'none':shown===0?'true':'false'); }} function watch(w) {{ if(!w)return; if(w.desktopFileNameChanged)w.desktopFileNameChanged.connect(report); if(w.windowClassChanged)w.windowClassChanged.connect(report); if(w.minimizedChanged)w.minimizedChanged.connect(report); }} workspace.windowActivated.connect(report); workspace.windowAdded.connect(watch); workspace.windowRemoved.connect(report); for(const w of workspace.windowList())watch(w); report();",
        std::process::id(),
        serde_json::to_string(service.as_str())?,
        serde_json::to_string(PATH)?,
        serde_json::to_string(&nonce)?
    )?;
    let source = if let Some(dir) = source_dir {
        let path = dir.join(".smart-profile-focus.js");
        source.persist(&path).map_err(|error| error.error)?;
        ScriptSource {
            path,
            temporary: None,
        }
    } else {
        ScriptSource {
            path: source.path().to_owned(),
            temporary: Some(source),
        }
    };
    let result=async {
    let id: i32 = scripting
        .call(
            "loadScript",
            &(
                source.path.to_str()
                    .context("Temporary path is not UTF-8")?,
                name,
            ),
        )
        .await?;
    ensure!(id >= 0, "KWin could not load the focus observer");
        let script=zbus::Proxy::new(&connection,"org.kde.KWin",format!("/Scripting/Script{id}"),"org.kde.kwin.Script").await?;
        script.call::<_,_,()>("run",&()).await?;
        tokio::select!{
            _=&mut *stop=>return Ok(()),
            ready=tokio::time::timeout(TIMEOUT,received)=>ready.context("KWin did not report its foreground window")??,
        }
        loop {
            tokio::select!{_=&mut *stop=>return Ok(()),_=tokio::time::sleep(TIMEOUT)=>{}}
            let current:String=bus.call("GetNameOwner",&("org.kde.KWin",)).await?;
            ensure!(current==owner,"KWin restarted; reconnecting");
        }
    }.await;
    let _: Result<bool, _> = scripting.call("unloadScript", &(name,)).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_identity_survives_unlink_without_trimming_a_real_filename_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe (deleted)");
        fs::copy("/usr/bin/sleep", &path).unwrap();
        let mut child = std::process::Command::new(&path).arg("2").spawn().unwrap();
        let before = executable(child.id());
        fs::remove_file(&path).unwrap();
        let after = executable(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(before, Some(path.clone()));
        assert_eq!(after, Some(path));
    }
    #[test]
    fn reports_require_nonce_and_bounded_identifiers_and_keep_recent_apps_unique() {
        let latest = Arc::new(Mutex::new(Snapshot::default()));
        let receiver = Receiver {
            nonce: "owned".into(),
            latest: latest.clone(),
            ready: Mutex::new(None),
        };
        assert!(!receiver.report("other", "1", "app", "class", "none"));
        assert!(!receiver.report("owned", "NaN", "app", "class", "none"));
        assert!(!receiver.report("owned", "0", &"a".repeat(1025), "class", "none"));
        let errors = latest.lock().unwrap().revision;
        assert!(receiver.report("owned", "0", "app", "class", "none"));
        assert!(receiver.report("owned", "0", "app", "class", "none"));
        assert_eq!(latest.lock().unwrap().revision, errors + 1);
        assert_eq!(latest.lock().unwrap().recent.len(), 1);
        assert!(receiver.report("owned", "0", "", "", "none"));
        assert!(latest.lock().unwrap().focus.is_none());
        for i in 0..40 {
            assert!(receiver.report("owned", "0", &format!("app{i}"), "class", "none"));
        }
        assert_eq!(latest.lock().unwrap().recent.len(), 32);
    }
}
