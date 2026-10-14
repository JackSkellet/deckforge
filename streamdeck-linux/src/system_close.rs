//! Window-close requests with an explicit process-signal alternative.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Method {
    Automatic,
    Windows,
    Terminate,
}
#[derive(Debug, PartialEq)]
pub struct Config {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub force: bool,
    pub method: Method,
}
impl Config {
    pub fn parse(settings: &Value) -> Result<Self> {
        let path = settings["path"]
            .as_str()
            .context("Choose an application executable")?
            .trim();
        ensure!(
            !path.is_empty() && !path.contains('\0'),
            "Choose an application executable"
        );
        let executable = crate::system_actions::find_program(path)?.canonicalize()?;
        let force = settings
            .get("forceQuit")
            .map(|v| v.as_bool().context("Force close must be boolean"))
            .transpose()?
            .unwrap_or(false);
        let method = if force {
            Method::Terminate
        } else {
            match settings
                .get("linuxCloseMethod")
                .and_then(Value::as_str)
                .unwrap_or(if settings.get("linuxCloseMethod").is_none() {
                    "automatic"
                } else {
                    "invalid"
                }) {
                "automatic" => Method::Automatic,
                "windows" => Method::Windows,
                "terminate" => Method::Terminate,
                _ => anyhow::bail!("Choose a supported close behavior"),
            }
        };
        let args = settings
            .get("linuxMatchArgs")
            .map(|v| v.as_str().context("Match arguments must be text"))
            .transpose()?
            .unwrap_or("");
        ensure!(
            args.len() <= 65536 && !args.contains('\0'),
            "Match arguments must be at most 64 KiB without NUL"
        );
        let args = if args.trim().is_empty() {
            vec![]
        } else {
            gtk::glib::shell_parse_argv(args)
                .context("Invalid argument quoting")?
                .into_iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect()
        };
        Ok(Self {
            executable,
            args,
            force,
            method,
        })
    }
    pub fn execute(&self) -> Result<String> {
        close(&self.executable, &self.args, self.force, self.method)
    }
}
pub fn close(executable: &Path, args: &[String], force: bool, method: Method) -> Result<String> {
    let processes = crate::system_process::pinned_matches(executable, args)?;
    if processes.is_empty() {
        return Ok("Nothing to close; no matching processes".into());
    }
    if !force && method != Method::Terminate {
        let result = crate::system_app_window::close_process_windows(&processes).context(
            "Desktop window close unavailable; choose Send SIGTERM explicitly if needed",
        )?;
        if result.found > 0 {
            ensure!(
                result.requested > 0,
                "Matching windows changed or do not support normal closing"
            );
            return Ok(format!(
                "Window close requested for {} of {} matching window(s); the application may ask to save",
                result.requested, result.found
            ));
        }
        ensure!(
            method != Method::Windows,
            "No matching application windows found"
        );
    }
    let mut count = 0;
    for process in processes {
        count += usize::from(process.signal(force)?);
    }
    ensure!(count > 0, "Matching processes have already exited");
    Ok(format!(
        "{} requested for {count} process(es)",
        if force { "Force close" } else { "SIGTERM" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn imports_native_force_flag_and_literal_linux_matching_options() {
        let config =
            Config::parse(&json!({"path":"/bin/sleep","linuxMatchArgs":"'two words' '$VALUE'"}))
                .unwrap();
        assert_eq!(config.method, Method::Automatic);
        assert!(!config.force);
        assert_eq!(config.args, ["two words", "$VALUE"]);
        assert!(Config::parse(&json!({"path":"/bin/sleep","linuxCloseMethod":"unknown"})).is_err());
        assert!(
            Config::parse(&json!({"path":"/bin/sleep","linuxMatchArgs":"'unterminated"})).is_err()
        );
        assert!(Config::parse(&json!({"path":"/bin/sleep","forceQuit":"yes"})).is_err());
        assert!(
            Config::parse(
                &json!({"path":"/bin/sleep","forceQuit":true,"linuxCloseMethod":"future"})
            )
            .unwrap()
            .force
        );
    }
}
