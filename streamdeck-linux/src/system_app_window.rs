//! Desktop window activation/close using KDE scripting or local X11. Only windows
//! owned by the selected executable's current-user processes can be affected.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Write, sync::Mutex, time::Duration};
use tokio::sync::oneshot;
const PATH: &str = "/org/deckforge/WindowRequest";
struct Completion {
    nonce: String,
    result: Mutex<Option<oneshot::Sender<String>>>,
}
#[zbus::interface(name = "org.deckforge.WindowRequest")]
impl Completion {
    fn complete(&self, nonce: &str, count: &str) {
        if nonce == self.nonce
            && count.len() <= 1024 * 1024
            && let Some(sender) = self.result.lock().unwrap().take()
        {
            let _ = sender.send(count.to_owned());
        }
    }
}
pub fn activate(processes: &[crate::system_process::PinnedProcess]) -> Result<u32> {
    ensure!(
        processes.len() <= 4096,
        "Too many matching application processes"
    );
    if processes.is_empty() {
        return Ok(0);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some() {
        return crate::system_x11_window::activate(processes);
    }
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let (_,targets)=kde_windows(processes).await?;
        if targets.is_empty(){return Ok(0);}
        let script=format!("const targets={}; let result=0; for(const w of workspace.windowList()) {{if(targets.some(t=>t.pid===w.pid&&t.id===w.internalId.toString())&&(w.normalWindow||w.dialog)){{w.minimized=false;workspace.activeWindow=w;result=1;break;}}}}",serde_json::to_string(&targets)?);
        run_script(&script).await?.parse().context("Invalid activation reply")
    })
}

#[derive(Clone, Deserialize, Serialize)]
pub struct WindowTarget {
    pub id: String,
    pub pid: u32,
    eligible: bool,
}
pub struct CloseResult {
    pub found: usize,
    pub requested: u32,
}

pub fn close_process_windows(
    processes: &[crate::system_process::PinnedProcess],
) -> Result<CloseResult> {
    ensure!(
        processes.len() <= 4096,
        "Too many matching application processes"
    );
    if processes.is_empty() {
        return Ok(CloseResult {
            found: 0,
            requested: 0,
        });
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some() {
        return crate::system_x11_window::close(processes);
    }
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let (found,targets)=kde_windows(processes).await?;
        if targets.is_empty(){return Ok(CloseResult{found,requested:0});}
        let script=format!("const targets={}; let result=0; for(const w of workspace.windowList()) {{if(targets.some(t=>t.pid===w.pid&&t.id===w.internalId.toString())&&(w.normalWindow||w.dialog)){{w.closeWindow();result++;}}}}",serde_json::to_string(&targets)?);
        let requested=run_script(&script).await?.parse().context("Invalid close reply")?;
        Ok(CloseResult{found,requested})
    })
}
async fn kde_windows(
    processes: &[crate::system_process::PinnedProcess],
) -> Result<(usize, Vec<WindowTarget>)> {
    let pids: Vec<_> = processes.iter().map(|p| p.pid).collect();
    let query = format!(
        "const pids={};let result=[];for(const w of workspace.windowList()){{if(pids.includes(w.pid))result.push({{id:w.internalId.toString(),pid:w.pid,eligible:w.normalWindow||w.dialog}});}}result=JSON.stringify(result);",
        serde_json::to_string(&pids)?
    );
    let windows: Vec<WindowTarget> = serde_json::from_str(&run_script(&query).await?)
        .context("Invalid desktop window inventory")?;
    ensure!(windows.len() <= 4096, "Too many matching windows");
    let found = windows.len();
    for window in &windows {
        ensure!(
            uuid::Uuid::parse_str(&window.id).is_ok(),
            "Desktop window has no stable identity"
        );
    }
    let targets = windows
        .into_iter()
        .filter(|w| w.eligible && processes.iter().any(|p| p.pid == w.pid && p.matches_now()))
        .collect();
    Ok((found, targets))
}

async fn run_script(operation: &str) -> Result<String> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = oneshot::channel();
    let connection = tokio::time::timeout(
        Duration::from_secs(2),
        zbus::connection::Builder::session()?
            .method_timeout(Duration::from_secs(2))
            .serve_at(
                PATH,
                Completion {
                    nonce: nonce.clone(),
                    result: Mutex::new(Some(sender)),
                },
            )?
            .build(),
    )
    .await
    .context("Desktop session bus timed out")??;
    let service = connection
        .unique_name()
        .context("Session bus has no unique name")?;
    let scripting = zbus::Proxy::new(
        &connection,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )
    .await?;
    let mut file = tempfile::Builder::new()
        .prefix("deckforge-window-")
        .suffix(".js")
        .tempfile()?;
    write!(
        file,
        "{operation} callDBus({}, {}, 'org.deckforge.WindowRequest', 'Complete', {}, String(result));",
        serde_json::to_string(service.as_str())?,
        serde_json::to_string(PATH)?,
        serde_json::to_string(&nonce)?
    )?;
    let name = format!("deckforge-window-{nonce}");
    let id: i32 = scripting
        .call(
            "loadScript",
            &(
                file.path()
                    .to_str()
                    .context("Temporary path is not UTF-8")?,
                &name,
            ),
        )
        .await
        .context("Window control requires KDE KWin scripting on this desktop")?;
    ensure!(
        id >= 0,
        "KWin could not load the application window request"
    );
    let result = async {
        let path = format!("/Scripting/Script{id}");
        let script =
            zbus::Proxy::new(&connection, "org.kde.KWin", path, "org.kde.kwin.Script").await?;
        script.call::<_, _, ()>("run", &()).await?;
        Ok(tokio::time::timeout(Duration::from_secs(2), receiver)
            .await
            .context("Desktop window request did not respond")??)
    }
    .await;
    let _: Result<bool, _> = scripting.call("unloadScript", &(&name,)).await;
    result
}
