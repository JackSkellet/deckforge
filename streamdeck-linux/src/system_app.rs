//! Open Application settings and Linux desktop launchers. No shell execution.
use anyhow::{Context, Result, ensure};
use gtk::{
    gio::{self, prelude::*},
    glib,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub const UUID: &str = "com.elgato.streamdeck.system.openapp";
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation {
    Open,
    Close,
    ForceClose,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub executable: PathBuf,
    pub desktop: Option<PathBuf>,
    pub args: Vec<String>,
    pub process_args: Vec<String>,
    pub bring_to_front: bool,
    pub hold: Option<Operation>,
}
#[derive(Serialize)]
pub struct Application {
    id: String,
    window_class: Option<String>,
    name: String,
    desktop: String,
    executable: String,
}
pub fn catalog() -> Vec<Application> {
    let mut entries: Vec<_> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| app.should_show())
        .filter_map(|app| {
            let info = app.downcast_ref::<gio::DesktopAppInfo>()?;
            let desktop = info.filename()?;
            Some(Application {
                window_class: info.startup_wm_class().map(String::from),
                id: app
                    .id()
                    .map(|id| id.strip_suffix(".desktop").unwrap_or(&id).to_owned())
                    .unwrap_or_default(),
                name: app.display_name().into(),
                desktop: desktop.to_string_lossy().into_owned(),
                executable: desktop_command(&app)
                    .map(|(exe, _)| exe)
                    .unwrap_or_default(),
            })
        })
        .collect();
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.desktop.clone()));
    entries
}
/// GIO's get_executable splits at the first space, even inside quoted paths.
/// Parse the complete Exec command and retain only fixed argument prefixes.
pub fn desktop_command(app: &gio::AppInfo) -> Result<(String, Vec<String>)> {
    let command = app
        .commandline()
        .context("Desktop application has no launch command")?;
    let parts = glib::shell_parse_argv(command.as_os_str())?;
    let executable = parts
        .first()
        .and_then(|s| literal_exec_token(&s.to_string_lossy()))
        .context("Desktop executable contains unsupported field codes")?;
    let args = parts
        .iter()
        .skip(1)
        .map_while(|s| literal_exec_token(&s.to_string_lossy()))
        .collect();
    Ok((executable, args))
}
fn literal_exec_token(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '%' && chars.next()? != '%' {
            return None;
        }
        result.push(c);
    }
    Some(result)
}

/// Resolve only a visible installed application. GTK theme lookup must run on
/// the native UI thread; image file decoding is left to the editor worker.
pub fn icon_path(desktop: &str) -> Result<Option<PathBuf>> {
    use gtk::prelude::IconThemeExt;
    ensure!(
        desktop.len() <= 4096 && Path::new(desktop).is_absolute(),
        "Invalid desktop application path"
    );
    let app = gio::AppInfo::all()
        .into_iter()
        .find(|app| {
            app.should_show()
                && app
                    .downcast_ref::<gio::DesktopAppInfo>()
                    .and_then(|info| info.filename())
                    .as_deref()
                    == Some(Path::new(desktop))
        })
        .context("Installed application is unavailable")?;
    let Some(icon) = app.icon() else {
        return Ok(None);
    };
    if let Some(file) = icon.downcast_ref::<gio::FileIcon>() {
        return Ok(file.file().path());
    }
    Ok(gtk::IconTheme::default()
        .and_then(|theme| theme.lookup_by_gicon(&icon, 64, gtk::IconLookupFlags::FORCE_SIZE))
        .and_then(|info| info.filename()))
}

