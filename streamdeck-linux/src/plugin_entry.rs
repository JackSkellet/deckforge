//! Portable entry selection is a Linux policy, not an Elgato platform claim.
//! Manifest roles/Node flags: https://docs.elgato.com/streamdeck/sdk/references/manifest/
//! Runtimes: https://docs.elgato.com/streamdeck/sdk/v1/introduction/plugin-environment/
//! Node suffixes: https://nodejs.org/api/packages.html#determining-module-system
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    os::unix::{fs::PermissionsExt, io::AsRawFd},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const NODE_FLAGS: [&str; 2] = ["--enable-source-maps", "--no-global-search-paths"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Node,
    Html,
    Native,
}
#[derive(Debug, Eq, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
    pub field: &'static str,
}

/// A declared generic entry is authoritative, including its errors. Only an
/// absent/null CodePath permits a unique portable Windows/macOS alternative.
pub fn resolve(root: &Path, manifest: &Value) -> Result<Entry> {
    let root = root.canonicalize().context("Resolve plugin directory")?;
    if let Some(code) = field(manifest, "CodePath")? {
        let relative = relative(code)?;
        let path = contained_file(&root, &relative)?;
        let kind = if let Some(kind) = kind(&relative) {
            kind
        } else if linux_native(&path)? {
            Kind::Native
        } else {
            anyhow::bail!(
                "Native Windows/macOS plugin binaries and other non-JavaScript/HTML entries are not supported on Linux"
            );
        };
        return Ok(Entry {
            path,
            kind,
            field: "CodePath",
        });
    }
    let mut choices: Vec<Entry> = Vec::new();
    for name in ["CodePathWin", "CodePathMac"] {
        let Some(code) = field(manifest, name)? else {
            continue;
        };
        let relative = relative(code)?;
        let Some(kind) = kind(&relative) else {
            continue;
        };
        let path = match contained_file(&root, &relative) {
            Ok(path) => path,
            Err(error)
                if error.downcast_ref::<io::Error>().is_some_and(|error| {
                    matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    )
                }) =>
            {
                continue;
            }
            Err(error) => return Err(error).with_context(|| format!("Invalid {name}")),
        };
        if !choices.iter().any(|entry| entry.path == path) {
            choices.push(Entry {
                path,
                kind,
                field: name,
            });
        }
    }
    ensure!(
        choices.len() <= 1,
        "Ambiguous portable plugin entries: CodePathWin and CodePathMac select different files; provide a generic CodePath"
    );
    choices.pop().context("Plugin has no existing portable JavaScript/HTML entry; native Windows/macOS binaries are unsupported")
}

fn linux_native(path: &Path) -> Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Plugin entry is not a regular file");
    let mut header = Vec::new();
    file.take(64).read_to_end(&mut header)?;
    if !header.starts_with(b"\x7fELF") {
        return Ok(false);
    }
    ensure!(header.len() >= 52, "Truncated Linux ELF plugin header");
    let expected = match std::env::consts::ARCH {
        "x86_64" => (2, 62),
        "aarch64" => (2, 183),
        "x86" => (1, 3),
        "arm" => (1, 40),
        _ => anyhow::bail!("Native plugin architecture is unsupported"),
    };
    ensure!(
        header[4] == expected.0
            && header[5] == 1
            && header[6] == 1
            && u16::from_le_bytes([header[18], header[19]]) == expected.1,
        "Linux plugin architecture does not match this host"
    );
    ensure!(
        matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3),
        "ELF plugin is not an executable"
    );
    ensure!(
        metadata.permissions().mode() & 0o111 != 0,
        "Linux plugin is not marked executable"
    );
    Ok(true)
}

fn field<'a>(manifest: &'a Value, name: &str) -> Result<Option<&'a str>> {
    let Some(value) = manifest.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    Ok(Some(
        value
            .as_str()
            .filter(|value| !value.is_empty())
            .with_context(|| format!("{name} must be a nonempty relative path"))?,
    ))
}
fn relative(value: &str) -> Result<PathBuf> {
    // Windows separators are path separators here, not literal Linux filenames.
    let path = PathBuf::from(value.replace('\\', "/"));
    ensure!(
        !value.contains(['\0', ':'])
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir)),
        "Plugin entry must remain inside its directory"
    );
    Ok(path)
}
fn kind(path: &Path) -> Option<Kind> {
    // Classify suffixes without modifying filename case or Node's module rules.
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "js" | "mjs" | "cjs" => Some(Kind::Node),
        "html" | "htm" => Some(Kind::Html),
        _ => None,
    }
}
fn contained_file(root: &Path, relative: &Path) -> Result<PathBuf> {
    let path = root
        .join(relative)
        .canonicalize()
        .context("Plugin entry unavailable")?;
    ensure!(
        path.starts_with(root),
        "Plugin entry symlink escapes its directory"
    );
    ensure!(path.is_file(), "Plugin entry is not a regular file");
    Ok(path)
}

