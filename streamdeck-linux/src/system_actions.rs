//! Native Linux execution of first-party System actions. Profile values are data,
//! never shell source. Blocking work runs outside the host/editor mutex.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const WEBSITE: &str = "com.elgato.streamdeck.system.website";
pub const OPEN: &str = "com.elgato.streamdeck.system.open";
pub const HOTKEY: &str = "com.elgato.streamdeck.system.hotkey";
pub const MULTIMEDIA: &str = "com.elgato.streamdeck.system.multimedia";
pub fn supported(uuid: &str) -> bool {
    matches!(uuid, WEBSITE | OPEN | HOTKEY | MULTIMEDIA)
}
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            WEBSITE,
            "Website",
            json!({"path":"","openInBrowser":true,"browser":""}),
        ),
        (OPEN, "Open application, file or folder", json!({"path":""})),
        (
            HOTKEY,
            "Hotkey",
            json!({"Coalesce":false,"Hotkeys":[{"VKeyCode":-1,"QTKeyCode":33554431,"KeyModifiers":0},{"VKeyCode":-1,"QTKeyCode":33554431,"KeyModifiers":0},{"VKeyCode":-1,"QTKeyCode":33554431,"KeyModifiers":0}]}),
        ),
        (MULTIMEDIA, "Media control", json!({"actionIdx":1})),
    ]
}
pub fn defaults(uuid: &str) -> Value {
    definitions()
        .into_iter()
        .find(|(id, _, _)| *id == uuid)
        .map(|(_, _, v)| v)
        .unwrap_or(json!({}))
}
#[derive(Debug, PartialEq)]
pub enum Plan {
    Launch { program: PathBuf, args: Vec<String> },
    Get(String),
    Chord(Vec<u16>),
    Press(Vec<u16>),
    Release,
    Repeat(Vec<u16>, u8),
}