pub fn icon_png(path: &Path) -> Result<String> {
    use base64::Engine;
    // Theme icons commonly use symlinks. Resolve the declared icon first, then
    // use the same regular-file, decode and external-SVG limits as action icons.
    let path = path.canonicalize()?;
    let bytes = crate::action_icons::png(
        path.parent().context("Icon directory missing")?,
        path.file_name()
            .and_then(|s| s.to_str())
            .context("Invalid icon filename")?,
    )?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
impl Config {
    pub fn parse(settings: &Value) -> Result<Self> {
        let executable = settings["exec"]
            .as_str()
            .context("Choose a Linux application")?;
        ensure!(
            !executable.is_empty() && !executable.contains('\0'),
            "Choose a Linux application"
        );
        ensure!(
            !executable.contains('\\') && executable.as_bytes().get(1) != Some(&b':'),
            "This application is for Windows; choose its Linux replacement"
        );
        let desktop = settings
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| Path::new(s).is_absolute() && s.ends_with(".desktop"))
            .map(PathBuf::from);
        let mut process_args = Vec::new();
        let executable = if let Some(path) = &desktop {
            let app = gio::DesktopAppInfo::from_filename(path)
                .context("The selected desktop application is unavailable")?;
            let (executable, fixed_args) = desktop_command(&app.upcast())?;
            process_args = fixed_args;
            crate::system_actions::find_program(&executable)?
        } else {
            crate::system_actions::find_program(executable)?
        };
        let args = settings
            .get("args")
            .map(|v| v.as_str().context("Arguments must be text"))
            .transpose()?
            .unwrap_or("");
        ensure!(
            args.len() <= 65536 && !args.contains('\0'),
            "Application arguments are too large or contain NUL"
        );
        let args = if args.trim().is_empty() {
            vec![]
        } else {
            glib::shell_parse_argv(args)?
                .into_iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect()
        };
        if desktop.is_none() {
            // Manual interpreter arguments identify the script, not just Python/Node.
            process_args = args.clone();
        }
        let bring_to_front = settings
            .get("bring_to_front")
            .map(|v| v.as_bool().context("Bring to front must be boolean"))
            .transpose()?
            .unwrap_or(true);
        let hold = match settings
            .get("long_press")
            .map(|v| v.as_str().context("Held-key action must be text"))
            .transpose()?
            .unwrap_or("quit")
        {
            "quit" => Some(Operation::Close),
            "force_kill" => Some(Operation::ForceClose),
            "none" => None,
            _ => anyhow::bail!("Choose a supported held-key action"),
        };
        Ok(Self {
            executable,
            desktop,
            args,
            process_args,
            bring_to_front,
            hold,
        })
    }
    fn desktop_info(&self, path: &Path) -> Result<gio::DesktopAppInfo> {
        if self.args.is_empty() {
            return gio::DesktopAppInfo::from_filename(path)
                .context("Application launcher unavailable");
        }
        let file = glib::KeyFile::new();
        file.load_from_file(path, glib::KeyFileFlags::KEEP_TRANSLATIONS)?;
        let exec = file.string("Desktop Entry", "Exec")?;
        // GIO has no arbitrary-argument launch API. Keep the launcher's settings,
        // append escaped literal argv, and use Exec for this explicit invocation.
        let mut expanded = String::new();
        let mut chars = exec.chars();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('k') => expanded.push_str(&quote_exec(&path.to_string_lossy())),
                    Some(other) => {
                        expanded.push('%');
                        expanded.push(other);
                    }
                    None => expanded.push('%'),
                }
            } else {
                expanded.push(c);
            }
        }
        for arg in &self.args {
            expanded.push(' ');
            expanded.push_str(&quote_exec(arg));
        }
        file.set_string("Desktop Entry", "Exec", &expanded);
        file.set_boolean("Desktop Entry", "DBusActivatable", false);
        gio::DesktopAppInfo::from_keyfile(&file)
            .context("Application launcher arguments are invalid")
    }
    pub fn execute(&self, operation: Operation) -> Result<(String, Option<Child>)> {
        match operation {
            Operation::ForceClose => {
                return Ok((
                    crate::system_close::close(
                        &self.executable,
                        &self.process_args,
                        true,
                        crate::system_close::Method::Terminate,
                    )?,
                    None,
                ));
            }
            Operation::Close => {
                let message = crate::system_close::close(
                    &self.executable,
                    &self.process_args,
                    false,
                    crate::system_close::Method::Automatic,
                )?;
                return Ok((message, None));
            }
            Operation::Open => {}
        }
        if self.bring_to_front {
            let processes =
                crate::system_process::pinned_for_activation(&self.executable, &self.process_args)?;
            match crate::system_app_window::activate(&processes) {
                Ok(count) if count > 0 => {
                    return Ok(("Running application brought to front".into(), None));
                }
                Ok(_) => {} // A background process without a window still needs a launch.
                Err(error) => {
                    if let Some(path) = &self.desktop {
                        let app = gio::DesktopAppInfo::from_filename(path)
                            .context("Application launcher unavailable")?;
                        if app.boolean("DBusActivatable") && self.args.is_empty() {
                            app.launch(&[], None::<&gio::AppLaunchContext>)?;
                            return Ok(("Application activation requested".into(), None));
                        }
                    }
                    return Err(error).context("Cannot bring the running application to front; disable Bring to front to launch another instance");
                }
            }
        }
        if let Some(path) = &self.desktop {
            self.desktop_info(path)?
                .launch(&[], None::<&gio::AppLaunchContext>)?;
            Ok(("Desktop application launch requested".into(), None))
        } else {
            let child = Command::new(&self.executable)
                .args(&self.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            Ok(("Application launch requested".into(), Some(child)))
        }
    }
}
fn quote_exec(value: &str) -> String {
    let mut result = String::from("\"");
    for c in value.chars() {
        match c {
            '%' => result.push_str("%%"),
            '\\' | '"' | '`' | '$' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}

/// Desktop IPC may wait for a compositor. Keep it off the keyboard's worker so
/// an unresponsive desktop cannot delay a held key's release.
enum Job {
    Application(Config, Operation),
    Close(crate::system_close::Config),
}
pub struct Worker {
    sender: std::sync::mpsc::SyncSender<(String, Job)>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Worker {
    pub fn new(reports: std::sync::mpsc::Sender<crate::system_actions::Report>) -> Self {
        use std::{
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
                mpsc,
            },
            time::Duration,
        };
        let (sender, receiver) = mpsc::sync_channel::<(String, Job)>(32);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut children: Vec<Child> = Vec::new();
            while !stopped.load(Ordering::Acquire) {
                children.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
                let (context, job) = match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let result = (|| {
                    let (message, child) = match job {
                        Job::Application(config, operation) => {
                            ensure!(
                                operation != Operation::Open || children.len() < 32,
                                "Too many applications are still running"
                            );
                            config.execute(operation)?
                        }
                        Job::Close(config) => (config.execute()?, None),
                    };
                    if let Some(child) = child {
                        children.push(child);
                    }
                    Ok(message)
                })();
                let _ = reports.send(crate::system_actions::Report {
                    context,
                    result,
                    prepared_keyboard: false,
                });
            }
            // User-launched applications outlive Deckforge; reap without closing them.
            for mut child in children {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
        });
        Self {
            sender,
            thread: Some(thread),
            stop,
        }
    }
    pub fn submit(&self, context: String, config: Config, operation: Operation) -> Result<()> {
        self.sender
            .try_send((context, Job::Application(config, operation)))
            .map_err(|_| anyhow::anyhow!("Application queue is full or stopped"))
    }
    pub fn submit_close(&self, context: String, config: crate::system_close::Config) -> Result<()> {
        self.sender
            .try_send((context, Job::Close(config)))
            .map_err(|_| anyhow::anyhow!("Application queue is full or stopped"))
    }
    pub fn cancel(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
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
    use serde_json::json;
    #[test]
    fn original_hold_modes_and_foreign_targets_are_explicit() {
        let manual =
            Config::parse(&json!({"exec":"/usr/bin/true","args":"'owned script' '$HOME'"}))
                .unwrap();
        assert_eq!(manual.process_args, ["owned script", "$HOME"]);
        for (value, expected) in [
            ("none", None),
            ("quit", Some(Operation::Close)),
            ("force_kill", Some(Operation::ForceClose)),
        ] {
            assert_eq!(
                Config::parse(&json!({"exec":"/usr/bin/true","long_press":value}))
                    .unwrap()
                    .hold,
                expected
            );
        }
        for settings in [
            json!({"exec":"C:/Windows/app.exe"}),
            json!({"exec":"/usr/bin/true","long_press":"force_quit"}),
            json!({"exec":"/usr/bin/true","bring_to_front":"yes"}),
        ] {
            assert!(Config::parse(&settings).is_err());
        }
        let c = Config::parse(&json!({"exec":std::env::current_exe().unwrap()})).unwrap();
        assert!(
            c.execute(Operation::Close)
                .unwrap_err()
                .to_string()
                .contains("Quit")
        );
    }
    #[test]
    fn arguments_are_literals_and_desktop_launch_preserves_its_working_directory() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("owned python");
        std::fs::copy("/usr/bin/python3", &executable).unwrap();
        let script = root.path().join("capture.py");
        let output = root.path().join("argv.json");
        std::fs::write(&script,"import os,sys,json\nopen(sys.argv[1],'w').write(json.dumps({'argv':sys.argv[2:],'cwd':os.getcwd()}))\n").unwrap();
        let desktop = root.path().join("owned launcher.desktop");
        std::fs::write(&desktop,format!("[Desktop Entry]\nType=Application\nName=Owned fixture\nExec=\"{}\" \"{}\" \"{}\" %k %% %U\nPath={}\n",executable.display(),script.display(),output.display(),root.path().display())).unwrap();
        let args = "'space argument' 'æøå' '$(touch forbidden)' '$HOME' '100%' 'quote\"'";
        let config = Config::parse(
            &json!({"exec":"/usr/bin/python3","source":desktop,"args":args,"bring_to_front":false}),
        )
        .unwrap();
        assert_eq!(
            config.process_args,
            [script.to_string_lossy(), output.to_string_lossy()]
        );
        config.execute(Operation::Open).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !output.exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let result: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(result["cwd"], root.path().to_str().unwrap());
        assert_eq!(
            result["argv"],
            json!([
                desktop.to_str().unwrap(),
                "%",
                "space argument",
                "æøå",
                "$(touch forbidden)",
                "$HOME",
                "100%",
                "quote\""
            ])
        );
        assert!(!root.path().join("forbidden").exists());
    }
}