#[derive(Default)]
struct Discovery {
    explicit: Option<PathBuf>,
    path: Vec<PathBuf>,
    version_roots: Vec<PathBuf>,
}
impl Discovery {
    fn environment(major: &str) -> Self {
        let mut roots = Vec::new();
        if let Some(nvm) = std::env::var_os("NVM_DIR") {
            roots.push(PathBuf::from(nvm).join("versions/node"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"));
            roots.extend([data.join("nvm"), home.join(".nvm/versions/node")]);
        }
        let mut path: Vec<_> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        path.push(PathBuf::from("/usr/bin"));
        Self {
            explicit: std::env::var_os(format!("DECKFORGE_NODE{major}")).map(PathBuf::from),
            path,
            version_roots: roots,
        }
    }
    fn candidates(&self, major: u32) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(explicit) = &self.explicit {
            if explicit.components().count() == 1 && !explicit.is_absolute() {
                candidates.extend(self.path.iter().map(|directory| directory.join(explicit)));
            } else {
                candidates.push(explicit.clone());
            }
        }
        candidates.extend(self.path.iter().map(|directory| directory.join("node")));
        for root in &self.version_roots {
            let mut versions: Vec<_> = fs::read_dir(root)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    let version = version(&name)?;
                    (version.0 == major).then_some((version, name, entry.path().join("bin/node")))
                })
                .collect();
            // Prefer highest installed patch/minor within each configured root;
            // the filename tie-breaker makes discovery independent of readdir order.
            versions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            candidates.extend(versions.into_iter().map(|(_, _, path)| path));
        }
        let mut seen = HashSet::new();
        candidates
            .into_iter()
            .filter_map(|path| path.canonicalize().ok())
            .filter(|path| {
                path.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                }) && seen.insert(path.clone())
            })
            .collect()
    }
}
fn version(name: &str) -> Option<(u32, u32, u32)> {
    let base = name.strip_prefix('v')?.split('-').next()?;
    let mut parts = base.split('.');
    let result = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(result)
}
fn major(required: &str) -> Result<u32> {
    ensure!(
        !required.is_empty()
            && required.len() <= 3
            && required.bytes().all(|byte| byte.is_ascii_digit()),
        "Invalid Node.js major version"
    );
    let major = required.parse()?;
    ensure!(major > 0, "Invalid Node.js major version");
    Ok(major)
}
fn select(
    candidates: Vec<PathBuf>,
    required: &str,
    mut probe: impl FnMut(&Path, &str) -> bool,
) -> Result<PathBuf> {
    // No arbitrary candidate count limit: a later valid runtime is not hidden by
    // invalid executables encountered first. Each individual probe is bounded.
    candidates
        .into_iter()
        .find(|path| probe(path, required))
        .with_context(|| {
            format!(
                "Node.js {required} is required; set DECKFORGE_NODE{required} to its executable"
            )
        })
}
pub fn node_runtime(required: &str) -> Result<PathBuf> {
    let major = major(required)?;
    select(
        Discovery::environment(required).candidates(major),
        required,
        probe_node,
    )
}
fn probe_node(path: &Path, required: &str) -> bool {
    let Ok(mut child) = Command::new(path)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_millis(250);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    if !success {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    let Some(stdout) = child.stdout.take() else {
        return false;
    };
    // SAFETY: stdout owns a live fd. Nonblocking reads prevent an incorrectly
    // written wrapper's inherited stdout from holding discovery open forever.
    if unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return false;
    }
    let mut bytes = Vec::new();
    let _ = stdout.take(128).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).starts_with(&format!("v{required}."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::symlink;
    fn file(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"fixture").unwrap();
        path
    }
    #[test]
    fn native_linux_binary_requires_valid_architecture_and_executable_permissions() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugin");
        fs::copy(std::env::current_exe().unwrap(), &plugin).unwrap();
        fs::set_permissions(&plugin, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve(root.path(), &json!({"CodePath":"plugin"}))
                .unwrap()
                .kind,
            Kind::Native
        );
        fs::set_permissions(&plugin, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            resolve(root.path(), &json!({"CodePath":"plugin"}))
                .unwrap_err()
                .to_string()
                .contains("executable")
        );
        let mut wrong = [0u8; 64];
        wrong[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        fs::write(&plugin, wrong).unwrap();
        fs::set_permissions(&plugin, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            resolve(root.path(), &json!({"CodePath":"plugin"}))
                .unwrap_err()
                .to_string()
                .contains("architecture")
        );
    }

    #[test]
    fn generic_portable_entry_wins_and_declared_generic_errors_never_fall_back() {
        let root = tempfile::tempdir().unwrap();
        let generic = file(root.path(), "main.cjs");
        file(root.path(), "other.mjs");
        file(root.path(), "plugin.exe");
        let entry = resolve(
            root.path(),
            &json!({"CodePath":"main.cjs","CodePathWin":"other.mjs"}),
        )
        .unwrap();
        assert_eq!(
            entry,
            Entry {
                path: generic,
                kind: Kind::Node,
                field: "CodePath"
            }
        );
        assert!(
            resolve(
                root.path(),
                &json!({"CodePath":"missing.js","CodePathWin":"other.mjs"})
            )
            .is_err()
        );
        let error = resolve(
            root.path(),
            &json!({"CodePath":"plugin.exe","CodePathWin":"other.mjs"}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Native Windows/macOS"));
        assert!(
            resolve(
                root.path(),
                &json!({"CodePath":"","CodePathMac":"other.mjs"})
            )
            .is_err()
        );
    }
    #[test]
    fn platform_fallback_requires_one_distinct_existing_portable_target() {
        let root = tempfile::tempdir().unwrap();
        let shared = file(root.path(), "bin/main.mjs");
        file(root.path(), "other.js");
        file(root.path(), "native.exe");
        let entry = resolve(
            root.path(),
            &json!({"CodePathWin":"bin\\main.mjs","CodePathMac":"bin/main.mjs"}),
        )
        .unwrap();
        assert_eq!(entry.path, shared);
        assert_eq!(entry.field, "CodePathWin");
        symlink(&shared, root.path().join("alias.mjs")).unwrap();
        assert!(
            resolve(
                root.path(),
                &json!({"CodePathWin":"alias.mjs","CodePathMac":"bin/main.mjs"})
            )
            .is_ok()
        );
        assert!(
            resolve(
                root.path(),
                &json!({"CodePathWin":"bin/main.mjs","CodePathMac":"other.js"})
            )
            .unwrap_err()
            .to_string()
            .contains("Ambiguous")
        );
        assert!(
            resolve(
                root.path(),
                &json!({"CodePath":null,"CodePathWin":"missing.js","CodePathMac":"other.js"})
            )
            .is_ok()
        );
        assert!(resolve(root.path(), &json!({"CodePathWin":"native.exe"})).is_err());
    }
    #[test]
    fn paths_cannot_escape_or_resolve_to_directories_and_case_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("plugin");
        fs::create_dir(&plugin).unwrap();
        file(root.path(), "outside.js");
        file(&plugin, "MAIN.CJS");
        let html = file(&plugin, "PAGE.HTML");
        assert_eq!(
            resolve(&plugin, &json!({"CodePath":"MAIN.CJS"}))
                .unwrap()
                .kind,
            Kind::Node
        );
        assert_eq!(
            resolve(&plugin, &json!({"CodePathWin":"PAGE.HTML"}))
                .unwrap()
                .path,
            html
        );
        assert!(resolve(&plugin, &json!({"CodePath":"main.cjs"})).is_err());
        for path in [
            "../outside.js",
            "..\\outside.js",
            "/tmp/outside.js",
            "C:\\outside.js",
            "https://example.com/a.js",
        ] {
            assert!(resolve(&plugin, &json!({"CodePathWin":path})).is_err());
        }
        symlink(root.path().join("outside.js"), plugin.join("escape.js")).unwrap();
        assert!(resolve(&plugin, &json!({"CodePath":"escape.js"})).is_err());
        fs::create_dir(plugin.join("directory.js")).unwrap();
        assert!(resolve(&plugin, &json!({"CodePath":"directory.js"})).is_err());
    }
    fn executable(root: &Path, name: &str) -> PathBuf {
        let path = file(root, name);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    #[test]
    fn runtime_order_is_explicit_then_path_then_sorted_matching_versions_without_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let explicit = executable(root.path(), "explicit");
        let path = executable(root.path(), "path/node");
        let versions = root.path().join("versions");
        let low = executable(&versions, "v20.9.0/bin/node");
        let high = executable(&versions, "v20.20.1/bin/node");
        executable(&versions, "v24.1.0/bin/node");
        let alias = root.path().join("alias");
        fs::create_dir(&alias).unwrap();
        symlink(&path, alias.join("node")).unwrap();
        let discovery = Discovery {
            explicit: Some(explicit.clone()),
            path: vec![path.parent().unwrap().into(), alias],
            version_roots: vec![versions],
        };
        assert_eq!(discovery.candidates(20), [explicit, path, high, low]);
    }
    #[test]
    fn valid_runtime_after_eight_failures_is_not_dropped_and_invalid_majors_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let versions = root.path().join("versions");
        for patch in 0..12 {
            executable(&versions, &format!("v20.0.{patch}/bin/node"));
        }
        let candidates = Discovery {
            version_roots: vec![versions.clone()],
            ..Default::default()
        }
        .candidates(20);
        let mut visited = Vec::new();
        let selected = select(candidates, "20", |path, _| {
            visited.push(path.to_owned());
            path == versions.join("v20.0.0/bin/node")
        })
        .unwrap();
        assert_eq!(selected, versions.join("v20.0.0/bin/node"));
        assert_eq!(visited.len(), 12);
        for bad in ["", "0", "20.1", "../20", "20=oops", "02000"] {
            assert!(major(bad).is_err());
        }
        assert_eq!(
            NODE_FLAGS,
            ["--enable-source-maps", "--no-global-search-paths"]
        );
    }
}
