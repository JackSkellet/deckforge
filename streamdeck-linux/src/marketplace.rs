//! Marketplace session and bounded package transfer, separate from plugin policy.
#[path = "marketplace_browser.rs"]
pub mod browser;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use url::Url;

const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024;
#[derive(Default, Serialize)]
pub struct State {
    pub busy: bool,
    pub authenticated: Option<bool>,
    pub account: Option<String>,
    pub message: String,
    pub error: Option<String>,
    pub pending: Option<uuid::Uuid>,
    pub review: Option<serde_json::Value>,
}

// Deliberately no Debug: url can contain a signed download credential.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserReply {
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}
impl BrowserReply {
    pub fn error(message: &str) -> Self {
        Self {
            kind: "error".into(),
            name: None,
            url: None,
            error: Some(message.into()),
        }
    }
}

pub fn private_directory(path: &Path) -> Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("Create private Marketplace directory"),
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Unsafe Marketplace directory"
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Serializes this browser profile across processes as well as editor requests.
pub fn lock(state: &Path) -> Result<fs::File> {
    let root = state.join("marketplace");
    private_directory(&root)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("session.lock"))?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "Marketplace is open in another Deckforge session. Close it and retry."
    );
    Ok(file)
}

pub fn clear_session(state: &Path) -> Result<()> {
    let profile = state.join("marketplace/browser");
    if profile.try_exists()? {
        private_directory(&profile)?;
        fs::remove_dir_all(profile).context("Clear Marketplace session")?;
    }
    Ok(())
}

pub fn session(
    state: &Path,
    operation: &str,
    variant: Option<uuid::Uuid>,
    running: impl Fn() -> bool,
) -> Result<BrowserReply> {
    let profile = state.join("marketplace/browser");
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("marketplace-browser")
        .arg("--profile")
        .arg(profile)
        .arg("--operation")
        .arg(operation);
    if let Some(id) = variant {
        command.arg("--variant").arg(id.to_string());
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Open Marketplace browser")?;
    let stdout = child
        .stdout
        .take()
        .context("Marketplace response pipe missing")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(65537).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(660);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if running() && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(anyhow::anyhow!(
                    "Marketplace request cancelled or timed out"
                ));
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Marketplace browser response failed"))??;
    ensure!(
        status?.success(),
        "Marketplace browser could not finish. Retry sign-in from Preferences."
    );
    ensure!(
        bytes.len() <= 65536,
        "Marketplace browser response exceeded its limit"
    );
    serde_json::from_slice(&bytes).context("Invalid Marketplace browser response")
}

fn allowed_download(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some("mp-cdn.elgato.com" | "mp-gateway.elgato.com" | "marketplace.elgato.com")
        )
}

fn download_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        // Elgato's CDN rejects otherwise valid signed links when this is absent.
        .user_agent(concat!("Deckforge/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

pub fn download(
    link: &str,
    directory: &Path,
    mut progress: impl FnMut(u64, Option<u64>) -> bool,
) -> Result<PathBuf> {
    let mut url = Url::parse(link)
        .map_err(|_| anyhow::anyhow!("Marketplace returned an invalid download URL"))?;
    let client = download_client()?;
    for _ in 0..=5 {
        ensure!(
            allowed_download(&url),
            "Marketplace returned an unsupported download host"
        );
        let response = client.get(url.clone()).send().map_err(|_| {
            anyhow::anyhow!("Marketplace download failed. Check your connection and retry.")
        })?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .context("Marketplace download redirect is missing")?;
            url = url
                .join(location)
                .map_err(|_| anyhow::anyhow!("Invalid Marketplace download redirect"))?;
            continue;
        }
        ensure!(
            response.status().is_success(),
            "Marketplace download was refused (HTTP {}). Retry to obtain a fresh link.",
            response.status().as_u16()
        );
        let total = response.content_length();
        ensure!(
            total.is_none_or(|n| n <= MAX_DOWNLOAD),
            "Marketplace package exceeds 128 MiB"
        );
        let path = directory.join("plugin.streamDeckPlugin");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let result = copy_download(response, &mut file, total, &mut progress);
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        file.sync_all()?;
        return Ok(path);
    }
    bail!("Too many Marketplace download redirects")
}
fn copy_download(
    mut source: impl Read,
    target: &mut impl Write,
    total: Option<u64>,
    progress: &mut impl FnMut(u64, Option<u64>) -> bool,
) -> Result<()> {
    let mut received = 0u64;
    let mut buffer = [0; 65536];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|_| anyhow::anyhow!("Marketplace download was interrupted. Retry."))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        ensure!(
            received <= MAX_DOWNLOAD,
            "Marketplace package exceeds 128 MiB"
        );
        ensure!(progress(received, total), "Marketplace download cancelled");
        target.write_all(&buffer[..count])?;
    }
    ensure!(
        received > 0 && total.is_none_or(|n| n == received),
        "Marketplace download is incomplete"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_transfer_identifies_deckforge_to_the_cdn() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        download_client()
            .unwrap()
            .get(format!("http://{address}/plugin"))
            .send()
            .unwrap();
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.contains(&format!(
            "user-agent: deckforge/{}\r\n",
            env!("CARGO_PKG_VERSION")
        )));
    }
    #[test]
    fn signed_links_and_redirects_are_confined_to_elgato_https() {
        assert!(allowed_download(
            &Url::parse("https://mp-cdn.elgato.com/plugin?signature=secret").unwrap()
        ));
        for value in [
            "http://mp-cdn.elgato.com/plugin",
            "https://mp-cdn.elgato.com.evil.test/a",
            "https://127.0.0.1/a",
            "file:///tmp/a",
            "https://mp-cdn.elgato.com:444/a",
            "https://user:secret@mp-cdn.elgato.com/a",
        ] {
            assert!(!allowed_download(&Url::parse(value).unwrap()));
        }
        assert!(browser::trusted_origin(
            "https://marketplace.elgato.com/account/products"
        ));
        assert!(!browser::trusted_origin("https://account.elgato.com/auth"));
        assert!(!browser::trusted_origin(
            "https://marketplace.elgato.com.evil.test"
        ));
    }
    #[test]
    fn transfer_handles_cancellation_truncation_and_unannounced_oversize() {
        let mut output = Vec::new();
        assert!(copy_download(&b"zip"[..], &mut output, Some(3), &mut |_, _| true).is_ok());
        assert_eq!(output, b"zip");
        assert!(
            copy_download(&b"zip"[..], &mut std::io::sink(), Some(4), &mut |_, _| true).is_err()
        );
        assert!(copy_download(&b"zip"[..], &mut std::io::sink(), None, &mut |_, _| false).is_err());
        assert!(
            copy_download(
                std::io::repeat(0).take(MAX_DOWNLOAD + 1),
                &mut std::io::sink(),
                None,
                &mut |_, _| true
            )
            .is_err()
        );
    }
    #[test]
    fn profile_is_private_locked_and_logout_does_not_follow_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let lock1 = lock(root.path()).unwrap();
        assert!(lock(root.path()).is_err());
        let profile = root.path().join("marketplace/browser");
        private_directory(&profile).unwrap();
        assert_eq!(
            fs::metadata(&profile).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::write(profile.join("cookies"), "PRIVATE_SESSION").unwrap();
        clear_session(root.path()).unwrap();
        assert!(!profile.exists());
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "keep").unwrap();
        std::os::unix::fs::symlink(&outside, &profile).unwrap();
        assert!(clear_session(root.path()).is_err());
        assert!(outside.join("keep").exists());
        drop(lock1);
        assert!(lock(root.path()).is_ok());
    }
}
