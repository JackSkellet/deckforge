//! Extract one known application member without installing or running vendor code.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const REFERENCE_LIMIT: u64 = 64 * 1024 * 1024;
const INSTALLER_LIMIT: u64 = 512 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const PAYLOAD: &str = "Stream_Deck.pkg/Payload";
const APPLICATION: &str = "./Applications/Stream Deck.app/Contents/MacOS/Stream Deck";

#[derive(Serialize)]
pub struct Imported {
    pub path: PathBuf,
    pub platform: &'static str,
    pub version: &'static str,
}

pub fn import(source: &Path, state: &Path) -> Result<Imported> {
    let source = source
        .canonicalize()
        .context("Find the Elgato installer or application")?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&source)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "Select a regular installer or application file"
    );
    ensure!(
        metadata.len() <= INSTALLER_LIMIT,
        "Elgato installer exceeds 512 MiB"
    );
    let mut magic = [0; 4];
    file.read_exact(&mut magic)
        .context("Installer or application file is truncated")?;
    let bytes = if &magic == b"xar!" {
        extract_pkg(&source)?
    } else if magic[..2] == *b"MZ" || magic == [0xca, 0xfe, 0xba, 0xbe] {
        ensure!(
            metadata.len() <= REFERENCE_LIMIT,
            "Application reference exceeds 64 MiB"
        );
        let mut bytes = magic.to_vec();
        file.take(REFERENCE_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= REFERENCE_LIMIT,
            "Application reference exceeds 64 MiB"
        );
        bytes
    } else {
        bail!(
            "Choose the supported Stream Deck macOS .pkg installer or its original application executable"
        );
    };
    let state = state
        .canonicalize()
        .context("Find the Deckforge data directory")?;
    let cache = state.join("references");
    match fs::symlink_metadata(&cache) {
        Ok(metadata) => ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Reference cache must be an owned private directory"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(&cache)?;
        }
        Err(error) => return Err(error.into()),
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".reference-")
        .tempfile_in(&cache)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    crate::protected_manifest::validate_reference(temporary.path())?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let path = cache.join(format!("{digest}.reference"));
    if path.try_exists()? || fs::symlink_metadata(&path).is_ok() {
        let existing = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .context("Open existing managed reference")?;
        let metadata = existing.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.len() <= REFERENCE_LIMIT,
            "Managed reference must be an owned private regular file"
        );
        let mut previous = Vec::new();
        existing
            .take(REFERENCE_LIMIT + 1)
            .read_to_end(&mut previous)?;
        ensure!(
            previous == bytes,
            "Managed reference contents do not match; the existing file was preserved"
        );
    } else {
        temporary
            .persist_noclobber(&path)
            .context("Save managed application reference")?;
        File::open(&cache)?.sync_all()?;
    }
    Ok(Imported {
        path,
        platform: if bytes.starts_with(b"MZ") {
            "Windows"
        } else {
            "macOS"
        },
        version: "7.6.0.23012",
    })
}

struct Processes(Vec<Child>);
impl Drop for Processes {
    fn drop(&mut self) {
        for child in &mut self.0 {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

fn extract_pkg(source: &Path) -> Result<Vec<u8>> {
    // Both commands write only to stdout. No member paths, installer scripts,
    // app bundles or package metadata are extracted into the filesystem.
    let outer = Command::new("bsdtar")
        .arg("-xOf")
        .arg(source)
        .arg(PAYLOAD)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Installer import requires bsdtar from libarchive")?;
    let mut processes = Processes(vec![outer]);
    let payload = processes.0[0]
        .stdout
        .take()
        .context("Installer payload pipe unavailable")?;
    let inner = Command::new("bsdtar")
        .args(["-xOf", "-", APPLICATION])
        .stdin(Stdio::from(payload))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Start installer application extraction")?;
    processes.0.push(inner);
    let output = processes.0[1]
        .stdout
        .take()
        .context("Application output pipe unavailable")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(REFERENCE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = Instant::now();
    let extracted = receiver.recv_timeout(TIMEOUT);
    let result = (|| {
        let bytes = extracted.context("Installer extraction timed out")??;
        ensure!(
            !bytes.is_empty(),
            "The installer did not contain the expected Stream Deck application"
        );
        ensure!(
            bytes.len() as u64 <= REFERENCE_LIMIT,
            "Extracted application exceeds 64 MiB"
        );
        for child in &mut processes.0 {
            loop {
                if let Some(status) = child.try_wait()? {
                    ensure!(
                        status.success(),
                        "Could not read the Stream Deck installer payload"
                    );
                    break;
                }
                ensure!(
                    started.elapsed() < TIMEOUT,
                    "Installer extraction timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(bytes)
    })();
    drop(processes); // Closing owned pipes also unblocks the bounded reader.
    let _ = reader.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_and_nonregular_inputs_leave_existing_reference_alone() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("unsupported.pkg");
        fs::write(&input, b"not an installer").unwrap();
        assert!(import(&input, root.path()).is_err());
        assert!(!root.path().join("references").exists());
        assert!(import(root.path(), root.path()).is_err());
        fs::write(&input, b"MZ this is not the supported executable").unwrap();
        assert!(import(&input, root.path()).is_err());
        assert_eq!(
            fs::read_dir(root.path().join("references"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    #[ignore = "requires the user-supplied Elgato macOS installer; extracts static data only"]
    fn supplied_pkg_extracts_a_verified_private_reference_without_vendor_execution() {
        let source = std::env::var_os("DECKFORGE_TEST_ELGATO_PKG").expect("set fixture path");
        let root = tempfile::tempdir().unwrap();
        let result = import(Path::new(&source), root.path()).unwrap();
        assert_eq!(result.platform, "macOS");
        assert_eq!(result.version, "7.6.0.23012");
        assert_eq!(fs::metadata(&result.path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(result.path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
        crate::protected_manifest::validate_reference(&result.path).unwrap();
        let second = import(Path::new(&source), root.path()).unwrap();
        assert_eq!(second.path, result.path);
        assert_eq!(
            fs::read_dir(result.path.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
