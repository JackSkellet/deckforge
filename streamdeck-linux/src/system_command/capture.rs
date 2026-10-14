//! Bounded nonblocking process capture on the existing System action worker.
use super::{Config, Output};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::VecDeque,
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const STDOUT_LIMIT: usize = 1024 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;
const PREVIEW_LIMIT: usize = 16 * 1024;
const MAX_RECORDS: usize = 16;
const MAX_RUNNING: usize = 8;

#[derive(Clone, Serialize)]
pub struct Summary {
    pub id: String,
    pub state: String,
    pub message: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub truncated: bool,
    pub can_copy: bool,
}

#[derive(Serialize)]
pub struct Preview {
    pub summary: Summary,
    pub stdout: String,
    pub stderr: String,
    pub preview_truncated: bool,
}

struct Record {
    context: String,
    summary: Summary,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
#[derive(Clone, Default)]
pub struct Records(Arc<Mutex<VecDeque<Record>>>);
impl Records {
    pub fn forget(&self, context: &str) {
        self.0.lock().unwrap().retain(|r| r.context != context);
    }
    pub fn summary(&self, context: &str) -> Option<Summary> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.context == context)
            .map(|r| r.summary.clone())
    }
    pub fn preview(&self, context: &str, id: &str) -> Result<Preview> {
        let records = self.0.lock().unwrap();
        let r = records
            .iter()
            .find(|r| r.context == context && r.summary.id == id)
            .context("Command result has been replaced or expired")?;
        ensure!(r.summary.state != "running", "Command is still running");
        // A split UTF-8 sequence may be displayed lossily only at a preview boundary.
        Ok(Preview {
            summary: r.summary.clone(),
            stdout: String::from_utf8_lossy(&r.stdout[..r.stdout.len().min(PREVIEW_LIMIT)])
                .into_owned(),
            stderr: String::from_utf8_lossy(&r.stderr[..r.stderr.len().min(PREVIEW_LIMIT)])
                .into_owned(),
            preview_truncated: r.stdout.len() > PREVIEW_LIMIT || r.stderr.len() > PREVIEW_LIMIT,
        })
    }
    pub fn copy_text(&self, context: &str, id: &str) -> Result<String> {
        let records = self.0.lock().unwrap();
        let r = records
            .iter()
            .find(|r| r.context == context && r.summary.id == id)
            .context("Command result has been replaced or expired")?;
        ensure!(
            r.summary.can_copy,
            "Only complete successful UTF-8 output can be copied"
        );
        Ok(String::from_utf8(r.stdout.clone())?)
    }
}

