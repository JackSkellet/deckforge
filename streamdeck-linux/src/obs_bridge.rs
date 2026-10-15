//! Optional, separately licensed native OBS module. No OBS process management.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};
use tungstenite::{Message, client::IntoClientRequest};
const MODULE: &str = "StreamDeckPlugin.so";
const PORTS: [u16; 7] = [28186, 39726, 34247, 42206, 38535, 40829, 40624];

fn base(variable: &str, fallback: &str) -> Result<PathBuf> {
    std::env::var_os(variable)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(fallback)))
        .context("Home directory unavailable")
}
fn destination() -> Result<PathBuf> {
    Ok(base("XDG_CONFIG_HOME", ".config")?.join("obs-studio/plugins/StreamDeckPlugin"))
}
fn bundle() -> Result<Option<PathBuf>> {
    let executable = std::env::current_exe()?;
    let candidates = [
        executable
            .parent()
            .context("Executable directory missing")?
            .join("obs-bridge"),
        base("XDG_DATA_HOME", ".local/share")?.join("deckforge/obs-bridge"),
    ];
    Ok(candidates
        .into_iter()
        .find(|p| p.join("build.json").is_file()))
}
fn digest(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    ensure!(bytes.len() < 128 * 1024 * 1024, "OBS module is too large");
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn manifest(root: &Path) -> Result<Value> {
    let file = fs::read(root.join("build.json"))?;
    ensure!(file.len() <= 32 * 1024, "Invalid OBS bridge manifest");
    Ok(serde_json::from_slice(&file)?)
}
fn validate(root: &Path) -> Result<Value> {
    let info = manifest(root)?;
    ensure!(
        info["sha256"].as_str() == Some(&digest(&root.join(MODULE))?),
        "OBS bridge checksum mismatch"
    );
    let elf = fs::read(root.join(MODULE))?;
    let machine = if cfg!(target_arch = "x86_64") {
        62u16
    } else if cfg!(target_arch = "aarch64") {
        183
    } else {
        0
    };
    ensure!(
        machine != 0
            && elf.len() >= 20
            && &elf[..6] == b"\x7fELF\x02\x01"
            && elf[18..20] == machine.to_le_bytes(),
        "OBS bridge architecture does not match this application"
    );
    for name in ["LICENSE.md", "LICENSES.md", "source.tar.gz"] {
        ensure!(
            root.join(name).is_file(),
            "OBS bridge source or license is missing"
        );
    }
    Ok(info)
}
fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    ensure!(
        !source.symlink_metadata()?.file_type().is_symlink(),
        "OBS bridge package contains a symlink"
    );
    if source.is_dir() {
        fs::create_dir_all(target)?;
        for item in fs::read_dir(source)? {
            let item = item?;
            copy_tree(&item.path(), &target.join(item.file_name()))?;
        }
    } else {
        ensure!(source.is_file(), "Invalid OBS bridge package entry");
        fs::copy(source, target)?;
    }
    Ok(())
}
fn install(source: &Path, target: &Path) -> Result<()> {
    let info = validate(source)?;
    // Preserve pre-existing third-party installations, including locally modified ones.
    if target.exists() {
        let previous = manifest(target)
            .context("An unmanaged OBS bridge already exists; it was left untouched")?;
        let installed = digest(&target.join("bin/64bit").join(MODULE))?;
        ensure!(
            previous["sha256"].as_str() == Some(&installed),
            "The installed OBS bridge was modified; it was left untouched"
        );
        if previous["sha256"] == info["sha256"] {
            return Ok(());
        }
        bail!(
            "A different OBS bridge is installed. Keep it or remove that managed installation before installing this build."
        );
    }
    let parent = target.parent().context("OBS plugin directory missing")?;
    fs::create_dir_all(parent)?;
    let pending = tempfile::Builder::new()
        .prefix(".deckforge-obs-")
        .tempdir_in(parent)?;
    fs::create_dir_all(pending.path().join("bin/64bit"))?;
    copy_tree(
        &source.join(MODULE),
        &pending.path().join("bin/64bit").join(MODULE),
    )?;
    for name in [
        "data",
        "LICENSE.md",
        "LICENSES.md",
        "source.tar.gz",
        "build.json",
    ] {
        copy_tree(&source.join(name), &pending.path().join(name))?;
    }
    fs::rename(pending.path(), target)?;
    Ok(())
}
fn probe() -> Option<Value> {
    for port in PORTS {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(120)) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let mut request = format!("ws://127.0.0.1:{port}/")
            .into_client_request()
            .ok()?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", "streamdeck-obs".parse().ok()?);
        let Ok((mut socket, response)) = tungstenite::client(request, stream) else {
            continue;
        };
        if response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok())
            != Some("streamdeck-obs")
        {
            continue;
        }
        if socket.send(Message::Text(json!({"jsonrpc":"2.0","id":"deckforge-probe","method":"version","params":{"version":"Deckforge setup"}}).to_string().into())).is_err() { continue; }
        // OBS may send its loaded notification before the version response.
        for _ in 0..16 {
            let Ok(message) = socket.read() else {
                break;
            };
            if let Message::Text(text) = message
                && let Ok(value) = serde_json::from_str::<Value>(&text)
                && value["id"] == "deckforge-probe"
                && value["result"]["obsverstr"].is_string()
                && value["result"]["version"].is_string()
            {
                let _ = socket.close(None);
                return Some(
                    json!({"port":port,"obs_version":value["result"]["obsverstr"],"bridge_version":value["result"]["version"]}),
                );
            }
        }
    }
    None
}
pub fn request(payload: &Value) -> Result<Value> {
    let target = destination()?;
    let source = bundle()?;
    match payload["operation"].as_str().unwrap_or("status") {
        "status" => {}
        "install" => install(
            source
                .as_deref()
                .context("Build the Linux OBS bridge first with scripts/build-obs-bridge.py")?,
            &target,
        )?,
        _ => bail!("Unknown OBS bridge operation"),
    }
    Ok(
        json!({"installed":target.join("bin/64bit").join(MODULE).is_file(),
        "available":source.is_some(),"path":target,"connection":probe()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_checks_architecture_checksum_and_preserves_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bundle");
        fs::create_dir_all(source.join("data/locale")).unwrap();
        let mut elf = vec![0u8; 20];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18..20].copy_from_slice(
            &(if cfg!(target_arch = "aarch64") {
                183u16
            } else {
                62u16
            })
            .to_le_bytes(),
        );
        fs::write(source.join(MODULE), &elf).unwrap();
        for name in ["LICENSE.md", "LICENSES.md", "source.tar.gz"] {
            fs::write(source.join(name), "fixture").unwrap();
        }
        fs::write(source.join("data/locale/en-US.ini"), "fixture").unwrap();
        fs::write(
            source.join("build.json"),
            json!({"sha256":digest(&source.join(MODULE)).unwrap()}).to_string(),
        )
        .unwrap();
        let target = temp.path().join("plugins/StreamDeckPlugin");
        install(&source, &target).unwrap();
        install(&source, &target).unwrap();
        fs::write(target.join("bin/64bit").join(MODULE), "user version").unwrap();
        assert!(install(&source, &target).is_err());
        assert_eq!(
            fs::read(target.join("bin/64bit").join(MODULE)).unwrap(),
            b"user version"
        );
        fs::write(source.join(MODULE), "corrupt").unwrap();
        assert!(install(&source, &temp.path().join("second")).is_err());
    }
}
