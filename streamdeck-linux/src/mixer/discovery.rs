//! Read-only installed application identities. Desktop Entry Specification 1.5:
//! https://specifications.freedesktop.org/desktop-entry/latest/
//! Flatpak exports: https://docs.flatpak.org/en/latest/conventions.html
//! Desktop IDs, not labels or generic launcher names, are persisted selectors.
use super::InstalledApplication;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_FILES: usize = 8192;
const MAX_FILE: usize = 256 * 1024;
#[derive(Clone, Default)]
pub(super) struct Catalog {
    entries: Vec<Entry>,
}
#[derive(Clone)]
struct Entry {
    no_display: bool,
    exec: Option<String>,
    id: String,
    name: String,
    file: PathBuf,
    aliases: BTreeSet<String>,
    executable: Option<PathBuf>,
    binary: Option<String>,
}
#[derive(Default)]
struct Process {
    display_name: Option<String>,
    desktop: Option<String>,
    flatpak: Option<String>,
    executable: Option<PathBuf>,
}
impl Catalog {
    pub fn scan(stop: &AtomicBool) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut roots = Vec::new();
        if let Some(dir) =
            absolute_env("XDG_DATA_HOME").or_else(|| home.as_ref().map(|h| h.join(".local/share")))
        {
            roots.push(dir.join("applications"));
        }
        let data = std::env::var_os("XDG_DATA_DIRS")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        roots.extend(
            std::env::split_paths(&data)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("applications")),
        );
        // Some sessions omit Flatpak export roots from XDG_DATA_DIRS.
        if let Some(home) = home {
            roots.push(home.join(".local/share/flatpak/exports/share/applications"));
        }
        roots.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        let paths = std::env::var_os("PATH")
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| p.is_absolute())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self::from_roots(&roots, &locale, &paths, stop)
    }
    fn from_roots(roots: &[PathBuf], locale: &str, paths: &[PathBuf], stop: &AtomicBool) -> Self {
        let mut seen = BTreeSet::new();
        let mut entries = Vec::new();
        let mut count = 0;
        for root in roots.iter().take(64) {
            let mut pending = vec![(root.clone(), 0)];
            while let Some((dir, depth)) = pending.pop() {
                if stop.load(Ordering::Acquire) || count >= MAX_FILES {
                    return Self::from_entries(entries);
                }
                let Ok(read) = fs::read_dir(&dir) else {
                    continue;
                };
                let mut files = read
                    .take(MAX_FILES - count)
                    .filter_map(Result::ok)
                    .collect::<Vec<_>>();
                files.sort_by_key(|entry| entry.file_name());
                for file in files {
                    count += 1;
                    if stop.load(Ordering::Acquire) || count > MAX_FILES {
                        return Self::from_entries(entries);
                    }
                    let path = file.path();
                    let Ok(kind) = file.file_type() else { continue };
                    if kind.is_dir() && depth < 8 {
                        pending.push((path, depth + 1));
                        continue;
                    }
                    if path.extension().is_none_or(|v| v != "desktop") {
                        continue;
                    }
                    let Some(id) = path
                        .strip_prefix(root)
                        .ok()
                        .and_then(|p| p.to_str())
                        .map(|s| s.replace('/', "-"))
                    else {
                        continue;
                    };
                    // First XDG occurrence wins, including Hidden tombstones.
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    let Some(text) = read_text(&path, MAX_FILE) else {
                        continue;
                    };
                    let values = section(&text, "Desktop Entry");
                    if values.get("Hidden").is_some_and(|v| v == "true")
                        || values.get("Type").is_none_or(|v| v != "Application")
                    {
                        continue;
                    }
                    let Some(name) = localized_name(&values, locale)
                        .filter(|v| !v.is_empty() && v.len() <= 1024)
                    else {
                        continue;
                    };
                    let mut aliases =
                        BTreeSet::from([id.clone(), id.trim_end_matches(".desktop").to_owned()]);
                    if let Some(flatpak) = values.get("X-Flatpak").filter(|v| valid_identity(v)) {
                        aliases.insert(flatpak.clone());
                    }
                    if let Some(path) = path.to_str() {
                        aliases.insert(path.to_owned());
                    }
                    let executable_token = values
                        .get("Exec")
                        .and_then(|v| exec_program(v))
                        .filter(|v| !generic(v));
                    let executable = executable_token
                        .as_ref()
                        .and_then(|s| executable_path(s, paths));
                    let binary = executable_token
                        .as_ref()
                        .and_then(|s| Path::new(s).file_name()?.to_str().map(str::to_owned));
                    entries.push(Entry {
                        no_display: values.get("NoDisplay").is_some_and(|v| v == "true"),
                        exec: values.get("Exec").cloned(),
                        id,
                        name,
                        file: path.canonicalize().unwrap_or(path),
                        aliases,
                        executable,
                        binary,
                    });
                }
            }
        }
        Self::from_entries(entries)
    }
    fn from_entries(mut entries: Vec<Entry>) -> Self {
        let mut folded = BTreeSet::new();
        for hidden in 0..entries.len() {
            if !entries[hidden].no_display {
                continue;
            }
            let matches = entries
                .iter()
                .enumerate()
                .filter(|(_, visible)| {
                    !visible.no_display
                        && visible.name == entries[hidden].name
                        && visible.exec.is_some()
                        && visible.exec == entries[hidden].exec
                        && visible.executable.is_some()
                        && visible.executable == entries[hidden].executable
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if matches.len() == 1 {
                let mut aliases = entries[hidden].aliases.clone();
                if let Some(path) = entries[hidden].file.to_str() {
                    aliases.insert(path.into());
                }
                entries[matches[0]].aliases.extend(aliases);
                folded.insert(hidden);
            }
        }
        Self {
            entries: entries
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !folded.contains(index))
                .map(|(_, entry)| entry)
                .collect(),
        }
    }
    pub fn applications(&self, detected: &BTreeSet<String>) -> Vec<InstalledApplication> {
        let mut apps = self
            .entries
            .iter()
            .filter(|e| !e.no_display)
            .map(|e| InstalledApplication {
                id: e.id.clone(),
                name: e.name.clone(),
                detected: detected.contains(&e.id),
            })
            .collect::<Vec<_>>();
        apps.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        apps
    }
    pub fn resolve(
        &self,
        app: Option<&str>,
        desktop: Option<&str>,
        binary: Option<&str>,
        pid: Option<&str>,
        display_name: &str,
    ) -> Option<String> {
        let mut process = pid
            .and_then(|v| v.parse::<u32>().ok())
            .map(process_identity)
            .unwrap_or_default();
        process.display_name = Some(display_name.into());
        self.resolve_with(app, desktop, binary, &process)
    }
    fn resolve_with(
        &self,
        app: Option<&str>,
        desktop: Option<&str>,
        binary: Option<&str>,
        process: &Process,
    ) -> Option<String> {
        // Explicit identity is stronger than shared host/sandbox executable names.
        let mut exact = BTreeSet::new();
        for hint in [
            desktop,
            process.desktop.as_deref(),
            process.flatpak.as_deref(),
            app,
        ]
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
        {
            let candidates = self
                .entries
                .iter()
                .filter(|e| e.aliases.contains(hint) || Path::new(hint) == e.file)
                .map(|e| e.id.clone())
                .collect::<BTreeSet<_>>();
            if candidates.len() > 1 {
                return None;
            }
            exact.extend(candidates);
        }
        if exact.len() == 1 {
            return exact.into_iter().next();
        }
        if !exact.is_empty()
            || process.flatpak.is_some()
            || process.desktop.is_some()
            || desktop.is_some()
        {
            return None;
        }
        if let Some(path) = &process.executable {
            let matches = self
                .entries
                .iter()
                .filter(|e| !e.no_display && e.executable.as_ref() == Some(path))
                .map(|e| e.id.clone())
                .collect::<BTreeSet<_>>();
            if !matches.is_empty() {
                return (matches.len() == 1)
                    .then(|| matches.into_iter().next())
                    .flatten();
            }
        }
        // Some packaged applications launch a sibling native executable through
        // a wrapper. Require both exact reported name and a private installation
        // directory; a browser PWA name or shared /usr/bin launcher is not proof.
        if let Some(path) = process.executable.as_ref()
            && let Some(directory) = path.parent().filter(|p| private_install_directory(p))
            && !generic(path.to_str().unwrap_or(""))
            && let Some(name) = process.display_name.as_deref()
        {
            let matches = self
                .entries
                .iter()
                .filter(|e| {
                    !e.no_display
                        && e.name == name
                        && e.executable
                            .as_ref()
                            .is_some_and(|launcher| launcher.parent() == Some(directory))
                })
                .map(|e| e.id.clone())
                .collect::<BTreeSet<_>>();
            if !matches.is_empty() {
                return (matches.len() == 1)
                    .then(|| matches.into_iter().next())
                    .flatten();
            }
        }
        let binary = binary.filter(|b| !generic(b))?;
        let matches = self
            .entries
            .iter()
            .filter(|e| !e.no_display && e.binary.as_deref() == Some(binary))
            .map(|e| e.id.clone())
            .collect::<BTreeSet<_>>();
        (matches.len() == 1)
            .then(|| matches.into_iter().next())
            .flatten()
    }
}
fn private_install_directory(path: &Path) -> bool {
    [
        "/opt",
        "/usr/lib",
        "/usr/lib64",
        "/usr/libexec",
        "/usr/local/lib",
    ]
    .iter()
    .any(|base| {
        path.strip_prefix(base).ok().is_some_and(|tail| {
            let mut parts = tail.components();
            parts.next().is_some_and(|first| {
                !matches!(
                    first.as_os_str().to_str(),
                    Some("bin" | "sbin" | "lib" | "lib64" | "libexec")
                )
            })
        })
    })
}
fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}
fn read_text(path: &Path, limit: usize) -> Option<String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}
fn section(text: &str, wanted: &str) -> BTreeMap<String, String> {
    let mut active = false;
    let mut result = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            active = line == format!("[{wanted}]");
            continue;
        }
        if !active || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            result
                .entry(key.trim().into())
                .or_insert_with(|| unescape(value));
        }
    }
    result
}
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('s') => out.push(' '),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(v) => {
                    out.push('\\');
                    out.push(v);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c)
        }
    }
    out
}
fn localized_name(values: &BTreeMap<String, String>, locale: &str) -> Option<String> {
    let (base, modifier) = locale
        .split_once('@')
        .map_or((locale, None), |(a, b)| (a, Some(b)));
    let base = base.split('.').next().unwrap_or(base);
    let lang = base.split('_').next().unwrap_or(base);
    let mut keys = Vec::new();
    if let Some(m) = modifier {
        keys.push(format!("Name[{base}@{m}]"));
    }
    keys.push(format!("Name[{base}]"));
    if let Some(m) = modifier {
        keys.push(format!("Name[{lang}@{m}]"));
    }
    keys.push(format!("Name[{lang}]"));
    keys.push("Name".into());
    keys.into_iter().find_map(|k| values.get(&k).cloned())
}
fn exec_program(value: &str) -> Option<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = false;
    let mut escape = false;
    for c in value.chars() {
        if escape {
            word.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == '"' {
            quote = !quote;
        } else if c.is_whitespace() && !quote {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quote || escape {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    let mut iter = words.into_iter();
    let first = iter.next()?;
    let program = if Path::new(&first).file_name().is_some_and(|s| s == "env") {
        iter.find(|w| w != "--" && !w.contains('='))?
    } else {
        first
    };
    if program.starts_with('-') || program.contains(['%', '=', '$', '`']) {
        None
    } else {
        Some(program)
    }
}
fn generic(program: &str) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    matches!(
        name,
        "" | "env"
            | "sh"
            | "bash"
            | "dash"
            | "zsh"
            | "fish"
            | "flatpak"
            | "bwrap"
            | "snap"
            | "gtk-launch"
            | "gio"
            | "xdg-open"
            | "wine"
            | "wine64"
            | "java"
            | "node"
            | "ruby"
            | "perl"
            | "mono"
            | "dotnet"
    ) || name.starts_with("python")
        || name.starts_with("electron")
}
fn executable_path(value: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(value);
    let canonical = if path.is_absolute() {
        path.canonicalize().ok()
    } else if path.components().count() == 1 {
        paths
            .iter()
            .find_map(|dir| dir.join(path).canonicalize().ok())
    } else {
        None
    }?;
    follow_forwarders(canonical)
}
/// Inspect only a narrow, literal shell-forwarder shape. No script is executed,
/// no variables are expanded, and no configuration files named by it are read.
fn follow_forwarders(mut path: PathBuf) -> Option<PathBuf> {
    let mut seen = BTreeSet::new();
    for depth in 0..=3 {
        if !seen.insert(path.clone()) {
            return None;
        }
        let Some(target) = read_text(&path, 16 * 1024).and_then(|text| forward_target(&text))
        else {
            return Some(path);
        };
        if depth == 3 {
            return None;
        }
        path = target.canonicalize().ok()?;
    }
    None
}
fn forward_target(script: &str) -> Option<PathBuf> {
    let shell = script.lines().next()?.strip_prefix("#!")?.trim();
    if !matches!(
        shell,
        "/bin/sh" | "/bin/bash" | "/bin/dash" | "/usr/bin/sh" | "/usr/bin/bash" | "/usr/bin/dash"
    ) {
        return None;
    }
    let lines = script
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect::<Vec<_>>();
    // Multiple execs (including redirection/process substitutions) and an exec
    // nested in a branch do not satisfy the unambiguous final-command shape.
    if lines
        .iter()
        .flat_map(|line| line.split(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        .filter(|word| *word == "exec")
        .count()
        != 1
    {
        return None;
    }
    let last = *lines.last()?;
    let args = last
        .strip_prefix("exec ")
        .or_else(|| last.strip_prefix("exec\t"))?
        .trim();
    if args.contains([';', '|', '&', '<', '>', '`']) || args.contains("$(") {
        return None;
    }
    let program = exec_program(args)?;
    if !Path::new(&program).is_absolute()
        || program.contains([';', '|', '&', '<', '>', '(', ')', '{', '}', '\n', '\r'])
    {
        return None;
    }
    // Reject unfinished compound constructs before the final exec. Ordinary
    // completed conditionals that only build arguments are allowed.
    let mut nesting = 0i32;
    for line in &lines[..lines.len() - 1] {
        let first = line.split_whitespace().next().unwrap_or("");
        match first {
            "if" | "case" | "for" | "while" | "until" | "select" => nesting += 1,
            "fi" | "esac" | "done" => nesting -= 1,
            _ => {}
        }
        if nesting < 0 || line.contains(['{', '}']) && !line.contains("${") {
            return None;
        }
    }
    (nesting == 0).then(|| PathBuf::from(program))
}
fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
fn process_identity(pid: u32) -> Process {
    let root = PathBuf::from(format!("/proc/{pid}"));
    if fs::metadata(&root).map_or(true, |m| m.uid() != unsafe { libc::geteuid() }) {
        return Process::default();
    }
    let stat = fs::read_to_string(root.join("stat")).ok();
    let mut result = Process {
        executable: fs::read_link(root.join("exe")).ok(),
        ..Default::default()
    };
    // Read only these allowlisted launch hints; never retain or log other values.
    if let Some(env) = read_text(&root.join("environ"), MAX_FILE) {
        for entry in env.split('\0') {
            if let Some((key, value)) = entry.split_once('=') {
                if matches!(key, "GIO_LAUNCHED_DESKTOP_FILE" | "BAMF_DESKTOP_FILE_HINT")
                    && valid_identity(value)
                {
                    result.desktop = Some(value.into());
                } else if key == "FLATPAK_ID" && valid_identity(value) {
                    result.flatpak = Some(value.into());
                }
            }
        }
    }
    if let Some(text) = read_text(&root.join("root/.flatpak-info"), MAX_FILE) {
        result.flatpak = section(&text, "Application")
            .get("name")
            .filter(|s| valid_identity(s))
            .cloned()
            .or(result.flatpak);
    }
    // Start-time identity avoids adopting hints from a recycled PID.
    let start = |s: &str| {
        s.rsplit_once(") ")
            .and_then(|(_, s)| s.split_whitespace().nth(19))
            .map(str::to_owned)
    };
    if stat.as_deref().and_then(start).is_none()
        || stat.as_deref().and_then(start)
            != fs::read_to_string(root.join("stat"))
                .ok()
                .as_deref()
                .and_then(start)
    {
        return Process::default();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(root: &Path, id: &str, extra: &str) {
        let path = root.join(id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!("[Desktop Entry]\nType=Application\nName=Example\n{extra}\n"),
        )
        .unwrap();
    }
    fn scan(roots: &[PathBuf], locale: &str) -> Catalog {
        Catalog::from_roots(roots, locale, &[], &AtomicBool::new(false))
    }
    #[test]
    fn installed_entries_exist_without_audio_and_xdg_override_hides_old() {
        let root = tempfile::tempdir().unwrap();
        let local = root.path().join("local");
        let system = root.path().join("system");
        entry(&system, "app.desktop", "Name[fr]=Système\nExec=/opt/player");
        entry(
            &local,
            "app.desktop",
            "Name[fr]=Lecteur local\nExec=/opt/player",
        );
        entry(&system, "gone.desktop", "Exec=/opt/gone");
        entry(&local, "gone.desktop", "Hidden=true");
        entry(
            &system,
            "nested/example.desktop",
            "Name[fr_CA]=Exemple\nExec=/opt/other",
        );
        let catalog = scan(&[local, system], "fr_CA.UTF-8");
        let apps = catalog.applications(&BTreeSet::new());
        assert_eq!(apps.len(), 2);
        assert!(apps.iter().all(|a| !a.detected));
        assert!(
            apps.iter()
                .any(|a| a.id == "app.desktop" && a.name == "Lecteur local")
        );
        assert!(
            apps.iter()
                .any(|a| a.id == "nested-example.desktop" && a.name == "Exemple")
        );
    }
    #[test]
    fn exact_executable_with_spaces_and_binary_ambiguity() {
        let root = tempfile::tempdir().unwrap();
        let one = root.path().join("one/Player with spaces");
        let two = root.path().join("two/Player with spaces");
        fs::create_dir_all(one.parent().unwrap()).unwrap();
        fs::create_dir_all(two.parent().unwrap()).unwrap();
        fs::write(&one, "").unwrap();
        fs::write(&two, "").unwrap();
        entry(
            root.path(),
            "one.desktop",
            &format!("Exec=\"{}\" %U", one.display()),
        );
        let catalog = scan(&[root.path().into()], "en");
        assert_eq!(
            catalog
                .resolve_with(None, None, Some("Player with spaces"), &Process::default())
                .as_deref(),
            Some("one.desktop")
        );
        entry(
            root.path(),
            "two.desktop",
            &format!("Exec=\"{}\" %U", two.display()),
        );
        let catalog = scan(&[root.path().into()], "en");
        assert!(
            catalog
                .resolve_with(None, None, Some("Player with spaces"), &Process::default())
                .is_none()
        );
        assert_eq!(
            catalog
                .resolve_with(
                    None,
                    None,
                    None,
                    &Process {
                        executable: Some(two),
                        ..Default::default()
                    }
                )
                .as_deref(),
            Some("two.desktop")
        );
    }
    #[test]
    fn flatpak_identity_distinguishes_shared_launcher_without_guessing() {
        let root = tempfile::tempdir().unwrap();
        entry(
            root.path(),
            "com.example.One.desktop",
            "Exec=/usr/bin/flatpak run com.example.One\nX-Flatpak=com.example.One",
        );
        entry(
            root.path(),
            "com.example.Two.desktop",
            "Exec=/usr/bin/flatpak run com.example.Two\nX-Flatpak=com.example.Two",
        );
        let catalog = scan(&[root.path().into()], "en");
        assert!(
            catalog
                .resolve_with(None, None, Some("flatpak"), &Process::default())
                .is_none()
        );
        assert_eq!(
            catalog
                .resolve_with(
                    Some("com.example.One"),
                    None,
                    Some("flatpak"),
                    &Process::default()
                )
                .as_deref(),
            Some("com.example.One.desktop")
        );
        assert_eq!(
            catalog
                .resolve_with(
                    None,
                    None,
                    Some("player"),
                    &Process {
                        flatpak: Some("com.example.Two".into()),
                        ..Default::default()
                    }
                )
                .as_deref(),
            Some("com.example.Two.desktop")
        );
        assert!(
            catalog
                .resolve_with(
                    Some("com.example.One"),
                    None,
                    None,
                    &Process {
                        flatpak: Some("com.example.Two".into()),
                        ..Default::default()
                    }
                )
                .is_none()
        );
    }
    #[test]
    fn launch_hint_resolves_shell_wrapper_and_shared_interpreters_never_match() {
        let root = tempfile::tempdir().unwrap();
        entry(root.path(), "shell.desktop", "Exec=bash -c something");
        entry(root.path(), "python.desktop", "Exec=python3 /opt/player.py");
        let catalog = scan(&[root.path().into()], "en");
        for binary in ["bash", "python3"] {
            assert!(
                catalog
                    .resolve_with(None, None, Some(binary), &Process::default())
                    .is_none()
            );
        }
        assert_eq!(
            catalog
                .resolve_with(
                    None,
                    None,
                    Some("bash"),
                    &Process {
                        desktop: Some(
                            root.path()
                                .join("shell.desktop")
                                .to_string_lossy()
                                .into_owned()
                        ),
                        ..Default::default()
                    }
                )
                .as_deref(),
            Some("shell.desktop")
        );
        assert_eq!(
            exec_program("env MODE=1 \"/opt/App Player\" %U").as_deref(),
            Some("/opt/App Player")
        );
        assert!(exec_program("\"unterminated").is_none());
    }
    #[test]
    fn duplicate_executable_is_ambiguous_but_explicit_desktop_id_is_not() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join("shared");
        fs::write(&exe, "").unwrap();
        for id in ["one.desktop", "two.desktop"] {
            entry(root.path(), id, &format!("Exec={}", exe.display()));
        }
        let catalog = scan(&[root.path().into()], "en");
        let process = Process {
            executable: Some(exe),
            ..Default::default()
        };
        assert!(
            catalog
                .resolve_with(None, None, Some("shared"), &process)
                .is_none()
        );
        assert_eq!(
            catalog
                .resolve_with(None, Some("one.desktop"), Some("shared"), &process)
                .as_deref(),
            Some("one.desktop")
        );
        assert!(
            catalog
                .applications(&BTreeSet::from(["one.desktop".into()]))
                .iter()
                .find(|a| a.id == "one.desktop")
                .unwrap()
                .detected
        );
    }
    fn bundled_entry(id: &str, name: &str, executable: &str) -> Entry {
        Entry {
            no_display: false,
            exec: Some(executable.into()),
            id: id.into(),
            name: name.into(),
            file: PathBuf::from(format!("/applications/{id}")),
            aliases: BTreeSet::new(),
            executable: Some(PathBuf::from(executable)),
            binary: None,
        }
    }
    #[test]
    fn hidden_portal_alias_folds_into_one_visible_app_but_distinct_launchers_do_not() {
        let root = tempfile::tempdir().unwrap();
        let launcher = root.path().join("launcher");
        fs::write(&launcher, "").unwrap();
        let command = format!("Exec=\"{}\" %U", launcher.display());
        entry(root.path(), "google-chrome.desktop", &command);
        entry(
            root.path(),
            "com.google.Chrome.desktop",
            &format!("{command}\nNoDisplay=true"),
        );
        entry(
            root.path(),
            "different-arguments.desktop",
            &format!("{command} --app-id=abc\nNoDisplay=true"),
        );
        entry(
            root.path(),
            "outlook.desktop",
            &format!("{command} --app-id=abc\nName[en]=Outlook (PWA)"),
        );
        let catalog = scan(&[root.path().into()], "en");
        assert_eq!(catalog.applications(&BTreeSet::new()).len(), 2);
        for hint in [
            "google-chrome.desktop",
            "com.google.Chrome.desktop",
            "com.google.Chrome",
        ] {
            assert_eq!(
                catalog
                    .resolve_with(Some(hint), None, None, &Process::default())
                    .as_deref(),
                Some("google-chrome.desktop")
            );
        }
        assert_eq!(
            catalog
                .resolve_with(
                    None,
                    Some(
                        root.path()
                            .join("com.google.Chrome.desktop")
                            .to_str()
                            .unwrap()
                    ),
                    None,
                    &Process::default()
                )
                .as_deref(),
            Some("google-chrome.desktop")
        );
        assert_eq!(
            catalog
                .resolve_with(
                    Some("different-arguments.desktop"),
                    None,
                    None,
                    &Process::default()
                )
                .as_deref(),
            Some("different-arguments.desktop")
        );
        assert_eq!(
            catalog
                .resolve_with(Some("outlook.desktop"), None, None, &Process::default())
                .as_deref(),
            Some("outlook.desktop")
        );
        // Same-name visible duplicates remain ambiguous; no hidden alias folds
        // into an arbitrarily chosen visible entry.
        entry(root.path(), "second-visible.desktop", &command);
        let catalog = scan(&[root.path().into()], "en");
        assert!(
            catalog
                .resolve_with(
                    None,
                    None,
                    None,
                    &Process {
                        executable: Some(launcher),
                        ..Default::default()
                    }
                )
                .is_none()
        );
    }
    #[test]
    fn folded_portal_alias_does_not_obscure_private_wrapper_name_match() {
        let mut main = bundled_entry(
            "google-chrome.desktop",
            "Google Chrome",
            "/opt/google/chrome/google-chrome",
        );
        main.aliases.insert(main.id.clone());
        let mut hidden = main.clone();
        hidden.id = "com.google.Chrome.desktop".into();
        hidden.aliases = BTreeSet::from([hidden.id.clone(), "com.google.Chrome".into()]);
        hidden.no_display = true;
        let mut different = hidden.clone();
        different.id = "hidden-webapp.desktop".into();
        different.aliases = BTreeSet::from([different.id.clone()]);
        different.exec = Some("/opt/google/chrome/google-chrome --app-id=abc".into());
        let catalog = Catalog::from_entries(vec![main, hidden, different]);
        assert_eq!(catalog.applications(&BTreeSet::new()).len(), 1);
        let process = Process {
            display_name: Some("Google Chrome".into()),
            executable: Some("/opt/google/chrome/chrome".into()),
            ..Default::default()
        };
        assert_eq!(
            catalog
                .resolve_with(None, None, Some("chrome"), &process)
                .as_deref(),
            Some("google-chrome.desktop")
        );
        assert_eq!(
            catalog
                .resolve_with(Some("com.google.Chrome"), None, None, &process)
                .as_deref(),
            Some("google-chrome.desktop")
        );
    }
    #[test]
    fn private_wrapper_bundle_name_separates_native_browser_from_pwa() {
        let catalog = Catalog {
            entries: vec![
                bundled_entry(
                    "google-chrome.desktop",
                    "Google Chrome",
                    "/opt/google/chrome/google-chrome",
                ),
                bundled_entry(
                    "outlook.desktop",
                    "Outlook (PWA)",
                    "/opt/google/chrome/google-chrome",
                ),
            ],
        };
        let mut process = Process {
            display_name: Some("Google Chrome".into()),
            executable: Some("/opt/google/chrome/chrome".into()),
            ..Default::default()
        };
        assert_eq!(
            catalog
                .resolve_with(None, None, Some("chrome"), &process)
                .as_deref(),
            Some("google-chrome.desktop")
        );
        process.display_name = Some("Outlook (PWA)".into());
        assert_eq!(
            catalog
                .resolve_with(None, None, Some("chrome"), &process)
                .as_deref(),
            Some("outlook.desktop")
        );
        process.display_name = Some("Chrome".into());
        assert!(
            catalog
                .resolve_with(None, None, Some("chrome"), &process)
                .is_none()
        );
        process.display_name = Some("Google Chrome".into());
        process.executable = Some("/opt/another/chrome".into());
        assert!(
            catalog
                .resolve_with(None, None, Some("chrome"), &process)
                .is_none()
        );
    }
    #[test]
    fn bundle_matching_rejects_shared_directories_generic_helpers_and_duplicate_names() {
        for dir in [
            "/usr/bin",
            "/bin",
            "/usr/local/bin",
            "/opt",
            "/opt/bin",
            "/usr/lib",
        ] {
            let catalog = Catalog {
                entries: vec![bundled_entry(
                    "app.desktop",
                    "Player",
                    &format!("{dir}/wrapper"),
                )],
            };
            let process = Process {
                display_name: Some("Player".into()),
                executable: Some(format!("{dir}/body").into()),
                ..Default::default()
            };
            assert!(
                catalog.resolve_with(None, None, None, &process).is_none(),
                "{dir}"
            );
        }
        let catalog = Catalog {
            entries: vec![
                bundled_entry("one.desktop", "Player", "/opt/player/wrapper"),
                bundled_entry("two.desktop", "Player", "/opt/player/wrapper"),
            ],
        };
        let mut process = Process {
            display_name: Some("Player".into()),
            executable: Some("/opt/player/body".into()),
            ..Default::default()
        };
        assert!(catalog.resolve_with(None, None, None, &process).is_none());
        let catalog = Catalog {
            entries: vec![bundled_entry(
                "one.desktop",
                "Player",
                "/opt/player/wrapper",
            )],
        };
        process.executable = Some("/opt/player/bash".into());
        assert!(catalog.resolve_with(None, None, None, &process).is_none());
    }
    #[test]
    fn literal_distro_forwarder_reaches_private_dynamic_wrapper_without_executing_it() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("launcher");
        let second = root.path().join("private-wrapper");
        fs::write(&first,format!("#!/bin/bash\nFLAGS=${{FLAGS:-}}\nif [[ -f /not/read ]]; then\n FLAGS=\"$(never-execute)\"\nfi\nexec \"{}\" $FLAGS \"$@\"\n",second.display())).unwrap();
        fs::write(
            &second,
            "#!/bin/bash\nHERE=/private\nexec \"$HERE/native\" \"$@\"\n",
        )
        .unwrap();
        assert_eq!(executable_path(first.to_str().unwrap(), &[]), Some(second));
    }
    #[test]
    fn nonliteral_branch_multiple_exec_and_forwarder_cycles_are_rejected() {
        for body in [
            "exec $TARGET \"$@\"",
            "exec $(find-target)",
            "exec relative-player",
            "exec /first\nexec /second",
            "if true; then\nexec /conditional\nfi",
            "if true; then\nexec /unfinished",
            "exec /body; other-command",
            "exec /body ; other-command",
            "exec /body $(dynamic-arguments)",
            "exec -a player /body",
        ] {
            assert!(
                forward_target(&format!("#!/bin/bash\n{body}\n")).is_none(),
                "{body}"
            );
        }
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        fs::write(&a, format!("#!/bin/sh\nexec {}\n", b.display())).unwrap();
        fs::write(&b, format!("#!/bin/sh\nexec {}\n", a.display())).unwrap();
        assert!(executable_path(a.to_str().unwrap(), &[]).is_none());
        for index in 0..5 {
            fs::write(
                root.path().join(format!("step{index}")),
                format!(
                    "#!/bin/sh\nexec {}/step{}\n",
                    root.path().display(),
                    index + 1
                ),
            )
            .unwrap();
        }
        assert!(executable_path(root.path().join("step0").to_str().unwrap(), &[]).is_none());
    }
    #[test]
    fn oversized_nonregular_invalid_utf8_and_cancelled_catalog_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("large.desktop"), vec![b'x'; MAX_FILE + 1]).unwrap();
        fs::write(root.path().join("invalid.desktop"), [255]).unwrap();
        let fifo = root.path().join("pipe.desktop");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        entry(root.path(), "valid.desktop", "Exec=/opt/valid");
        assert_eq!(scan(&[root.path().into()], "en").entries.len(), 1);
        assert!(
            Catalog::from_roots(&[root.path().into()], "en", &[], &AtomicBool::new(true))
                .entries
                .is_empty()
        );
    }
}
