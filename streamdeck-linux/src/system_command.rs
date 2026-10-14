//! Linux command actions. Shell interpretation is an explicit saved mode.
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde_json::Value;
use std::{path::PathBuf, process::Command};

pub const UUID: &str = "com.deckforge.system.command";
pub mod capture;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Output {
    Discard,
    Show,
    Clipboard,
}

#[derive(Debug, PartialEq)]
pub struct Config {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub directory: PathBuf,
    pub output: Output,
    pub timeout_seconds: u64,
    pub environment: Vec<(String, Option<String>)>,
}

fn text<'a>(settings: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    let value = settings
        .get(key)
        .map(|v| v.as_str().context("Command fields must be text"))
        .transpose()?
        .unwrap_or(default);
    ensure!(
        value.len() <= 65536 && !value.contains('\0'),
        "Command fields must be at most 64 KiB without NUL characters"
    );
    Ok(value)
}

fn arguments(value: &str) -> Result<Vec<String>> {
    if value.trim().is_empty() {
        return Ok(vec![]);
    }
    Ok(gtk::glib::shell_parse_argv(value)
        .context("Invalid argument quoting")?
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn environment(settings: &Value) -> Result<Vec<(String, Option<String>)>> {
    let Some(rows) = settings.get("environment") else {
        return Ok(vec![]);
    };
    let rows = rows
        .as_array()
        .context("Environment overrides must be a list")?;
    ensure!(
        rows.len() <= 64,
        "At most 64 environment overrides are supported"
    );
    let mut names = std::collections::HashSet::new();
    let mut bytes = 0usize;
    let mut result = Vec::new();
    for row in rows {
        ensure!(row.is_object(), "Environment override must be an object");
        let name = text(row, "name", "")?;
        ensure!(
            !name.is_empty()
                && name.len() <= 128
                && name.bytes().enumerate().all(|(i, b)| b == b'_'
                    || b.is_ascii_alphabetic()
                    || (i > 0 && b.is_ascii_digit())),
            "Environment names must start with a letter or underscore and contain only letters, digits or underscores"
        );
        ensure!(
            names.insert(name),
            "Environment variable {name} is listed more than once"
        );
        let value = text(row, "value", "")?;
        bytes += name.len() + value.len();
        ensure!(bytes <= 65536, "Environment overrides exceed 64 KiB");
        let unset = row
            .get("unset")
            .map(|v| {
                v.as_bool()
                    .context("Unset environment variable must be boolean")
            })
            .transpose()?
            .unwrap_or(false);
        result.push((name.into(), (!unset).then(|| value.to_owned())));
    }
    Ok(result)
}

fn terminal_payload(
    program: &std::path::Path,
    args: &[String],
    directory: &std::path::Path,
    environment: &[(String, Option<String>)],
) -> Result<String> {
    // Konsole expands $ENV even in separately supplied argv. Transport a quoted
    // exec command as base64, so a terminal cannot reinterpret saved arguments.
    // The only dollar in the transport is shell command substitution, not $ENV.
    let mut script = format!("cd {} && exec ", quote(&directory.to_string_lossy()));
    if !environment.is_empty() {
        let env_program = crate::system_actions::find_program("/usr/bin/env")?;
        script.push_str(&quote(&env_program.to_string_lossy()));
        for (name, _) in environment.iter().filter(|(_, value)| value.is_none()) {
            script.push_str(" -u ");
            script.push_str(&quote(name));
        }
        for (name, value) in environment {
            if let Some(value) = value {
                script.push(' ');
                script.push_str(&quote(&format!("{name}={value}")));
            }
        }
        script.push(' ');
        // env treats any following operand containing '=' as an assignment.
        // A zero-priority-adjustment exec wrapper disambiguates such filenames
        // without another shell rewriting target environment variables.
        if program.as_os_str().as_encoded_bytes().contains(&b'=') {
            let nice = crate::system_actions::find_program("/usr/bin/nice")?;
            script.push_str(&quote(&nice.to_string_lossy()));
            script.push_str(" -n 0 -- ");
        }
    }
    script.push_str(&quote(&program.to_string_lossy()));
    for arg in args {
        script.push(' ');
        script.push_str(&quote(arg));
    }
    ensure!(
        script.len() <= 65536,
        "Terminal command exceeds 64 KiB after quoting"
    );
    let decoder = crate::system_actions::find_program("/usr/bin/base64")?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(script);
    Ok(format!(
        "eval \"$(printf '%s' '{encoded}' | {} -d)\"",
        quote(&decoder.to_string_lossy())
    ))
}

impl Config {
    pub fn parse(settings: &Value) -> Result<Self> {
        let environment = environment(settings)?;
        let search_path = match environment.iter().find(|(name, _)| name == "PATH") {
            Some((_, Some(path))) => std::ffi::OsString::from(path),
            Some((_, None)) => std::ffi::OsString::from("/usr/bin:/bin"),
            None => std::env::var_os("PATH").unwrap_or_default(),
        };
        let (program, args) = match text(settings, "mode", "program")? {
            "program" => (
                crate::system_actions::find_program_in_path(
                    text(settings, "program", "")?,
                    &search_path,
                )?,
                arguments(text(settings, "args", "")?)?,
            ),
            "shell" => {
                let script = text(settings, "script", "")?;
                ensure!(!script.trim().is_empty(), "Enter a shell script");
                (
                    crate::system_actions::find_program("/bin/sh")?,
                    vec!["-c".into(), script.into(), "deckforge-command".into()],
                )
            }
            _ => anyhow::bail!("Unknown command mode"),
        };
        let directory = text(settings, "directory", "")?;
        let directory = if directory.is_empty() {
            PathBuf::from(std::env::var_os("HOME").context("Home directory is unavailable")?)
        } else {
            PathBuf::from(directory)
        };
        ensure!(
            directory.is_absolute() && directory.is_dir(),
            "Choose an existing absolute working directory"
        );
        let terminal = settings
            .get("terminal")
            .map(|v| v.as_bool().context("Run in terminal must be boolean"))
            .transpose()?
            .unwrap_or(false);
        let output = match text(settings, "output", "discard")? {
            "discard" => Output::Discard,
            "show" => Output::Show,
            "clipboard" => Output::Clipboard,
            _ => anyhow::bail!("Unknown command output mode"),
        };
        let timeout_seconds = settings
            .get("timeout_seconds")
            .map(|v| v.as_u64().context("Command timeout must be whole seconds"))
            .transpose()?
            .unwrap_or(30);
        ensure!(
            (1..=3600).contains(&timeout_seconds),
            "Command timeout must be between 1 and 3600 seconds"
        );
        if terminal {
            let launcher = crate::system_actions::find_program(text(
                settings,
                "terminal_program",
                "konsole",
            )?)?;
            let mut prefix = arguments(text(settings, "terminal_args", "--separate --hold -e")?)?;
            prefix.extend([
                "/bin/sh".into(),
                "-c".into(),
                terminal_payload(&program, &args, &directory, &environment)?,
            ]);
            Ok(Self {
                program: launcher,
                args: prefix,
                directory,
                output: Output::Discard,
                timeout_seconds,
                // Overrides belong to the target command, not its terminal GUI.
                environment: vec![],
            })
        } else {
            Ok(Self {
                program,
                args,
                directory,
                output,
                timeout_seconds,
                environment,
            })
        }
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).current_dir(&self.directory);
        for (name, value) in &self.environment {
            if let Some(value) = value {
                command.env(name, value);
            } else {
                command.env_remove(name);
            }
        }
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn environment_values_are_literal_and_terminal_transport_matches_background() {
        let value = "  spaces '$DECKFORGE_ENV_EXAMPLE' $(not-a-command)\n🌍  ";
        for terminal in [false, true] {
            let config = Config::parse(&json!({"program":"/usr/bin/printenv","args":"DECKFORGE_ENV_EXAMPLE", "terminal":terminal,"terminal_program":"/usr/bin/env","terminal_args":"", "environment":[{"name":"DECKFORGE_ENV_EXAMPLE","value":value},{"name":"DECKFORGE_ENV_EMPTY","value":""},{"name":"DECKFORGE_ENV_REMOVE","unset":true,"value":"retained draft"}]})).unwrap();
            let output = config.command().output().unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{value}\n")
            );
        }
        let config=Config::parse(&json!({"program":"/usr/bin/printenv","args":"DECKFORGE_ENV_EMPTY", "environment":[{"name":"DECKFORGE_ENV_EMPTY","value":""}]})).unwrap();
        assert_eq!(config.command().output().unwrap().stdout, b"\n");
    }

    #[test]
    fn custom_path_resolves_only_the_target_program_without_changing_host_environment() {
        let root = tempfile::tempdir().unwrap();
        let tool = root.path().join("deckforge-owned-env-reader");
        std::os::unix::fs::symlink("/usr/bin/printenv", &tool).unwrap();
        let before = std::env::var_os("PATH");
        let config=Config::parse(&json!({"program":"deckforge-owned-env-reader","args":"PATH", "environment":[{"name":"PATH","value":root.path().to_str().unwrap()}]})).unwrap();
        assert_eq!(config.program, tool);
        let output = config.command().output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim_end(),
            root.path().to_str().unwrap()
        );
        assert_eq!(std::env::var_os("PATH"), before);
        let config=Config::parse(&json!({"program":"printenv","args":"PATH", "environment":[{"name":"PATH","unset":true}]})).unwrap();
        assert!(!config.command().output().unwrap().status.success());
        assert!(
            Config::parse(
                &json!({"program":"printenv","environment":[{"name":"PATH","value":""}]})
            )
            .is_err()
        );
    }

    #[test]
    fn terminal_environment_handles_executable_paths_containing_equals() {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("program=with-equals");
        std::os::unix::fs::symlink("/usr/bin/printenv", &program).unwrap();
        let config=Config::parse(&json!({"program":program,"args":"DECKFORGE_ENV_EQUALS","terminal":true,"terminal_program":"/usr/bin/env","terminal_args":"","environment":[{"name":"DECKFORGE_ENV_EQUALS","value":"unchanged"}]})).unwrap();
        let output = config.command().output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"unchanged\n");
    }

    #[test]
    fn invalid_environment_and_duplicate_names_are_rejected_without_exposing_values() {
        for environment in [
            json!({}),
            json!([{"name":"2BAD","value":"private-value"}]),
            json!([{"name":"A=B"}]),
            json!([{"name":"VALID","value":"nul\u{0}"}]),
            json!([{"name":"X","unset":"yes"}]),
            json!([{"name":"X"},{"name":"X"}]),
            json!([{"name":"X","value":false}]),
            json!([{"name":"X","value":"x".repeat(65536)}]),
        ] {
            let error = Config::parse(&json!({"program":"/bin/true","environment":environment}))
                .unwrap_err()
                .to_string();
            assert!(!error.contains("private-value"));
        }
    }

    #[test]
    fn program_arguments_are_literal_and_directory_is_used() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::parse(&json!({"program":"/usr/bin/printf", "args":"'%s|%s' 'two words' '$(touch injected); $HOME'", "directory":root.path()})).unwrap();
        let output = config.command().output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "two words|$(touch injected); $HOME"
        );
        assert!(!root.path().join("injected").exists());
        let config =
            Config::parse(&json!({"program":"/bin/pwd", "directory":root.path()})).unwrap();
        assert_eq!(
            String::from_utf8(config.command().output().unwrap().stdout)
                .unwrap()
                .trim(),
            root.path().to_str().unwrap()
        );
    }

    #[test]
    fn explicit_shell_supports_pipes_and_redirection() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::parse(&json!({"mode":"shell", "script":"printf 'hello' | tr a-z A-Z > result", "directory":root.path()})).unwrap();
        assert!(!root.path().join("result").exists());
        assert!(config.command().status().unwrap().success());
        assert_eq!(
            std::fs::read_to_string(root.path().join("result")).unwrap(),
            "HELLO"
        );
    }

    #[test]
    fn terminal_receives_distinct_command_arguments() {
        let config = Config::parse(&json!({"program":"/usr/bin/printf", "args":"'%s|%s' 'two words' '$HOME'", "directory":"/tmp", "terminal":true, "terminal_program":"/usr/bin/env", "terminal_args":""})).unwrap();
        let output = config.command().output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "two words|$HOME");
        for settings in [
            json!({"mode":"unknown"}),
            json!({"mode":"shell","script":""}),
            json!({"program":"/bin/echo","directory":"relative"}),
            json!({"program":"/bin/echo","args":"'unterminated"}),
            json!({"program":"/bin/echo","terminal":"true"}),
            json!({"mode":"shell","script":"a\u{0}b"}),
        ] {
            assert!(Config::parse(&settings).is_err(), "{settings}");
        }
    }
}
