use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub token: Option<Token>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("DECKFORGE_PLUGIN_DATA") {
        return Ok(PathBuf::from(path).join("discord-auth.json"));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .context("No private Discord plugin data directory")?;
    Ok(base.join("deckforge-discord/discord-auth.json"))
}
fn safe_directory(path: &Path, create: bool) -> Result<()> {
    // Reject existing symlink ancestors before creating any child directory.
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) => ensure!(
                !m.file_type().is_symlink() && m.is_dir(),
                "Private config directory contains a symlink or non-directory"
            ),
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if create {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let m = fs::metadata(path)?;
    ensure!(
        m.uid() == unsafe { libc::geteuid() },
        "Private config directory has another owner"
    );
    if create {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    ensure!(
        fs::metadata(path)?.mode() & 0o077 == 0,
        "Private config directory must have mode 0700"
    );
    Ok(())
}
pub fn load(path: &Path) -> Result<Option<Config>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    safe_directory(
        path.parent().context("Private config has no directory")?,
        false,
    )?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = file.metadata()?;
    ensure!(
        m.is_file() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "Private Discord config must be an owned regular file with mode 0600"
    );
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 65536,
        "Private Discord config exceeds 64 KiB"
    );
    let config: Config =
        serde_json::from_slice(&bytes).context("Invalid private Discord config")?;
    validate(&config)?;
    Ok(Some(config))
}
pub fn save(path: &Path, config: &Config) -> Result<()> {
    validate(config)?;
    let parent = path.parent().context("Private config has no directory")?;
    safe_directory(parent, true)?;
    if let Ok(m) = fs::symlink_metadata(path) {
        ensure!(
            m.is_file() && !m.file_type().is_symlink(),
            "Private config destination is not a regular file"
        );
    }
    let temporary = parent.join(format!(".discord-auth-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(config)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn forget(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && parent.exists()
    {
        safe_directory(parent, false)?;
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub fn validate(config: &Config) -> Result<()> {
    ensure!(
        (16..=22).contains(&config.client_id.len())
            && config.client_id.bytes().all(|b| b.is_ascii_digit()),
        "Enter your Discord application's numeric client ID"
    );
    ensure!(
        !config.client_secret.trim().is_empty() && config.client_secret.len() <= 4096,
        "Enter your Discord application's client secret"
    );
    ensure!(
        config.scopes.len() <= 32
            && !config.scopes.is_empty()
            && config.scopes.iter().all(|s| s.len() <= 80
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_')),
        "Invalid Discord OAuth scopes"
    );
    ensure!(
        config.redirect_uri.len() <= 2048,
        "Redirect URI is too long"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_private_and_round_trip_outside_action_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/auth.json");
        let config = Config {
            client_id: "123456789012345678".into(),
            client_secret: "fixture-secret".into(),
            redirect_uri: String::new(),
            scopes: vec!["rpc".into()],
            token: None,
        };
        save(&path, &config).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(load(&path).unwrap().unwrap().client_id, config.client_id);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&path).is_err());
    }
    #[test]
    fn symlink_destination_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth");
        let target = dir.path().join("other");
        fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let config = Config {
            client_id: "123456789012345678".into(),
            client_secret: "fixture-secret".into(),
            redirect_uri: String::new(),
            scopes: vec!["rpc".into()],
            token: None,
        };
        assert!(save(&path, &config).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "original");
    }
}