struct Stream<R> {
    reader: Option<R>,
    data: Vec<u8>,
    count: u64,
    limit: usize,
}
impl<R: Read + AsRawFd> Stream<R> {
    fn new(reader: R, limit: usize) -> Result<Self> {
        let fd = reader.as_raw_fd();
        // These are owned child pipe descriptors; O_NONBLOCK does not affect its writer.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
            "Could not configure command output pipe"
        );
        Ok(Self {
            reader: Some(reader),
            data: Vec::new(),
            count: 0,
            limit,
        })
    }
    fn drain(&mut self) -> Result<()> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(());
        };
        let mut buffer = [0u8; 8192];
        for _ in 0..8 {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    self.reader = None;
                    break;
                }
                Ok(n) => {
                    self.count = self.count.saturating_add(n as u64);
                    let keep = n.min(self.limit.saturating_sub(self.data.len()));
                    self.data.extend_from_slice(&buffer[..keep]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    fn truncated(&self) -> bool {
        self.count > self.data.len() as u64
    }
}
struct Running {
    context: String,
    summary: Summary,
    child: Child,
    stdout: Stream<std::process::ChildStdout>,
    stderr: Stream<std::process::ChildStderr>,
    deadline: Instant,
    status: Option<ExitStatus>,
    exited: Option<Instant>,
    stopped: Option<&'static str>,
    failure: Option<String>,
    copy: bool,
}
fn kill_group(child: &Child) {
    // Called only before reaping the direct child, so its process-group ID cannot
    // have been reused. This never targets the host or its terminal session.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
}
pub struct Manager {
    running: Vec<Running>,
    pub records: Records,
}
impl Manager {
    pub fn new(records: Records) -> Self {
        Self {
            running: Vec::new(),
            records,
        }
    }
    pub fn len(&self) -> usize {
        self.running.len()
    }
    pub fn is_running(&self, context: &str) -> bool {
        self.running.iter().any(|run| run.context == context)
    }
    pub fn start(&mut self, context: &str, config: Config) -> Result<()> {
        ensure!(
            self.running.len() < MAX_RUNNING,
            "Too many captured commands are running"
        );
        ensure!(
            !self.running.iter().any(|r| r.context == context),
            "This command is already running; stop it before starting another"
        );
        let mut child = config
            .command()
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not run command")?;
        let streams = (|| {
            Ok::<_, anyhow::Error>((
                Stream::new(
                    child.stdout.take().context("Missing stdout pipe")?,
                    STDOUT_LIMIT,
                )?,
                Stream::new(
                    child.stderr.take().context("Missing stderr pipe")?,
                    STDERR_LIMIT,
                )?,
            ))
        })();
        let (stdout, stderr) = match streams {
            Ok(streams) => streams,
            Err(e) => {
                kill_group(&child);
                let _ = child.wait();
                return Err(e);
            }
        };
        let summary = Summary {
            id: uuid::Uuid::new_v4().to_string(),
            state: "running".into(),
            message: "Command running".into(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            truncated: false,
            can_copy: false,
        };
        let mut records = self.records.0.lock().unwrap();
        records.retain(|r| r.context != context);
        while records.len() >= MAX_RECORDS {
            let i = records
                .iter()
                .position(|r| r.summary.state != "running")
                .expect("Fewer running commands than retained results");
            records.remove(i);
        }
        records.push_back(Record {
            context: context.into(),
            summary: summary.clone(),
            stdout: vec![],
            stderr: vec![],
        });
        self.running.push(Running {
            context: context.into(),
            summary,
            child,
            stdout,
            stderr,
            deadline: Instant::now() + Duration::from_secs(config.timeout_seconds),
            status: None,
            exited: None,
            stopped: None,
            failure: None,
            copy: config.output == Output::Clipboard,
        });
        Ok(())
    }
    pub fn cancel(&mut self, context: &str, id: &str) -> Result<()> {
        let run = self
            .running
            .iter_mut()
            .find(|r| r.context == context && r.summary.id == id)
            .context("Command is no longer running")?;
        if run.status.is_none() && run.exited.is_none() {
            kill_group(&run.child);
        }
        run.stopped = Some("cancelled");
        Ok(())
    }
    pub fn poll(
        &mut self,
        clipboard: &mut Option<arboard::Clipboard>,
    ) -> Vec<(String, Result<String>)> {
        let mut completed = Vec::new();
        let mut index = 0;
        while index < self.running.len() {
            let run = &mut self.running[index];
            let now = Instant::now();
            if run.stopped.is_none()
                && run.status.is_none()
                && run.exited.is_none()
                && now >= run.deadline
            {
                kill_group(&run.child);
                run.stopped = Some("timed_out");
            }
            if let Err(e) = run.stdout.drain().and_then(|()| run.stderr.drain()) {
                run.failure = Some(format!("Could not read command output: {e}"));
                if run.status.is_none() && run.exited.is_none() {
                    kill_group(&run.child);
                }
            }
            if run.status.is_none() && run.exited.is_none() {
                match run.child.try_wait() {
                    Ok(Some(status)) => {
                        run.status = Some(status);
                        run.exited = Some(now);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        run.failure = Some(format!("Could not wait for command: {e}"));
                        run.exited = Some(now);
                    }
                }
            }
            run.summary.stdout_bytes = run.stdout.count;
            run.summary.stderr_bytes = run.stderr.count;
            run.summary.truncated = run.stdout.truncated() || run.stderr.truncated();
            let eof = run.stdout.reader.is_none() && run.stderr.reader.is_none();
            let finished = run.exited.is_some()
                && (eof
                    || run
                        .exited
                        .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(500)));
            if !finished {
                if let Some(record) = self
                    .records
                    .0
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r.summary.id == run.summary.id)
                {
                    record.summary = run.summary.clone();
                }
                index += 1;
                continue;
            }
            let mut run = self.running.swap_remove(index);
            let success = run.status.is_some_and(|s| s.success())
                && run.stopped.is_none()
                && run.failure.is_none()
                && eof;
            run.summary.state = if let Some(stopped) = run.stopped {
                stopped.into()
            } else if success {
                "succeeded".into()
            } else {
                "failed".into()
            };
            run.summary.can_copy = success
                && !run.stdout.truncated()
                && std::str::from_utf8(&run.stdout.data).is_ok()
                && !run.stdout.data.contains(&0);
            let result = (|| -> Result<String> {
                ensure!(run.stopped != Some("timed_out"), "Command timed out");
                ensure!(run.stopped != Some("cancelled"), "Command stopped");
                if let Some(error) = &run.failure {
                    anyhow::bail!("{error}");
                }
                ensure!(
                    eof,
                    "Command exited but a descendant kept its output pipe open"
                );
                ensure!(
                    run.status.is_some_and(|s| s.success()),
                    "Command exited with {}",
                    run.status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "unknown status".into())
                );
                if run.copy {
                    ensure!(
                        !run.stdout.truncated(),
                        "Command output exceeds 1 MiB; clipboard unchanged"
                    );
                    let text = String::from_utf8(run.stdout.data.clone())
                        .context("Command output is not UTF-8; clipboard unchanged")?;
                    ensure!(
                        !text.contains('\0'),
                        "Command output contains NUL; clipboard unchanged"
                    );
                    crate::system_clipboard::set_text(clipboard, text)?;
                    Ok("Command finished; output copied to clipboard".into())
                } else {
                    Ok(if run.summary.truncated {
                        "Command finished; output truncated"
                    } else {
                        "Command finished"
                    }
                    .into())
                }
            })();
            run.summary.message = match &result {
                Ok(s) => s.clone(),
                Err(e) => format!("{e:#}"),
            };
            // Execution may succeed while the optional clipboard write fails.
            if let Some(record) = self
                .records
                .0
                .lock()
                .unwrap()
                .iter_mut()
                .find(|r| r.summary.id == run.summary.id)
            {
                record.summary = run.summary;
                record.stdout = run.stdout.data;
                record.stderr = run.stderr.data;
            }
            completed.push((run.context, result));
        }
        completed
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        for mut run in self.running.drain(..) {
            if run.status.is_none() && run.exited.is_none() {
                kill_group(&run.child);
            }
            std::thread::spawn(move || {
                let _ = run.child.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(script: &str, output: &str) -> Config {
        Config::parse(&json!({"mode":"shell","script":script,"output":output,"timeout_seconds":2,"directory":"/tmp"})).unwrap()
    }
    fn finish(manager: &mut Manager, context: &str) -> (Summary, Result<String>) {
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            for (owner, result) in manager.poll(&mut None) {
                if owner == context {
                    return (manager.records.summary(context).unwrap(), result);
                }
            }
            assert!(Instant::now() < until, "Capture did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn captures_separate_streams_and_rejects_replaced_results() {
        let records = Records::default();
        let mut manager = Manager::new(records.clone());
        manager
            .start(
                "a",
                config("printf '  hello 🌍\\n'; printf 'diagnostic' >&2", "show"),
            )
            .unwrap();
        assert!(manager.start("a", config("true", "show")).is_err());
        let (first, result) = finish(&mut manager, "a");
        assert!(result.is_ok());
        assert_eq!(first.state, "succeeded");
        let preview = records.preview("a", &first.id).unwrap();
        assert_eq!(preview.stdout, "  hello 🌍\n");
        assert_eq!(preview.stderr, "diagnostic");
        assert_eq!(records.copy_text("a", &first.id).unwrap(), preview.stdout);
        assert!(records.preview("other", &first.id).is_err());
        manager
            .start("a", config("printf failure; exit 7", "show"))
            .unwrap();
        assert!(records.copy_text("a", &first.id).is_err());
        let (second, result) = finish(&mut manager, "a");
        assert!(result.is_err());
        assert_eq!(second.state, "failed");
        assert!(!second.can_copy);
        assert_eq!(records.preview("a", &second.id).unwrap().stdout, "failure");
    }
    #[test]
    fn drains_overflow_without_deadlock_and_never_copies_partial_or_binary_output() {
        let mut manager = Manager::new(Records::default());
        manager.start("huge",config("/usr/bin/python -c 'import sys; sys.stdout.write(\"x\"*1100000);sys.stderr.write(\"e\"*100000)'","clipboard")).unwrap();
        let (summary, result) = finish(&mut manager, "huge");
        assert!(result.unwrap_err().to_string().contains("exceeds"));
        assert_eq!(summary.stdout_bytes, 1100000);
        assert_eq!(summary.stderr_bytes, 100000);
        assert!(summary.truncated);
        assert!(!summary.can_copy);
        let preview = manager.records.preview("huge", &summary.id).unwrap();
        assert_eq!(preview.stdout.len(), PREVIEW_LIMIT);
        assert!(preview.preview_truncated);
        for (context, script) in [("binary", "printf '\\377'"), ("nul", "printf '\\000'")] {
            manager.start(context, config(script, "clipboard")).unwrap();
            let (summary, result) = finish(&mut manager, context);
            assert!(result.is_err());
            assert!(!summary.can_copy);
        }
    }
    #[test]
    fn stops_and_times_out_only_owned_process_groups() {
        let mut unrelated = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let mut manager = Manager::new(Records::default());
        manager
            .start("stop", config("sleep 5 & wait", "show"))
            .unwrap();
        let id = manager.records.summary("stop").unwrap().id;
        assert!(manager.cancel("stop", "stale").is_err());
        manager.cancel("stop", &id).unwrap();
        let (summary, result) = finish(&mut manager, "stop");
        assert_eq!(summary.state, "cancelled");
        assert!(result.is_err());
        let mut timed = config("sleep 5 & wait", "show");
        timed.timeout_seconds = 1;
        manager.start("timeout", timed).unwrap();
        let (summary, result) = finish(&mut manager, "timeout");
        assert_eq!(summary.state, "timed_out");
        assert!(result.is_err());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }
    #[test]
    fn inherited_output_pipe_does_not_hold_worker_open() {
        let mut manager = Manager::new(Records::default());
        manager
            .start("descendant", config("sleep 1 & printf done", "clipboard"))
            .unwrap();
        let start = Instant::now();
        let (summary, result) = finish(&mut manager, "descendant");
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(result.unwrap_err().to_string().contains("descendant"));
        assert!(!summary.can_copy);
    }
    #[test]
    fn bounded_history_keeps_active_runs_and_evicts_completed_data() {
        let mut manager = Manager::new(Records::default());
        manager.start("active", config("sleep 5", "show")).unwrap();
        let active = manager.records.summary("active").unwrap().id;
        for i in 0..20 {
            let context = format!("c{i}");
            manager
                .start(&context, config("printf small", "show"))
                .unwrap();
            assert!(finish(&mut manager, &context).1.is_ok());
        }
        assert!(manager.records.summary("c0").is_none());
        assert_eq!(manager.records.0.lock().unwrap().len(), MAX_RECORDS);
        assert_eq!(manager.records.summary("active").unwrap().id, active);
        manager.cancel("active", &active).unwrap();
        let _ = finish(&mut manager, "active");
    }
    #[test]
    fn dropping_capture_manager_stops_its_live_child() {
        let mut manager = Manager::new(Records::default());
        manager
            .start("owned", config("sleep 5 & wait", "show"))
            .unwrap();
        let pid = manager.running[0].child.id();
        drop(manager);
        let deadline = Instant::now() + Duration::from_secs(2);
        let path = format!("/proc/{pid}/stat");
        while let Ok(stat) = std::fs::read_to_string(&path) {
            if stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Owned child survived manager shutdown"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