fn text<'a>(settings: &'a Value, key: &str) -> Result<&'a str> {
    let value = settings[key]
        .as_str()
        .with_context(|| format!("{key} must be text"))?
        .trim();
    ensure!(
        !value.is_empty(),
        "Choose a target before using this action"
    );
    ensure!(!value.contains('\0'), "Target contains a NUL character");
    Ok(value)
}
fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
fn find_program(value: &str) -> Result<PathBuf> {
    ensure!(
        !value.starts_with('-'),
        "Executable cannot start with a dash"
    );
    let path = Path::new(value);
    if path.is_absolute() {
        ensure!(executable(path), "Choose an executable Linux application");
        return Ok(path.into());
    }
    ensure!(
        !value.contains('/') && !value.contains('\\'),
        "Choose an absolute Linux path or an executable name"
    );
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| p.is_absolute())
        .map(|p| p.join(value))
        .find(|p| executable(p))
        .with_context(|| format!("Linux application {value} was not found"))
}
fn opener(target: String) -> Result<Plan> {
    Ok(Plan::Launch {
        program: find_program("xdg-open")?,
        args: vec![target],
    })
}
fn website_url(value: &str) -> Result<url::Url> {
    // Hostnames (including localhost:port) are convenient native editor input.
    let has_scheme = value.split_once(':').is_some_and(|(scheme, rest)| {
        let first = rest.split('/').next().unwrap_or("");
        scheme != "localhost"
            && (first.is_empty() || !first.chars().all(|c| c.is_ascii_digit()))
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    let value = if has_scheme {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let parsed = url::Url::parse(&value).context("Invalid website address")?;
    ensure!(
        !matches!(parsed.scheme(), "javascript" | "data"),
        "This URI scheme is not a website target"
    );
    Ok(parsed)
}
pub fn plan(uuid: &str, settings: &Value) -> Result<Plan> {
    match uuid {
        WEBSITE => {
            let target = website_url(text(settings, "path")?)?;
            let browser_mode = settings
                .get("openInBrowser")
                .map(|v| v.as_bool().context("openInBrowser must be boolean"))
                .transpose()?
                .unwrap_or(true);
            if !browser_mode {
                ensure!(
                    matches!(target.scheme(), "http" | "https"),
                    "Background requests require HTTP or HTTPS"
                );
                Ok(Plan::Get(target.to_string()))
            } else {
                let browser = settings
                    .get("browser")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if browser.is_empty() {
                    opener(target.to_string())
                } else {
                    Ok(Plan::Launch {
                        program: find_program(browser)?,
                        args: vec![target.to_string()],
                    })
                }
            }
        }
        OPEN => {
            let value = text(settings, "path")?;
            let value = value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(value);
            ensure!(
                !(value.as_bytes().get(1) == Some(&b':') || value.starts_with("\\\\")),
                "This is a Windows path; choose its Linux application or file"
            );
            let expanded = if let Some(rest) = value.strip_prefix("~/") {
                PathBuf::from(std::env::var_os("HOME").context("Home directory unavailable")?)
                    .join(rest)
            } else {
                PathBuf::from(value)
            };
            if expanded.is_absolute() {
                ensure!(
                    expanded.exists(),
                    "Target does not exist on Linux; choose an application, file or folder"
                );
                if executable(&expanded) {
                    Ok(Plan::Launch {
                        program: expanded,
                        args: vec![],
                    })
                } else {
                    opener(expanded.to_string_lossy().into_owned())
                }
            } else {
                Ok(Plan::Launch {
                    program: find_program(value)?,
                    args: vec![],
                })
            }
        }
        HOTKEY => Ok(Plan::Chord(crate::system_keyboard::imported_hotkey(
            settings, 0,
        )?)),
        MULTIMEDIA => {
            // Observed first-party profile icons establish these subset values.
            let key = match settings["actionIdx"]
                .as_i64()
                .context("Choose a media control")?
            {
                0 => 165,
                1 => 164,
                2 => 163,
                3 => 166,
                13 => 208,
                14 => 168,
                4 => 113,
                5 => 115,
                6 => 114,
                other => bail!("Media control {other} has no verified Linux mapping yet"),
            };
            Ok(Plan::Chord(vec![key]))
        }
        _ => bail!("System action is not implemented"),
    }
}
#[derive(Clone, Copy)]
pub enum Trigger {
    Down,
    Up,
    Rotate(i8),
    Tap,
}
pub fn input_plan(
    uuid: &str,
    controller: &str,
    settings: &Value,
    trigger: Trigger,
) -> Result<Option<Plan>> {
    if uuid == HOTKEY {
        let index = if controller == "Encoder" { 2 } else { 0 };
        let translated = |index| crate::system_keyboard::imported_hotkey(settings, index);
        return Ok(match trigger {
            Trigger::Up => Some(Plan::Release),
            Trigger::Down => Some(Plan::Press(translated(index)?)),
            Trigger::Tap if controller == "Encoder" => Some(Plan::Chord(translated(2)?)),
            Trigger::Rotate(ticks) if controller == "Encoder" && ticks != 0 => Some(Plan::Repeat(
                translated(if ticks < 0 { 0 } else { 1 })?,
                ticks.unsigned_abs(),
            )),
            _ => None,
        });
    }
    if uuid == MULTIMEDIA && controller == "Encoder" {
        ensure!(
            settings["actionIdx"] == 18,
            "Select System volume for this dial"
        );
        return Ok(match trigger {
            Trigger::Up | Trigger::Tap => Some(Plan::Chord(vec![113])),
            Trigger::Rotate(ticks) if ticks != 0 => Some(Plan::Repeat(
                vec![if ticks < 0 { 114 } else { 115 }],
                ticks.unsigned_abs(),
            )),
            _ => None,
        });
    }
    ensure!(
        controller == "Keypad",
        "This system action's dial mapping is not implemented yet"
    );
    if matches!(trigger, Trigger::Up) {
        Ok(Some(plan(uuid, settings)?))
    } else {
        Ok(None)
    }
}
struct Job {
    context: String,
    plan: Plan,
}
pub struct Report {
    pub context: String,
    pub result: Result<String>,
}
pub struct Worker {
    sender: mpsc::SyncSender<Job>,
    reports: mpsc::Receiver<Report>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(32);
        let (report_tx, reports) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            let mut keyboard: Option<crate::system_keyboard::Keyboard> = None;
            let mut holds = HashMap::new();
            let mut requests: Vec<thread::JoinHandle<()>> = Vec::new();
            let mut children: Vec<(String, Child)> = Vec::new();
            while !stopped.load(Ordering::Acquire) {
                children.retain_mut(|(context, child)| match child.try_wait() {
                    Ok(None) => true,
                    Ok(Some(status)) => {
                        if !status.success() {
                            let _ = report_tx.send(Report {
                                context: context.clone(),
                                result: Err(anyhow::anyhow!("Application exited with {status}")),
                            });
                        }
                        false
                    }
                    Err(e) => {
                        let _ = report_tx.send(Report {
                            context: context.clone(),
                            result: Err(e.into()),
                        });
                        false
                    }
                });
                requests.retain(|task| !task.is_finished());
                let job = match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                if let Plan::Get(url) = job.plan {
                    if requests.len() >= 8 {
                        let _ = report_tx.send(Report {
                            context: job.context,
                            result: Err(anyhow::anyhow!("Too many background requests")),
                        });
                        continue;
                    }
                    let reports = report_tx.clone();
                    requests.push(thread::spawn(move || {
                        let result = (|| -> Result<String> {
                            let client = reqwest::blocking::Client::builder()
                                .connect_timeout(Duration::from_secs(2))
                                .timeout(Duration::from_secs(5))
                                .redirect(reqwest::redirect::Policy::limited(5))
                                .build()?;
                            let response = client
                                .get(url)
                                .send()
                                .map_err(|e| e.without_url())?
                                .error_for_status()
                                .map_err(|e| e.without_url())?;
                            Ok(format!("Background request returned {}", response.status()))
                        })();
                        let _ = reports.send(Report {
                            context: job.context,
                            result,
                        });
                    }));
                    continue;
                }
                let keyboard_action = matches!(
                    job.plan,
                    Plan::Chord(_) | Plan::Press(_) | Plan::Release | Plan::Repeat(_, _)
                );
                let result = (|| -> Result<String> {
                    match job.plan {
                        Plan::Launch { program, args } => {
                            ensure!(
                                children.len() < 32,
                                "Too many applications are still running"
                            );
                            let child = Command::new(program)
                                .args(args)
                                .stdin(Stdio::null())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                                .context("Could not launch application")?;
                            children.push((job.context.clone(), child));
                            Ok("Application launch requested".into())
                        }
                        Plan::Get(_) => unreachable!(),
                        Plan::Release => {
                            if let Some(token) = holds.remove(&job.context)
                                && let Some(keyboard) = &mut keyboard
                            {
                                keyboard.release(token)?;
                            }
                            Ok("Keyboard shortcut released".into())
                        }
                        Plan::Press(keys) => {
                            if !keys.is_empty() && !holds.contains_key(&job.context) {
                                if keyboard.is_none() {
                                    keyboard = Some(crate::system_keyboard::Keyboard::new()?);
                                }
                                if stopped.load(Ordering::Acquire) {
                                    bail!("Action cancelled during shutdown");
                                }
                                let token = keyboard.as_mut().unwrap().press(&keys)?;
                                holds.insert(job.context.clone(), token);
                            }
                            Ok("Keyboard shortcut pressed".into())
                        }
                        Plan::Repeat(keys, count) => {
                            if keyboard.is_none() {
                                keyboard = Some(crate::system_keyboard::Keyboard::new()?);
                            }
                            for _ in 0..count {
                                if stopped.load(Ordering::Acquire) {
                                    bail!("Action cancelled during shutdown");
                                }
                                keyboard.as_mut().unwrap().chord(&keys)?;
                            }
                            Ok("Dial shortcuts sent".into())
                        }
                        Plan::Chord(keys) => {
                            if keyboard.is_none() {
                                keyboard = Some(crate::system_keyboard::Keyboard::new()?);
                            }
                            if stopped.load(Ordering::Acquire) {
                                bail!("Action cancelled during shutdown");
                            }
                            keyboard.as_mut().unwrap().chord(&keys)?;
                            Ok("Keyboard shortcut sent".into())
                        }
                    }
                })();
                if keyboard_action && result.is_err() {
                    holds.clear();
                    if let Some(mut failed) = keyboard.take() {
                        let _ = failed.release_all();
                    }
                }
                let _ = report_tx.send(Report {
                    context: job.context,
                    result,
                });
            }
            if let Some(keyboard) = &mut keyboard {
                let _ = keyboard.release_all();
            }
            drop(keyboard); // Release held controls before waiting on pending HTTP work.
            for request in requests {
                let _ = request.join();
            }
            // Opened user applications outlive the host; reap without terminating them.
            for (_, mut child) in children {
                thread::spawn(move || {
                    let _ = child.wait();
                });
            }
        });
        Self {
            sender,
            reports,
            stop,
            thread: Some(thread),
        }
    }
    pub fn submit(&self, context: &str, plan: Plan) -> Result<()> {
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "System actions are stopped"
        );
        self.sender.try_send(Job{context:context.into(),plan}).map_err(|_|{
            // A lost release must never leave a modifier held. Stop drains no more
            // presses and destroys the keyboard, releasing every held key.
            self.cancel();
            anyhow::anyhow!("System action queue is full; keyboard released. Restart Deckforge to resume system actions")
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_never_interpret_shell_source() {
        assert!(plan(OPEN, &json!({"path":"/bin/true; touch /tmp/not-created"})).is_err());
        assert!(
            plan(OPEN, &json!({"path":"C:\\Windows\\notepad.exe"}))
                .unwrap_err()
                .to_string()
                .contains("Windows path")
        );
        assert!(plan(OPEN, &json!({"path":"--help"})).is_err());
        assert_eq!(
            plan(
                WEBSITE,
                &json!({"path":"example.org/a?x=$(touch%20bad)","openInBrowser":false})
            )
            .unwrap(),
            Plan::Get("https://example.org/a?x=$(touch%20bad)".into())
        );
        assert!(
            plan(
                WEBSITE,
                &json!({"path":"file:///tmp/a","openInBrowser":false})
            )
            .is_err()
        );
    }
    #[test]
    fn website_preserves_uri_handlers_and_normalizes_hostnames() {
        assert_eq!(
            website_url("spotify:album:test").unwrap().as_str(),
            "spotify:album:test"
        );
        assert_eq!(
            website_url("localhost:8000/page").unwrap().as_str(),
            "https://localhost:8000/page"
        );
        assert_eq!(
            website_url("example.org:8080/page").unwrap().as_str(),
            "https://example.org:8080/page"
        );
        assert!(website_url("javascript:alert(1)").is_err());
        assert!(website_url("data:text/plain,hello").is_err());
    }
    #[test]
    fn explicit_executable_preserves_spaces_and_shell_characters() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("app $(literal)");
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            plan(OPEN, &json!({"path":format!("\"{}\"",path.display())})).unwrap(),
            Plan::Launch {
                program: path,
                args: vec![]
            }
        );
    }
    #[test]
    fn dial_triggers_select_original_shortcut_records() {
        let settings = json!({"Hotkeys":[{"QTKeyCode":65},{"QTKeyCode":66},{"QTKeyCode":67}]});
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Rotate(-3)).unwrap(),
            Some(Plan::Repeat(vec![30], 3))
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Rotate(2)).unwrap(),
            Some(Plan::Repeat(vec![48], 2))
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Down).unwrap(),
            Some(Plan::Press(vec![46]))
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Tap).unwrap(),
            Some(Plan::Chord(vec![46]))
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &json!({}), Trigger::Up).unwrap(),
            Some(Plan::Release)
        );
        assert_eq!(
            input_plan(
                MULTIMEDIA,
                "Encoder",
                &json!({"actionIdx":18}),
                Trigger::Rotate(-2)
            )
            .unwrap(),
            Some(Plan::Repeat(vec![114], 2))
        );
        assert_eq!(
            input_plan(
                MULTIMEDIA,
                "Encoder",
                &json!({"actionIdx":18}),
                Trigger::Tap
            )
            .unwrap(),
            Some(Plan::Chord(vec![113]))
        );
    }
    #[test]
    fn unknown_media_enums_are_not_guessed() {
        assert!(plan(MULTIMEDIA, &json!({"actionIdx":999})).is_err());
        assert_eq!(
            plan(MULTIMEDIA, &json!({"actionIdx":4})).unwrap(),
            Plan::Chord(vec![113])
        );
    }
    #[test]
    fn worker_performs_local_get_and_reports_http_failures() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for status in ["204 No Content", "503 Service Unavailable"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = [0; 4096];
                let n = stream.read(&mut bytes).unwrap();
                assert!(String::from_utf8_lossy(&bytes[..n]).starts_with("GET /probe HTTP/1.1"));
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let worker = Worker::new();
        for success in [true, false] {
            worker
                .submit("test", Plan::Get(format!("http://{address}/probe")))
                .unwrap();
            let report = worker.reports.recv_timeout(Duration::from_secs(8)).unwrap();
            assert_eq!(report.result.is_ok(), success);
        }
        server.join().unwrap();
    }
    #[test]
    fn worker_launches_an_exact_local_program() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("marker");
        let executable = temp.path().join("app");
        fs::write(&executable, "#!/bin/sh\nprintf tested > \"$1\"\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let worker = Worker::new();
        worker
            .submit(
                "test",
                Plan::Launch {
                    program: executable,
                    args: vec![marker.to_str().unwrap().into()],
                },
            )
            .unwrap();
        assert!(
            worker
                .reports
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .result
                .is_ok()
        );
        for _ in 0..100 {
            if marker.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fs::read_to_string(marker).unwrap(), "tested");
    }
}
