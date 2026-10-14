//! Native Linux execution of first-party System actions. Profile values are data,
//! never shell source except in the explicit Run command shell mode.
//! Blocking work runs outside the host/editor mutex.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
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
    time::{Duration, Instant},
};

pub const SLEEP: &str = "com.elgato.streamdeck.system.sleep";
pub const WEBSITE: &str = "com.elgato.streamdeck.system.website";
pub const OPEN: &str = "com.elgato.streamdeck.system.open";
pub const CLOSE: &str = "com.elgato.streamdeck.system.close";
pub const TEXT: &str = "com.elgato.streamdeck.system.text";
pub const MOUSE_WHEEL: &str = "com.elgato.streamdeck.system.mouse-wheel";
pub const TIMER: &str = "com.elgato.streamdeck.system.timer";
pub const HOTKEY: &str = "com.elgato.streamdeck.system.hotkey";
pub const HOTKEY_SWITCH: &str = "com.elgato.streamdeck.system.hotkeyswitch";
pub const MULTIMEDIA: &str = "com.elgato.streamdeck.system.multimedia";
pub fn supported(uuid: &str) -> bool {
    matches!(
        uuid,
        WEBSITE
            | crate::system_clipboard::UUID
            | crate::system_command::UUID
            | crate::system_app::UUID
            | OPEN
            | CLOSE
            | TEXT
            | TIMER
            | HOTKEY
            | HOTKEY_SWITCH
            | MOUSE_WHEEL
            | MULTIMEDIA
            | crate::brightness::UUID
            | SLEEP
    )
}
pub fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            crate::system_clipboard::UUID,
            "Clipboard",
            json!({"operation":"copy_text","text":"","terminal_shortcuts":false,"linuxKeyMode":"logical"}),
        ),
        (
            crate::system_command::UUID,
            "Run command",
            json!({"mode":"program","program":"","args":"","script":"","directory":"","terminal":false,"terminal_program":"konsole","terminal_args":"--separate --hold -e","output":"show","timeout_seconds":30,"environment":[]}),
        ),
        (SLEEP, "Sleep", json!({"DeviceUUID":""})),
        (
            crate::brightness::UUID,
            "Device brightness",
            json!({"actionIdx":0}),
        ),
        (
            WEBSITE,
            "Website",
            json!({"path":"","openInBrowser":true,"browser":""}),
        ),
        (OPEN, "Open", json!({"path":""})),
        (
            crate::system_app::UUID,
            "Open Application",
            json!({"exec":"","args":"","bring_to_front":true,"long_press":"quit"}),
        ),
        (
            CLOSE,
            "Close",
            json!({"path":"","forceQuit":false,"linuxCloseMethod":"windows","linuxMatchArgs":""}),
        ),
        (MOUSE_WHEEL, "Mouse Wheel", json!({})),
        (
            TIMER,
            "Timer",
            json!({"duration":30,"actionIdx":0,"lastUserFile":""}),
        ),
        (
            TEXT,
            "Text",
            json!({"pastedText":"","isSendingEnter":false,"isTypingMode":false,"linuxKeyMode":"logical"}),
        ),
        (
            HOTKEY,
            "Hotkey",
            json!({"Coalesce":false,"Hotkeys":[{"VKeyCode":-1,"QTKeyCode":33554431,"KeyModifiers":0},{"VKeyCode":-1,"QTKeyCode":33554431,"KeyModifiers":0},{"VKeyCode":-1,"QTKeyCode":33554431,"KeyModifiers":0}]}),
        ),
        (MULTIMEDIA, "Multimedia", json!({"actionIdx":1})),
        (
            HOTKEY_SWITCH,
            "Hotkey Switch",
            json!({"Hotkeys":[{"QTKeyCode":33554431,"VKeyCode":-1,"KeyModifiers":0},{"QTKeyCode":33554431,"VKeyCode":-1,"KeyModifiers":0}]}),
        ),
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
    PrepareTyping,
    Type {
        text: String,
        enter: bool,
    },
    Clipboard(String),
    Command(crate::system_command::Config),
    CommandCancel(String),
    CommandCopy(String),
    /// Initialize the virtual keyboard without pressing or releasing any key.
    PrepareKeyboard,
    Launch {
        program: PathBuf,
        args: Vec<String>,
    },
    Get(String),
    Close(crate::system_close::Config),
    Application(crate::system_app::Config, crate::system_app::Operation),
    Scroll {
        ticks: i32,
        horizontal: bool,
    },
    Paste {
        text: String,
        enter: bool,
        shortcut: crate::system_keyboard::Chord,
    },
    Chord(crate::system_keyboard::Chord),
    Press(crate::system_keyboard::Chord),
    Release,
    Repeat(crate::system_keyboard::Chord, u8),
    Dial {
        stroke: crate::system_dial::Stroke,
        count: u8,
        coalesce: bool,
    },
    CancelDials(HashSet<String>),
}
impl Plan {
    fn prepares_keyboard(&self) -> bool {
        matches!(self, Self::PrepareKeyboard)
    }
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
pub(crate) fn find_program(value: &str) -> Result<PathBuf> {
    find_program_in_path(value, &std::env::var_os("PATH").unwrap_or_default())
}
pub(crate) fn find_program_in_path(value: &str, search_path: &std::ffi::OsStr) -> Result<PathBuf> {
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
    std::env::split_paths(search_path)
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
        crate::system_clipboard::UUID => crate::system_clipboard::plan(settings),
        crate::system_command::UUID => Ok(Plan::Command(crate::system_command::Config::parse(
            settings,
        )?)),
        CLOSE => Ok(Plan::Close(crate::system_close::Config::parse(settings)?)),
        TEXT => {
            let typing = settings
                .get("isTypingMode")
                .map(|v| v.as_bool().context("Typing mode must be boolean"))
                .transpose()?
                .unwrap_or(false);
            let content = settings
                .get("pastedText")
                .map(|value| value.as_str().context("Text must be a string"))
                .transpose()?
                .unwrap_or("");
            ensure!(
                content.len() <= 1024 * 1024 && !content.contains('\0'),
                "Text must be at most 1 MiB without NUL characters"
            );
            let enter = settings
                .get("isSendingEnter")
                .map(|v| v.as_bool().context("Send Enter must be boolean"))
                .transpose()?
                .unwrap_or(false);
            ensure!(
                !content.is_empty() || enter,
                "Enter text or enable Send Enter"
            );
            if typing {
                crate::system_typing::keysyms(content, enter)?;
                return Ok(Plan::Type {
                    text: content.into(),
                    enter,
                });
            }
            let custom = if settings["Hotkey"].is_object() {
                crate::system_keyboard::imported_chord(&json!({"Hotkeys":[settings["Hotkey"]]}), 0)?
            } else {
                vec![].into()
            };
            Ok(Plan::Paste {
                text: content.into(),
                enter,
                shortcut: if custom.is_empty() {
                    crate::system_keyboard::Chord::from_settings(
                        vec![29, 47],
                        if settings["Hotkey"].get("linuxKeyMode").is_some() {
                            &settings["Hotkey"]
                        } else {
                            settings
                        },
                    )?
                } else {
                    custom
                },
            })
        }
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
        HOTKEY => Ok(Plan::Chord(crate::system_keyboard::imported_chord(
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
            Ok(Plan::Chord(vec![key].into()))
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
#[cfg(test)]
pub fn input_plan(
    uuid: &str,
    controller: &str,
    settings: &Value,
    trigger: Trigger,
) -> Result<Option<Plan>> {
    input_plan_state(uuid, controller, settings, trigger, 0)
}
pub fn input_plan_state(
    uuid: &str,
    controller: &str,
    settings: &Value,
    trigger: Trigger,
    state: u64,
) -> Result<Option<Plan>> {
    if uuid == HOTKEY_SWITCH {
        ensure!(
            controller == "Keypad",
            "Use Action Trigger to place Hotkey Switch on a dial"
        );
        return Ok(match trigger {
            Trigger::Down => Some(Plan::Press(crate::system_keyboard::imported_chord(
                settings,
                (state % 2) as usize,
            )?)),
            Trigger::Up => Some(Plan::Release),
            _ => None,
        });
    }
    if uuid == HOTKEY {
        let index = if controller == "Encoder" { 2 } else { 0 };
        let translated = |index| crate::system_keyboard::imported_chord(settings, index);
        let queued = |index: usize, count| -> Result<Plan> {
            let keys = translated(index)?;
            let coalesce = settings
                .get("Coalesce")
                .map(|v| v.as_bool().context("Coalesce must be boolean"))
                .transpose()?
                .unwrap_or(true);
            let chord = &settings["Hotkeys"][index];
            let modifiers = crate::system_keyboard::effective_modifiers(chord)? & 0x1fff != 0;
            Ok(Plan::Dial {
                stroke: crate::system_dial::Stroke {
                    keys,
                    gesture: index as u8,
                    interval: Duration::from_millis(if modifiers { 50 } else { 30 }),
                },
                count,
                coalesce,
            })
        };
        return Ok(match trigger {
            Trigger::Up => Some(Plan::Release),
            Trigger::Down => Some(Plan::Press(translated(index)?)),
            Trigger::Tap if controller == "Encoder" => Some(queued(2, 1)?),
            Trigger::Rotate(ticks) if controller == "Encoder" && ticks != 0 => {
                Some(queued(usize::from(ticks > 0), ticks.unsigned_abs())?)
            }
            _ => None,
        });
    }
    if uuid == MULTIMEDIA && controller == "Encoder" {
        ensure!(
            settings["actionIdx"] == 18,
            "Select System volume for this dial"
        );
        return Ok(match trigger {
            Trigger::Up | Trigger::Tap => Some(Plan::Chord(vec![113].into())),
            Trigger::Rotate(ticks) if ticks != 0 => Some(Plan::Repeat(
                vec![if ticks < 0 { 114 } else { 115 }].into(),
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
fn emit_dial_events(
    events: Vec<crate::system_dial::Event>,
    keyboard: &mut crate::system_keyboard::Keyboard,
    holds: &mut HashMap<String, u64>,
) -> Result<()> {
    for event in events {
        match event {
            crate::system_dial::Event::Down(context, keys) => {
                let token = keyboard.press_chord(&keys)?;
                holds.insert(context, token);
            }
            crate::system_dial::Event::Up(context) => {
                if let Some(token) = holds.remove(&context) {
                    keyboard.release(token)?;
                }
            }
        }
    }
    Ok(())
}
struct PendingPaste {
    context: String,
    enter: bool,
    due: Instant,
}
pub struct Report {
    pub context: String,
    pub result: Result<String>,
    /// Identifies the preparation request on both success and failure.
    pub prepared_keyboard: bool,
}
pub struct Worker {
    pub typing_status: crate::system_typing::Shared,
    pub command_results: crate::system_command::capture::Records,
    sender: mpsc::SyncSender<Job>,
    reports: mpsc::Receiver<Report>,
    stop: Arc<AtomicBool>,
    keyboard_ready: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        Self::start(None)
    }
    fn start(mut keyboard: Option<crate::system_keyboard::Keyboard>) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(32);
        let (report_tx, reports) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let keyboard_ready = Arc::new(AtomicBool::new(false));
        let ready = keyboard_ready.clone();
        let command_results = crate::system_command::capture::Records::default();
        let records = command_results.clone();
        let typing_status = crate::system_typing::Shared::default();
        let typing_state = typing_status.clone();
        let thread = thread::spawn(move || {
            if let Some(active) = keyboard.as_mut() {
                active.bind_cancellation(stopped.clone());
            }
            let mut typing: Option<crate::system_typing::Worker> = None;
            let mut commands = crate::system_command::capture::Manager::new(records);
            let mut clipboard: Option<arboard::Clipboard> = None;
            let mut mouse: Option<crate::system_mouse::Mouse> = None;
            let mut holds = HashMap::new();
            let mut dials = crate::system_dial::Dials::default();
            let mut dial_holds = HashMap::new();
            let mut requests: Vec<thread::JoinHandle<()>> = Vec::new();
            let mut children: Vec<(String, Child)> = Vec::new();
            let mut applications: Option<crate::system_app::Worker> = None;
            let mut paste: Option<PendingPaste> = None;
            let mut deferred: VecDeque<Job> = VecDeque::new();
            while !stopped.load(Ordering::Acquire) {
                if paste.as_ref().is_some_and(|p| Instant::now() >= p.due) {
                    let pending = paste.take().unwrap();
                    let result = (|| -> Result<String> {
                        ensure!(!stopped.load(Ordering::Acquire), "Text action cancelled");
                        if pending.enter {
                            keyboard
                                .as_mut()
                                .context("Paste keyboard unavailable")?
                                .chord(&[28])?;
                        }
                        Ok("Text paste sent to the focused application".into())
                    })();
                    if result.is_err() {
                        ready.store(false, Ordering::Release);
                        holds.clear();
                        dial_holds.clear();
                        dials.clear();
                        if let Some(mut failed) = keyboard.take() {
                            let _ = failed.release_all();
                        }
                    }
                    let _ = report_tx.send(Report {
                        context: pending.context,
                        result,
                        prepared_keyboard: false,
                    });
                }
                // Captured output can write the clipboard too. Keep it behind the
                // current paste, while still allowing explicit Stop below.
                let completed = if paste.is_none() {
                    commands.poll(&mut clipboard)
                } else {
                    Vec::new()
                };
                for (context, result) in completed {
                    let _ = report_tx.send(Report {
                        context,
                        result,
                        prepared_keyboard: false,
                    });
                }
                if let Some(active) = &mut keyboard {
                    let events = dials.poll(Instant::now(), paste.is_none());
                    if let Err(error) = emit_dial_events(events, active, &mut dial_holds) {
                        if let Some(pending) = paste.take() {
                            let _ = report_tx.send(Report {
                                context: pending.context,
                                result: Err(anyhow::anyhow!(
                                    "Text paste cancelled after keyboard failure"
                                )),
                                prepared_keyboard: false,
                            });
                        }
                        ready.store(false, Ordering::Release);
                        holds.clear();
                        dial_holds.clear();
                        dials.clear();
                        if let Some(mut failed) = keyboard.take() {
                            let _ = failed.release_all();
                        }
                        let _ = report_tx.send(Report {
                            context: String::new(),
                            result: Err(error),
                            prepared_keyboard: false,
                        });
                    }
                }
                children.retain_mut(|(context, child)| match child.try_wait() {
                    Ok(None) => true,
                    Ok(Some(status)) => {
                        if !status.success() {
                            let _ = report_tx.send(Report {
                                context: context.clone(),
                                result: Err(anyhow::anyhow!("Application exited with {status}")),
                                prepared_keyboard: false,
                            });
                        }
                        false
                    }
                    Err(e) => {
                        let _ = report_tx.send(Report {
                            context: context.clone(),
                            result: Err(e.into()),
                            prepared_keyboard: false,
                        });
                        false
                    }
                });
                requests.retain(|task| !task.is_finished());
                let now = Instant::now();
                let wait = dials.wait(now, paste.is_none()).min(
                    paste.as_ref().map_or(Duration::from_millis(50), |p| {
                        p.due.saturating_duration_since(now)
                    }),
                );
                let queued = if paste.is_none() {
                    deferred.pop_front()
                } else {
                    None
                };
                let job = match queued
                    .map(Ok)
                    .unwrap_or_else(|| receiver.recv_timeout(wait))
                {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                if paste.is_some() {
                    let priority = match &job.plan {
                        Plan::Release => {
                            if holds.contains_key(&job.context) {
                                deferred.retain(|queued| {
                                    !(queued.context == job.context
                                        && matches!(queued.plan, Plan::Press(_)))
                                });
                                true
                            } else {
                                !deferred.iter().any(|queued| {
                                    queued.context == job.context
                                        && matches!(queued.plan, Plan::Press(_))
                                })
                            }
                        }
                        Plan::CancelDials(contexts) => {
                            deferred.retain(|queued| {
                                !(contexts.contains(&queued.context)
                                    && matches!(queued.plan, Plan::Dial { .. }))
                            });
                            true
                        }
                        Plan::CommandCancel(_) => true,
                        _ => false,
                    };
                    if !priority {
                        if deferred.len() < 32 {
                            deferred.push_back(job);
                        } else {
                            // A release rejected under pressure must not leave a
                            // still-deferred press to execute without its release.
                            if matches!(job.plan, Plan::Release) {
                                deferred.retain(|queued| {
                                    !(queued.context == job.context
                                        && matches!(queued.plan, Plan::Press(_)))
                                });
                            }
                            let _ = report_tx.send(Report {
                                context: job.context,
                                result: Err(anyhow::anyhow!(
                                    "System action queue is full during text paste"
                                )),
                                prepared_keyboard: false,
                            });
                        }
                        continue;
                    }
                }
                if let Plan::Get(url) = job.plan {
                    if requests.len() >= 8 {
                        let _ = report_tx.send(Report {
                            context: job.context,
                            result: Err(anyhow::anyhow!("Too many background requests")),
                            prepared_keyboard: false,
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
                            prepared_keyboard: false,
                        });
                    }));
                    continue;
                }
                if let Plan::Application(config, operation) = job.plan {
                    let worker = applications
                        .get_or_insert_with(|| crate::system_app::Worker::new(report_tx.clone()));
                    if let Err(error) = worker.submit(job.context.clone(), config, operation) {
                        let _ = report_tx.send(Report {
                            context: job.context,
                            result: Err(error),
                            prepared_keyboard: false,
                        });
                    }
                    continue;
                }
                if let Plan::Close(config) = job.plan {
                    let worker = applications
                        .get_or_insert_with(|| crate::system_app::Worker::new(report_tx.clone()));
                    if let Err(error) = worker.submit_close(job.context.clone(), config) {
                        let _ = report_tx.send(Report {
                            context: job.context,
                            result: Err(error),
                            prepared_keyboard: false,
                        });
                    }
                    continue;
                }
                if let Plan::Type { text, enter } = job.plan {
                    let result = (|| -> Result<()> {
                        ensure!(
                            holds.is_empty() && dial_holds.is_empty(),
                            "Release held keyboard shortcuts before typing text"
                        );
                        let actor = typing
                            .as_ref()
                            .context("Enable desktop typing in Text settings first")?;
                        actor.type_text(&job.context, &text, enter)?;
                        // Keep keyboard actions FIFO-ordered. UI cancellation uses Shared
                        // directly and never waits behind this queue.
                        while typing_state.is_typing() && !stopped.load(Ordering::Acquire) {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        let _ = report_tx.send(Report {
                            context: job.context,
                            result: Err(error),
                            prepared_keyboard: false,
                        });
                    }
                    continue;
                }
                let paste_job = matches!(&job.plan, Plan::Paste { .. });
                let keyboard_action = matches!(
                    job.plan,
                    Plan::PrepareKeyboard
                        | Plan::Chord(_)
                        | Plan::Press(_)
                        | Plan::Release
                        | Plan::Repeat(_, _)
                        | Plan::Dial { .. }
                        | Plan::CancelDials(_)
                        | Plan::Paste { .. }
                );
                let prepared_keyboard = job.plan.prepares_keyboard();
                let result = (|| -> Result<String> {
                    match job.plan {
                        Plan::PrepareTyping => {
                            typing
                                .get_or_insert_with(|| {
                                    crate::system_typing::Worker::start(
                                        typing_state.clone(),
                                        report_tx.clone(),
                                    )
                                })
                                .prepare()?;
                            Ok("Desktop typing setup requested".into())
                        }
                        Plan::Type { .. } => unreachable!(),
                        Plan::Clipboard(text) => {
                            crate::system_clipboard::set_text(&mut clipboard, text)?;
                            Ok("Text copied to desktop clipboard".into())
                        }
                        Plan::Command(config) => {
                            ensure!(
                                !commands.is_running(&job.context),
                                "This command is already running; stop it before starting another"
                            );
                            ensure!(
                                children.len() + commands.len() < 32,
                                "Too many applications are still running"
                            );
                            if config.output != crate::system_command::Output::Discard {
                                commands.start(&job.context, config)?;
                                return Ok("Command running".into());
                            }
                            let child = config
                                .command()
                                .stdin(Stdio::null())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                                .context("Could not run command")?;
                            children.push((job.context.clone(), child));
                            commands.records.forget(&job.context);
                            Ok("Command started".into())
                        }
                        Plan::CommandCancel(id) => {
                            commands.cancel(&job.context, &id)?;
                            Ok("Command stop requested".into())
                        }
                        Plan::CommandCopy(id) => {
                            let text = commands.records.copy_text(&job.context, &id)?;
                            crate::system_clipboard::set_text(&mut clipboard, text)?;
                            Ok("Command output copied to clipboard".into())
                        }
                        Plan::Application(_, _) => unreachable!(),
                        Plan::Dial {
                            stroke,
                            count,
                            coalesce,
                        } => {
                            if stroke.keys.is_empty() || count == 0 {
                                return Ok("No keyboard shortcuts to send".into());
                            }
                            prepare_keyboard(&mut keyboard, &ready, &stopped)?;
                            ensure!(!stopped.load(Ordering::Acquire), "Dial shortcut cancelled");
                            dials.enqueue(&job.context, stroke, count, coalesce, Instant::now());
                            Ok("Dial shortcuts queued".into())
                        }
                        Plan::CancelDials(contexts) => {
                            let events = dials.cancel(&contexts);
                            if let Some(active) = &mut keyboard {
                                emit_dial_events(events, active, &mut dial_holds)?;
                            }
                            Ok("Departing dial shortcuts cancelled".into())
                        }
                        Plan::Scroll { ticks, horizontal } => {
                            if mouse.is_none() {
                                mouse = Some(crate::system_mouse::Mouse::new()?);
                            }
                            ensure!(!stopped.load(Ordering::Acquire), "Scroll cancelled");
                            if let Err(error) = mouse.as_mut().unwrap().scroll(ticks, horizontal) {
                                mouse = None;
                                return Err(error);
                            }
                            Ok("Scroll sent".into())
                        }
                        Plan::Close(_) => unreachable!(),
                        Plan::Paste {
                            text,
                            enter,
                            shortcut,
                        } => {
                            prepare_keyboard(&mut keyboard, &ready, &stopped)?;
                            ensure!(!stopped.load(Ordering::Acquire), "Text action cancelled");
                            crate::system_clipboard::set_text(&mut clipboard, text)?;
                            keyboard.as_mut().unwrap().send_chord(&shortcut)?;
                            paste = Some(PendingPaste {
                                context: job.context.clone(),
                                enter,
                                due: Instant::now() + Duration::from_millis(150),
                            });
                            Ok(String::new())
                        }
                        Plan::PrepareKeyboard => {
                            prepare_keyboard(&mut keyboard, &ready, &stopped)?;
                            Ok("Keyboard ready".into())
                        }
                        Plan::Launch { program, args } => {
                            ensure!(
                                children.len() + commands.len() < 32,
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
                            if keys.is_empty() {
                                return Ok("No keyboard shortcut configured".into());
                            }
                            if !holds.contains_key(&job.context) {
                                prepare_keyboard(&mut keyboard, &ready, &stopped)?;
                                if stopped.load(Ordering::Acquire) {
                                    bail!("Action cancelled during shutdown");
                                }
                                let token = keyboard.as_mut().unwrap().press_chord(&keys)?;
                                holds.insert(job.context.clone(), token);
                            }
                            Ok("Keyboard shortcut pressed".into())
                        }
                        Plan::Repeat(keys, count) => {
                            if keys.is_empty() || count == 0 {
                                return Ok("No keyboard shortcuts to send".into());
                            }
                            prepare_keyboard(&mut keyboard, &ready, &stopped)?;
                            for _ in 0..count {
                                if stopped.load(Ordering::Acquire) {
                                    bail!("Action cancelled during shutdown");
                                }
                                keyboard.as_mut().unwrap().send_chord(&keys)?;
                            }
                            Ok("Dial shortcuts sent".into())
                        }
                        Plan::Chord(keys) => {
                            if keys.is_empty() {
                                return Ok("No keyboard shortcut configured".into());
                            }
                            prepare_keyboard(&mut keyboard, &ready, &stopped)?;
                            if stopped.load(Ordering::Acquire) {
                                bail!("Action cancelled during shutdown");
                            }
                            keyboard.as_mut().unwrap().send_chord(&keys)?;
                            Ok("Keyboard shortcut sent".into())
                        }
                    }
                })();
                if keyboard_action && result.is_err() {
                    if let Some(pending) = paste.take() {
                        let _ = report_tx.send(Report {
                            context: pending.context,
                            result: Err(anyhow::anyhow!(
                                "Text paste cancelled after keyboard failure"
                            )),
                            prepared_keyboard: false,
                        });
                    }
                    ready.store(false, Ordering::Release);
                    holds.clear();
                    dial_holds.clear();
                    dials.clear();
                    if let Some(mut failed) = keyboard.take() {
                        let _ = failed.release_all();
                    }
                }
                if paste_job && result.is_ok() {
                    continue;
                }
                let _ = report_tx.send(Report {
                    context: job.context,
                    result,
                    prepared_keyboard,
                });
            }
            ready.store(false, Ordering::Release);
            if let Some(keyboard) = &mut keyboard {
                let _ = keyboard.release_all();
            }
            drop(keyboard); // Release held controls before waiting on pending HTTP work.
            if let Some(worker) = &applications {
                worker.cancel();
            }
            drop(typing);
            drop(commands); // Stop supervised commands before waiting for HTTP workers.
            drop(applications);
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
            typing_status,
            command_results,
            sender,
            reports,
            stop,
            keyboard_ready,
            thread: Some(thread),
        }
    }
    pub fn submit(&self, context: &str, plan: Plan) -> Result<()> {
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "System actions are stopped"
        );
        self.sender.try_send(Job{context:context.into(),plan}).map_err(|error|{
            if let mpsc::TrySendError::Full(job) = &error
                && matches!(job.plan, Plan::Type { .. } | Plan::PrepareTyping) {
                return anyhow::anyhow!("Typing queue is full; wait for the current text or stop typing");
            }
            // A lost release must never leave a modifier held. Stop drains no more
            // presses and destroys the keyboard, releasing every held key.
            self.cancel();
            anyhow::anyhow!("System action queue is full; keyboard released. Restart Deckforge to resume system actions")
        })
    }
    pub fn drain(&self) -> Vec<Report> {
        self.reports.try_iter().collect()
    }
    pub fn keyboard_ready(&self) -> bool {
        !self.stop.load(Ordering::Acquire)
            && self.keyboard_ready.load(Ordering::Acquire)
            && self
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
    }
    pub fn cancel(&self) {
        self.typing_status.cancel();
        self.stop.store(true, Ordering::Release);
        self.keyboard_ready.store(false, Ordering::Release);
    }
}
fn prepare_keyboard(
    keyboard: &mut Option<crate::system_keyboard::Keyboard>,
    ready: &AtomicBool,
    stopped: &Arc<AtomicBool>,
) -> Result<()> {
    if keyboard.is_none() {
        ready.store(false, Ordering::Release);
        let mut active = crate::system_keyboard::Keyboard::new()?;
        active.bind_cancellation(stopped.clone());
        *keyboard = Some(active);
    }
    ensure!(
        !stopped.load(Ordering::Acquire),
        "Keyboard preparation cancelled during shutdown"
    );
    // Keyboard::new includes its registration grace period. Only announce ready
    // afterwards, before the FIFO worker publishes its preparation report.
    ready.store(true, Ordering::Release);
    Ok(())
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
#[path = "system_actions/focused_clipboard_test.rs"]
mod focused_clipboard_test;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_text_is_empty_for_enter_only_but_invalid_text_is_not_coerced() {
        assert_eq!(
            plan(TEXT, &json!({"isSendingEnter":true})).unwrap(),
            plan(TEXT, &json!({"pastedText":"","isSendingEnter":true})).unwrap()
        );
        assert!(plan(TEXT, &json!({})).is_err());
        assert!(plan(TEXT, &json!({"pastedText":null,"isSendingEnter":true})).is_err());
        assert!(plan(TEXT, &json!({"pastedText":7,"isSendingEnter":true})).is_err());
    }
    #[test]
    fn full_typing_queue_rejects_text_without_stopping_keyboard_but_lost_release_still_cancels() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (_, reports) = mpsc::channel();
        let worker = Worker {
            sender,
            reports,
            stop: Arc::new(AtomicBool::new(false)),
            keyboard_ready: Arc::new(AtomicBool::new(false)),
            thread: None,
            typing_status: crate::system_typing::Shared::default(),
            command_results: crate::system_command::capture::Records::default(),
        };
        let text = || Plan::Type {
            text: "test".into(),
            enter: false,
        };
        worker.submit("first", text()).unwrap();
        assert!(
            worker
                .submit("overflow", text())
                .unwrap_err()
                .to_string()
                .contains("Typing queue")
        );
        assert!(!worker.stop.load(Ordering::Acquire));
        assert_eq!(receiver.recv().unwrap().context, "first");
        worker.submit("release", Plan::Release).unwrap();
        assert!(worker.submit("lost-release", Plan::Release).is_err());
        assert!(worker.stop.load(Ordering::Acquire));
    }
    #[test]
    fn worker_services_dial_deadlines_cancellation_and_shutdown_without_desktop_input() {
        for cancel in [true, false] {
            let file = tempfile::tempfile().unwrap();
            let worker = Worker::start(Some(crate::system_keyboard::Keyboard::with_test_writer(
                file.try_clone().unwrap(),
            )));
            // A long interval allows deterministic cancellation before the next stroke.
            worker
                .submit(
                    "device-a/dial",
                    Plan::Dial {
                        stroke: crate::system_dial::Stroke {
                            keys: vec![194].into(),
                            gesture: 0,
                            interval: Duration::from_secs(5),
                        },
                        count: 127,
                        coalesce: true,
                    },
                )
                .unwrap();
            assert!(
                worker
                    .reports
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .result
                    .is_ok()
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while file.metadata().unwrap().len() == 0 {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            let frame = (2 * std::mem::size_of::<libc::input_event>()) as u64;
            assert_eq!(file.metadata().unwrap().len(), frame);
            worker
                .submit(
                    "",
                    Plan::CancelDials(HashSet::from(["device-b/dial".into()])),
                )
                .unwrap();
            assert!(
                worker
                    .reports
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .result
                    .is_ok()
            );
            assert_eq!(file.metadata().unwrap().len(), frame);
            if cancel {
                worker
                    .submit(
                        "",
                        Plan::CancelDials(HashSet::from(["device-a/dial".into()])),
                    )
                    .unwrap();
                assert!(
                    worker
                        .reports
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap()
                        .result
                        .is_ok()
                );
                assert_eq!(file.metadata().unwrap().len(), frame * 2);
            }
            drop(worker);
            assert_eq!(file.metadata().unwrap().len(), frame * 2);
        }
    }
    #[test]
    fn dial_queue_writes_paced_key_events_and_preserves_another_hold() {
        use std::{
            io::{Read, Seek, SeekFrom},
            mem::{offset_of, size_of},
        };
        let mut file = tempfile::tempfile().unwrap();
        let mut keyboard =
            crate::system_keyboard::Keyboard::with_test_writer(file.try_clone().unwrap());
        let physical = keyboard.press(&[29]).unwrap();
        let mut dials = crate::system_dial::Dials::default();
        let mut holds = HashMap::new();
        let now = Instant::now();
        dials.enqueue(
            "a",
            crate::system_dial::Stroke {
                keys: vec![29, 30].into(),
                gesture: 0,
                interval: Duration::from_millis(50),
            },
            3,
            true,
            now,
        );
        emit_dial_events(dials.poll(now, true), &mut keyboard, &mut holds).unwrap();
        assert!(dials.poll(now + Duration::from_millis(49), true).is_empty());
        emit_dial_events(
            dials.cancel(&HashSet::from(["a".into()])),
            &mut keyboard,
            &mut holds,
        )
        .unwrap();
        assert!(holds.is_empty());
        assert!(dials.poll(now + Duration::from_secs(1), true).is_empty());
        keyboard.release(physical).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let events: Vec<_> = bytes
            .chunks_exact(size_of::<libc::input_event>())
            .filter_map(|e| {
                let field = |at| u16::from_ne_bytes(e[at..at + 2].try_into().unwrap());
                (field(offset_of!(libc::input_event, type_)) == 1).then(|| {
                    (
                        field(offset_of!(libc::input_event, code)),
                        i32::from_ne_bytes(
                            e[offset_of!(libc::input_event, value)..][..4]
                                .try_into()
                                .unwrap(),
                        ),
                    )
                })
            })
            .collect();
        assert_eq!(events, [(29, 1), (30, 1), (30, 0), (29, 0)]);
    }
    #[test]
    fn dial_modifier_timing_coalesce_default_and_invalid_values() {
        let mut settings = json!({"Hotkeys":[{"QTKeyCode":65,"KeyModifiers":2},{"QTKeyCode":66},{"QTKeyCode":67}], "Coalesce":false});
        assert!(
            matches!(input_plan(HOTKEY, "Encoder", &settings, Trigger::Rotate(-128)).unwrap(),
            Some(Plan::Dial { stroke: crate::system_dial::Stroke { interval, .. }, count:128, coalesce:false }) if interval == Duration::from_millis(50))
        );
        settings["Coalesce"] = json!("bad");
        assert!(input_plan(HOTKEY, "Encoder", &settings, Trigger::Tap).is_err());
        // Release must remain possible even after malformed settings are imported.
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Up).unwrap(),
            Some(Plan::Release)
        );
    }
    #[test]
    fn dial_timing_uses_native_legacy_marker_and_modern_mask_precedence() {
        for (mask, millis) in [(0, 30), (8192, 50)] {
            let settings = json!({"Hotkeys":[{"QTKeyCode":65,"KeyModifiers":mask,"KeyCtrl":true}]});
            let Some(Plan::Dial { stroke, .. }) =
                input_plan(HOTKEY, "Encoder", &settings, Trigger::Rotate(-1)).unwrap()
            else {
                panic!("Expected a dial stroke")
            };
            assert_eq!(stroke.interval, Duration::from_millis(millis));
            assert_eq!(
                stroke.keys.keys,
                if mask == 0 { vec![30] } else { vec![29, 30] }
            );
        }
    }
    #[test]
    fn fresh_clipboard_defaults_follow_layout_without_changing_legacy_text_or_custom_chords() {
        assert_eq!(defaults(TEXT)["linuxKeyMode"], "logical");
        assert_eq!(
            defaults(crate::system_clipboard::UUID)["linuxKeyMode"],
            "logical"
        );
        for (settings, logical) in [
            (json!({"pastedText":"hello","isTypingMode":false}), false),
            (
                json!({"pastedText":"hello","isTypingMode":false,"linuxKeyMode":"logical"}),
                true,
            ),
            (
                json!({"pastedText":"hello","isTypingMode":false,"linuxKeyMode":"logical","Hotkey":{"QTKeyCode":86,"KeyModifiers":2}}),
                false,
            ),
            (
                json!({"pastedText":"hello","isTypingMode":false,"linuxKeyMode":"logical","Hotkey":{"linuxKeyMode":"physical"}}),
                false,
            ),
        ] {
            let Plan::Paste { shortcut, .. } = plan(TEXT, &settings).unwrap() else {
                panic!("Expected paste")
            };
            assert_eq!(shortcut.logical, logical);
            assert_eq!(shortcut.keys, [29, 47]);
        }
        assert!(matches!(plan(TEXT,&json!({"pastedText":"hello","isTypingMode":true,"linuxKeyMode":null,"Hotkey":"dormant"})).unwrap(),Plan::Type{..}));
    }
    #[test]
    fn preparation_is_the_only_plan_classified_as_a_readiness_report() {
        assert!(Plan::PrepareKeyboard.prepares_keyboard());
        for plan in [
            Plan::Release,
            Plan::Chord(vec![].into()),
            Plan::Press(vec![].into()),
            Plan::Repeat(vec![].into(), 0),
            Plan::Get("https://example.invalid".into()),
            Plan::Launch {
                program: PathBuf::from("unused"),
                args: vec![],
            },
        ] {
            assert!(!plan.prepares_keyboard());
        }
    }
    #[test]
    fn lazy_worker_and_empty_keyboard_jobs_never_become_ready() {
        let worker = Worker::new();
        assert!(!worker.keyboard_ready());
        for plan in [
            Plan::Chord(vec![].into()),
            Plan::Press(vec![].into()),
            Plan::Repeat(vec![].into(), 3),
            Plan::Repeat(vec![194].into(), 0),
            Plan::Release,
        ] {
            worker.submit("empty", plan).unwrap();
            let report = worker.reports.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(report.result.is_ok());
            assert!(!report.prepared_keyboard);
            assert!(!worker.keyboard_ready());
        }
        let ready = worker.keyboard_ready.clone();
        drop(worker);
        assert!(!ready.load(Ordering::Acquire));
    }
    #[test]
    fn cancellation_clears_readiness_immediately() {
        let worker = Worker::new();
        // Model the flag only; this test never creates a uinput device.
        worker.keyboard_ready.store(true, Ordering::Release);
        worker.cancel();
        assert!(!worker.keyboard_ready.load(Ordering::Acquire));
        assert!(!worker.keyboard_ready());
    }
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
            Some(Plan::Dial {
                stroke: crate::system_dial::Stroke {
                    keys: vec![30].into(),
                    gesture: 0,
                    interval: Duration::from_millis(30)
                },
                count: 3,
                coalesce: true
            })
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Rotate(2)).unwrap(),
            Some(Plan::Dial {
                stroke: crate::system_dial::Stroke {
                    keys: vec![48].into(),
                    gesture: 1,
                    interval: Duration::from_millis(30)
                },
                count: 2,
                coalesce: true
            })
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Down).unwrap(),
            Some(Plan::Press(vec![46].into()))
        );
        assert_eq!(
            input_plan(HOTKEY, "Encoder", &settings, Trigger::Tap).unwrap(),
            Some(Plan::Dial {
                stroke: crate::system_dial::Stroke {
                    keys: vec![46].into(),
                    gesture: 2,
                    interval: Duration::from_millis(30)
                },
                count: 1,
                coalesce: true
            })
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
            Some(Plan::Repeat(vec![114].into(), 2))
        );
        assert_eq!(
            input_plan(
                MULTIMEDIA,
                "Encoder",
                &json!({"actionIdx":18}),
                Trigger::Tap
            )
            .unwrap(),
            Some(Plan::Chord(vec![113].into()))
        );
    }
    #[test]
    fn unknown_media_enums_are_not_guessed() {
        assert!(plan(MULTIMEDIA, &json!({"actionIdx":999})).is_err());
        assert_eq!(
            plan(MULTIMEDIA, &json!({"actionIdx":4})).unwrap(),
            Plan::Chord(vec![113].into())
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
#[test]
fn new_system_actions_preserve_unicode_and_select_switch_state() {
    let settings = json!({"pastedText":"Hei 🌍\næøå","isSendingEnter":true,"isTypingMode":false});
    assert_eq!(
        plan(TEXT, &settings).unwrap(),
        Plan::Paste {
            text: "Hei 🌍\næøå".into(),
            enter: true,
            shortcut: vec![29, 47].into()
        }
    );
    assert_eq!(
        plan(TEXT, &json!({"pastedText":"abc","isTypingMode":true})).unwrap(),
        Plan::Type {
            text: "abc".into(),
            enter: false
        }
    );
    assert!(plan(TEXT, &json!({"pastedText":"\u{0}"})).is_err());
    let settings =
        json!({"Hotkeys":[{"QTKeyCode":65,"KeyModifiers":0},{"QTKeyCode":66,"KeyModifiers":0}]});
    assert_eq!(
        input_plan_state(HOTKEY_SWITCH, "Keypad", &settings, Trigger::Down, 0).unwrap(),
        Some(Plan::Press(vec![30].into()))
    );
    assert_eq!(
        input_plan_state(HOTKEY_SWITCH, "Keypad", &settings, Trigger::Down, 1).unwrap(),
        Some(Plan::Press(vec![48].into()))
    );
    assert_eq!(
        input_plan_state(HOTKEY_SWITCH, "Keypad", &json!({}), Trigger::Up, 1).unwrap(),
        Some(Plan::Release)
    );
}
