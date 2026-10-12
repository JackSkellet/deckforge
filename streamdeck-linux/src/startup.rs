//! Per-user XDG login startup. Reading a preference never writes a desktop entry.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

const MARKER: &str = "X-Deckforge-Managed";
#[derive(Debug, Default, serde::Serialize)]
pub struct Status {
    pub enabled: bool,
    pub this_session: bool,
}
#[derive(Clone)]
pub struct Startup {
    entry: PathBuf,
    executable: PathBuf,
    state: PathBuf,
    simulate: bool,
}
impl Startup {
    pub fn current(state: &Path, simulate: bool) -> Result<Self> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .map(|home| home.join(".config"))
            })
            .context("No absolute user configuration directory is available")?;
        Ok(Self {
            entry: config.join("autostart/deckforge.desktop"),
            executable: std::env::current_exe().context("Locate Deckforge executable")?,
            state: match state.canonicalize() {
                Ok(path) => path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::path::absolute(state)?
                }
                Err(error) => return Err(error).context("Locate session directory"),
            },
            simulate,
        })
    }
    pub fn path(&self) -> &Path {
        &self.entry
    }
    fn identity(&self) -> String {
        format!("{:x}", Sha256::digest(self.state.as_os_str().as_bytes()))
    }
    pub fn status(&self) -> Result<Status> {
        let Some(content) = read_entry(&self.entry)? else {
            return Ok(Status::default());
        };
        let values = fields(&content)?;
        ensure!(
            values.get(MARKER) == Some(&"true"),
            "{} is a custom startup entry; Deckforge will not overwrite it",
            self.entry.display()
        );
        Ok(Status {
            enabled: values.get("Hidden") != Some(&"true"),
            this_session: values.get("X-Deckforge-State").copied()
                == Some(self.identity().as_str())
                && values.get("X-Deckforge-Simulate").copied()
                    == Some(if self.simulate { "true" } else { "false" }),
        })
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<Status> {
        self.status()?; // Refuse foreign files and symlinks, including when disabling.
        let content = self.content(enabled)?;
        let parent = self.entry.parent().context("Startup entry has no parent")?;
        fs::create_dir_all(parent).context("Create user startup directory")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        // Revalidate after preparing the replacement so a foreign edit is not
        // silently replaced by a stale UI preference.
        self.status()?;
        temporary
            .persist(&self.entry)
            .context("Save login startup preference")?;
        self.status()
    }
    fn content(&self, enabled: bool) -> Result<String> {
        // A disabled per-user entry also masks a system-wide entry with the same
        // name. Removing the file could unexpectedly re-enable login startup.
        if !enabled {
            return Ok(format!(
                "[Desktop Entry]\nType=Application\nName=Deckforge\nHidden=true\n{MARKER}=true\n"
            ));
        }
        ensure!(
            self.executable.is_file(),
            "Install or restart Deckforge before enabling login startup"
        );
        let executable = path_text(&self.executable)?;
        ensure!(
            !executable.contains('='),
            "The executable path cannot contain '=' in a desktop entry"
        );
        ensure!(
            self.state.is_dir(),
            "Open this session before enabling login startup"
        );
        let state = path_text(&self.state)?;
        // GIO resolves the executable before expanding %% field codes. Keep
        // a literal-percent path in an argument to env, which then execs it.
        // No shell or command string evaluation is involved.
        let program = if executable.contains('%') {
            ensure!(
                Path::new("/usr/bin/env").is_file(),
                "This executable path requires /usr/bin/env for login startup"
            );
            format!("/usr/bin/env -- {}", quote(executable)?)
        } else {
            quote(executable)?
        };
        let exec = format!(
            "{} desktop --background --state {}{}",
            program,
            quote(state)?,
            if self.simulate { " --simulate" } else { "" }
        );
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName=Deckforge\nComment=Keep Stream Deck controls available after login\nExec={exec}\nIcon=deckforge\nTerminal=false\nHidden=false\n{MARKER}=true\nX-Deckforge-State={}\nX-Deckforge-Simulate={}\n",
            self.identity(),
            self.simulate
        ))
    }
}
fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("Login startup requires UTF-8 paths")
}
fn quote(value: &str) -> Result<String> {
    ensure!(
        !value.chars().any(char::is_control),
        "Login startup paths cannot contain control characters"
    );
    let mut output = String::from("\"");
    // Desktop Entry string decoding precedes Exec quoting and field expansion.
    // This is not a shell command. Literal % is escaped for the launcher.
    for c in value.chars() {
        if c == '%' {
            output.push_str("%%");
        } else {
            if "\\\"`$".contains(c) {
                output.push_str("\\\\");
            }
            if c == '\\' {
                output.push_str("\\\\");
            } else {
                output.push(c);
            }
        }
    }
    output.push('"');
    Ok(output)
}
fn read_entry(path: &Path) -> Result<Option<String>> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Read login startup entry"),
    };
    ensure!(
        file.metadata()?.is_file(),
        "Startup entry must be a regular file"
    );
    let mut text = String::new();
    Read::by_ref(&mut file)
        .take(65_537)
        .read_to_string(&mut text)?;
    ensure!(text.len() <= 65_536, "Startup entry exceeds 64 KiB");
    Ok(Some(text))
}
fn fields(content: &str) -> Result<HashMap<&str, &str>> {
    let mut values = HashMap::new();
    let mut entry = false;
    let mut found = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') {
            entry = line == "[Desktop Entry]";
            if entry {
                ensure!(!found, "Duplicate Desktop Entry section");
                found = true;
            }
        } else if entry && !line.is_empty() && !line.starts_with('#') {
            let (key, value) = line.split_once('=').context("Malformed startup entry")?;
            ensure!(
                values.insert(key.trim(), value.trim()).is_none(),
                "Duplicate startup entry key"
            );
        }
    }
    ensure!(found, "Startup entry has no Desktop Entry section");
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Startup {
        let executable = root.join("Deck forge % $ ` \\\" binary");
        fs::write(&executable, "fixture").unwrap();
        let state = root.join("state space=50%");
        fs::create_dir(&state).unwrap();
        Startup {
            entry: root.join("config/autostart/deckforge.desktop"),
            executable,
            state,
            simulate: true,
        }
    }
    #[test]
    fn enable_disable_and_read_preserve_user_state() {
        let dir = tempfile::tempdir().unwrap();
        let startup = fixture(dir.path());
        assert!(!startup.status().unwrap().enabled);
        assert!(!startup.entry.parent().unwrap().exists());
        let status = startup.set_enabled(true).unwrap();
        assert!(status.enabled && status.this_session);
        let content = fs::read_to_string(startup.path()).unwrap();
        assert!(content.contains(" --simulate\n"));
        assert!(content.contains(" --background --state "));
        let status = startup.set_enabled(false).unwrap();
        assert!(!status.enabled);
        assert!(
            fs::read_to_string(startup.path())
                .unwrap()
                .contains("Hidden=true")
        );
        assert!(startup.state.is_dir());
    }
    #[test]
    fn foreign_files_symlinks_and_oversized_entries_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let startup = fixture(dir.path());
        fs::create_dir_all(startup.entry.parent().unwrap()).unwrap();
        let content = "[Desktop Entry]\nName=My custom startup\nExec=custom\n";
        fs::write(startup.path(), content).unwrap();
        assert!(startup.set_enabled(true).is_err());
        assert!(startup.set_enabled(false).is_err());
        assert_eq!(fs::read_to_string(startup.path()).unwrap(), content);
        fs::remove_file(startup.path()).unwrap();
        std::os::unix::fs::symlink(&startup.executable, startup.path()).unwrap();
        assert!(startup.set_enabled(true).is_err());
        assert!(startup.path().is_symlink());
        fs::remove_file(startup.path()).unwrap();
        fs::write(startup.path(), "x".repeat(65_537)).unwrap();
        assert!(startup.status().is_err());
    }
    #[test]
    fn session_identity_and_simulation_are_distinguished() {
        let dir = tempfile::tempdir().unwrap();
        let startup = fixture(dir.path());
        startup.set_enabled(true).unwrap();
        let mut other = startup.clone();
        other.simulate = false;
        assert!(!other.status().unwrap().this_session);
        other.simulate = true;
        other.state = dir.path().to_owned();
        assert!(!other.status().unwrap().this_session);
    }
    #[test]
    fn quoted_exec_has_two_escaping_layers_and_literal_percent() {
        assert_eq!(
            quote("a\\b\"c$d`e%f").unwrap(),
            r#""a\\\\b\\"c\\$d\\`e%%f""#
        );
        assert!(quote("a\nb").is_err());
        assert!(quote("a\tb").is_err());
        assert!(fields("[Desktop Entry]\nHidden=true\nHidden=false\n").is_err());
    }
}
