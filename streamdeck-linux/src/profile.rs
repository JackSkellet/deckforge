//! Profile format inferred from Elgato's own Lights Out fixtures. No host code reused.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::Path,
};

const MAX_FILES: usize = 10_000;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Slot {
    pub controller: String,
    pub column: u8,
    pub row: u8,
    /// Plugin action UUID, distinct from the per-placement ActionID.
    pub uuid: String,
    pub settings: Value,
    pub states: Value,
    /// Preserve unsupported properties for future support and lossless inspection.
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadedProfile {
    pub name: String,
    pub uuid: String,
    pub page: String,
    pub pages: Vec<String>,
    pub slots: Vec<Slot>,
    pub warnings: Vec<String>,
    pub raw: Value,
    pub page_raw: Value,
    #[serde(skip)]
    files: BTreeMap<String, Vec<u8>>,
    #[serde(skip)]
    root: String,
    #[serde(skip)]
    page_root: String,
}

impl LoadedProfile {
    /// Resolve an embedded asset relative to the selected page, then profile root.
    /// Plugin-relative images remain the responsibility of the plugin loader.
    pub fn asset(&self, path: &str) -> Result<Vec<u8>> {
        safe_path(path)?;
        for base in [&self.page_root, &self.root] {
            if let Some(bytes) = self.files.get(&format!("{base}{path}")) {
                return Ok(bytes.clone());
            }
        }
        bail!("Embedded profile asset not found: {path}")
    }

    pub fn warn_unknown_actions(&mut self, known: &HashSet<String>) {
        let mut seen = HashSet::new();
        for slot in &self.slots {
            if !known.contains(&slot.uuid) && seen.insert(&slot.uuid) {
                self.warnings.push(format!(
                    "Action {} is preserved but has no installed handler",
                    slot.uuid
                ));
            }
        }
    }
}

/// Load the currently selected page, falling back to the default page.
pub fn load(path: &Path) -> Result<LoadedProfile> {
    load_page(path, None)
}

/// Select a page by UUID, encoded directory name, or the literal "default".
pub fn load_page(path: &Path, selected: Option<&str>) -> Result<LoadedProfile> {
    let files = read_files(path)?;
    let mut loaded = parse(files, selected)?;
    if loaded.uuid.is_empty() && path.is_dir() {
        loaded.uuid = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .trim_end_matches(".sdProfile")
            .to_owned();
    }
    Ok(loaded)
}

fn safe_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && !path.starts_with('/') && !path.contains(['\\', ':', '\0']),
        "Unsafe profile path: {path:?}"
    );
    ensure!(
        path.split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".."),
        "Unsafe profile path: {path:?}"
    );
    Ok(())
}

fn insert_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
    name: String,
    data: Vec<u8>,
) -> Result<()> {
    safe_path(&name)?;
    ensure!(files.len() < MAX_FILES, "Profile contains too many files");
    ensure!(
        data.len() as u64 <= MAX_FILE,
        "Profile file is too large: {name}"
    );
    *total = total
        .checked_add(data.len())
        .context("Profile size overflow")?;
    ensure!(
        *total <= MAX_TOTAL,
        "Profile exceeds 128 MiB expanded limit"
    );
    ensure!(
        files.insert(name.clone(), data).is_none(),
        "Duplicate archive path: {name}"
    );
    Ok(())
}

fn read_files(path: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let mut total = 0;
    if path.is_dir() {
        read_directory(path, "", &mut files, &mut total)?;
    } else {
        let file =
            fs::File::open(path).with_context(|| format!("Open profile {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file).context("Read .streamDeckProfile ZIP")?;
        crate::package::validate_central_directory(path, archive.central_directory_start())?;
        ensure!(
            archive.len() <= MAX_FILES,
            "Profile contains too many entries"
        );
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            safe_path(entry.name().trim_end_matches('/'))?;
            ensure!(
                entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
                "Profile contains a symlink"
            );
            if entry.is_dir() {
                continue;
            }
            ensure!(entry.size() <= MAX_FILE, "Profile entry exceeds 32 MiB");
            let name = entry.name().to_owned();
            let mut bytes = Vec::new();
            (&mut entry).take(MAX_FILE + 1).read_to_end(&mut bytes)?;
            insert_file(&mut files, &mut total, name, bytes)?;
        }
    }
    Ok(files)
}

fn read_directory(
    path: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
) -> Result<()> {
    ensure!(
        prefix.matches('/').count() < 32,
        "Profile directory is too deeply nested"
    );
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = format!(
            "{prefix}{}",
            entry
                .file_name()
                .to_str()
                .context("Non-UTF8 profile filename")?
        );
        safe_path(&name)?;
        ensure!(!kind.is_symlink(), "Profile contains a symlink: {name}");
        if kind.is_dir() {
            read_directory(&entry.path(), &format!("{name}/"), files, total)?;
        } else {
            ensure!(kind.is_file(), "Profile contains a special file: {name}");
            ensure!(
                entry.metadata()?.len() <= MAX_FILE,
                "Profile file exceeds 32 MiB: {name}"
            );
            let mut bytes = Vec::new();
            fs::File::open(entry.path())?
                .take(MAX_FILE + 1)
                .read_to_end(&mut bytes)?;
            insert_file(files, total, name, bytes)?;
        }
    }
    Ok(())
}

fn json_file(files: &BTreeMap<String, Vec<u8>>, path: &str) -> Result<Value> {
    serde_json::from_slice(files.get(path).with_context(|| format!("Missing {path}"))?)
        .with_context(|| format!("Invalid JSON in {path}"))
}

fn parse(files: BTreeMap<String, Vec<u8>>, selected: Option<&str>) -> Result<LoadedProfile> {
    let roots: Vec<_> = files
        .keys()
        .filter(|p| {
            p.as_str() == "manifest.json"
                || (p.ends_with(".sdProfile/manifest.json") && p.matches('/').count() == 1)
        })
        .cloned()
        .collect();
    ensure!(
        roots.len() == 1,
        "Expected one profile root manifest, found {}",
        roots.len()
    );
    let root_manifest = &roots[0];
    let raw = json_file(&files, root_manifest)?;
    let root = root_manifest.trim_end_matches("manifest.json").to_owned();
    let uuid = root
        .trim_end_matches('/')
        .trim_end_matches(".sdProfile")
        .to_owned();
    let mut warnings = Vec::new();
    let mut pages: Vec<String> = raw
        .pointer("/Pages/Pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    for key in ["Default", "Current"] {
        if let Some(id) = raw
            .pointer(&format!("/Pages/{key}"))
            .and_then(Value::as_str)
            && !pages.iter().any(|p| p == id)
        {
            pages.push(id.to_owned());
        }
    }
    let selected = if selected == Some("default") {
        Some(
            raw.pointer("/Pages/Default")
                .and_then(Value::as_str)
                .context("Profile has no default page")?,
        )
    } else {
        selected
    };
    let page = selected
        .or_else(|| raw.pointer("/Pages/Current").and_then(Value::as_str))
        .or_else(|| raw.pointer("/Pages/Default").and_then(Value::as_str))
        .or_else(|| pages.first().map(String::as_str))
        .map(str::to_owned);
    let (page, page_root, page_data) =
        if raw.get("Controllers").is_some() || raw.get("Actions").is_some() {
            ensure!(
                selected.is_none(),
                "A page-only profile cannot select another page"
            );
            (String::new(), root.clone(), raw.clone())
        } else {
            let page = page.context("Profile has no page selection")?;
            safe_path(&page)?;
            let mut candidates = vec![format!("{root}Profiles/{page}/")];
            if let Some(encoded) = encoded_uuid(&page) {
                candidates.push(format!("{root}Profiles/{encoded}/"));
            }
            let page_root = candidates
                .into_iter()
                .find(|p| files.contains_key(&format!("{p}manifest.json")))
                .with_context(|| {
                    format!("Selected page {page} is missing; refusing to guess another page")
                })?;
            let data = json_file(&files, &format!("{page_root}manifest.json"))?;
            (page, page_root, data)
        };
    let mut slots = Vec::new();
    if let Some(controllers) = page_data.get("Controllers").and_then(Value::as_array) {
        for controller in controllers {
            let kind = controller
                .get("Type")
                .and_then(Value::as_str)
                .context("Controller has no Type")?;
            if !["Keypad", "Encoder"].contains(&kind) {
                warnings.push(format!("Unsupported controller {kind} is preserved"));
            }
            parse_slots(kind, &controller["Actions"], &mut slots)?;
        }
    } else if page_data.get("Actions").is_some() {
        parse_slots("Keypad", &page_data["Actions"], &mut slots)?;
    } else {
        bail!("Page contains no Controllers or Actions");
    }
    let mut occupied = HashSet::new();
    for slot in &slots {
        ensure!(
            occupied.insert((slot.controller.clone(), slot.column, slot.row)),
            "Duplicate slot {},{} in {}",
            slot.column,
            slot.row,
            slot.controller
        );
    }
    slots.sort_by_key(|s| (s.controller.clone(), s.row, s.column));
    Ok(LoadedProfile {
        name: raw["Name"]
            .as_str()
            .unwrap_or("Imported profile")
            .to_owned(),
        uuid,
        page,
        pages,
        slots,
        warnings,
        raw,
        page_raw: page_data,
        files,
        root,
        page_root,
    })
}

fn parse_slots(controller: &str, actions: &Value, slots: &mut Vec<Slot>) -> Result<()> {
    for (position, action) in actions
        .as_object()
        .context("Controller Actions must be an object")?
    {
        let (column, row) = position
            .split_once(',')
            .context("Action coordinate must be column,row")?;
        let column = column.parse::<u8>().context("Invalid action column")?;
        let row = row.parse::<u8>().context("Invalid action row")?;
        let uuid = action["UUID"]
            .as_str()
            .context("Action has no UUID")?
            .to_owned();
        let settings = action.get("Settings").cloned().unwrap_or_else(|| json!({}));
        ensure!(settings.is_object(), "Action settings must be an object");
        let states = action.get("States").cloned().unwrap_or_else(|| json!([]));
        ensure!(states.is_array(), "Action States must be an array");
        // Preserve image strings verbatim, but forbid filesystem escape syntax.
        for state in states.as_array().unwrap() {
            if let Some(image) = state
                .get("Image")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                safe_path(image)?;
            }
        }
        slots.push(Slot {
            controller: controller.to_owned(),
            column,
            row,
            uuid,
            settings,
            states,
            raw: action.clone(),
        });
    }
    Ok(())
}

/// Elgato fixtures encode UUID bytes as base32hex, skipping U, followed by Z.
/// Verified against all twelve page references in the six official profiles.
fn encoded_uuid(uuid: &str) -> Option<String> {
    let compact: String = uuid.chars().filter(|c| *c != '-').collect();
    if compact.len() != 32 || !compact.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&compact[i..i + 2], 16).ok())
        .collect();
    let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTVW";
    let (mut bits, mut buffer) = (0u32, 0u32);
    let mut encoded = String::new();
    for byte in bytes? {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            encoded.push(alphabet[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        encoded.push(alphabet[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    encoded.push('Z');
    Some(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../evidence/official-samples/lights-out/com.elgato.lightsout.sdPlugin")
            .join(name)
    }

    #[test]
    fn official_profiles_resolve_encoded_pages() {
        for (name, count) in [
            ("sdp", 8),
            ("sd-mini", 6),
            ("sd-mobile", 64),
            ("sd-neo", 8),
            ("sd", 15),
            ("sdxl", 32),
        ] {
            let path = fixture(&format!("{name}.streamDeckProfile"));
            let loaded = load(&path).unwrap();
            assert_eq!(loaded.name, "Lights Out");
            assert_eq!(loaded.slots.len(), count, "{name}");
            assert_eq!(loaded.slots[0].uuid, "com.elgato.lightsout.reset");
            assert_eq!(loaded.slots[0].settings, json!({}));
            assert!(load_page(&path, Some("default")).unwrap().slots.is_empty());
        }
    }

    #[test]
    fn preserves_unknown_dials_and_assets() {
        let mut files = BTreeMap::new();
        files.insert("manifest.json".into(),serde_json::to_vec(&json!({"Name":"Custom","Controllers":[{"Type":"Encoder","Actions":{"2,0":{"UUID":"unknown.action","Settings":{"value":42},"States":[{"Image":"Images/dial.png","Title":"Dial"}],"FutureField":true}}}]})).unwrap());
        files.insert("Images/dial.png".into(), vec![1, 2, 3]);
        let mut loaded = parse(files, None).unwrap();
        loaded.warn_unknown_actions(&HashSet::new());
        assert_eq!(loaded.slots[0].column, 2);
        assert_eq!(loaded.slots[0].raw["FutureField"], true);
        assert_eq!(loaded.slots[0].states[0]["Title"], "Dial");
        assert_eq!(loaded.asset("Images/dial.png").unwrap(), vec![1, 2, 3]);
        assert!(loaded.asset("../secret").is_err());
        assert_eq!(loaded.warnings.len(), 1);
    }

    #[test]
    fn refuses_archive_traversal() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file("../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"bad").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let path =
            std::env::temp_dir().join(format!("profile-traversal-{}.zip", std::process::id()));
        fs::write(&path, bytes).unwrap();
        let result = load(&path);
        fs::remove_file(path).unwrap();
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsafe profile path")
        );
    }

    #[test]
    fn rejects_duplicate_archive_names() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("duplicate.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for name in ["a", "b"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"value").unwrap();
        }
        zip.finish().unwrap();
        let mut bytes = fs::read(&path).unwrap();
        for index in 0..bytes.len() - 46 {
            if &bytes[index..index + 4] == b"PK\x01\x02" && bytes[index + 46] == b'b' {
                bytes[index + 46] = b'a';
            }
        }
        fs::write(&path, bytes).unwrap();
        assert!(
            load(&path)
                .unwrap_err()
                .to_string()
                .contains("Duplicate ZIP")
        );
    }

    #[test]
    fn directory_import_matches_archive() {
        let directory = tempfile::tempdir().unwrap();
        let mut archive =
            zip::ZipArchive::new(fs::File::open(fixture("sdp.streamDeckProfile")).unwrap())
                .unwrap();
        // This known official fixture is extracted only for a directory-format test.
        archive.extract(directory.path()).unwrap();
        let root = directory
            .path()
            .join("6B51B3B0-412D-476F-ACBC-8090BE7696E0.sdProfile");
        let loaded = load(&root).unwrap();
        assert_eq!(loaded.uuid, "6B51B3B0-412D-476F-ACBC-8090BE7696E0");
        assert_eq!(loaded.slots.len(), 8);
        assert!(load_page(&root, Some("not-a-page")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hostname", root.join("escape")).unwrap();
            assert!(load(&root).unwrap_err().to_string().contains("symlink"));
        }
    }

    #[test]
    fn rejects_cross_platform_paths() {
        for path in ["../x", "/x", "C:/x", "a\\..\\x", "a/./b", "a//b"] {
            assert!(safe_path(path).is_err());
        }
        assert!(safe_path("Images/key.png").is_ok());
        assert_eq!(
            encoded_uuid("37aea345-7227-420a-bff0-988d1b05aeaa").unwrap(),
            "6VNA6HBI4T10LFWGJ26HM1DEL8Z"
        );
    }

    #[test]
    #[ignore = "requires Elgato 7.6 installer artifacts; DECKFORGE_ELGATO_RAW overrides their directory"]
    fn windows_installer_plus_profiles() {
        let root = std::env::var_os("DECKFORGE_ELGATO_RAW")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../evidence/elgato-7.6/raw")
            });
        for (file, current, token) in [
            (
                "flsysF8Fq6CoTq162E1zc6fyeIlKcw",
                "fca9bcac-ff0f-4ccd-8811-9af6dc361030",
                "WIKRPB7W1T6CR20HJBRDODGG60Z",
            ),
            (
                "fls1.KgkvZ_osjTGmFQpIDeeaNhGPU",
                "17a52f6c-b5fc-4899-9349-762c97c844e0",
                "2VIIVR5LWH49J4Q9EOM9FI24S0Z",
            ),
        ] {
            let path = root.join(file);
            let loaded = load(&path).unwrap();
            assert_eq!(loaded.raw["Device"]["Model"], "20GBD9901");
            assert_eq!(loaded.name, "Default Profile");
            assert_eq!(loaded.page, current);
            assert_eq!(encoded_uuid(current).unwrap(), token);
            assert_eq!(
                loaded
                    .slots
                    .iter()
                    .filter(|s| s.controller == "Keypad")
                    .count(),
                6
            );
            assert_eq!(
                loaded
                    .slots
                    .iter()
                    .filter(|s| s.controller == "Encoder")
                    .count(),
                4
            );
            let apps = loaded
                .slots
                .iter()
                .find(|s| s.controller == "Keypad" && s.column == 0 && s.row == 0)
                .unwrap();
            assert_eq!(apps.uuid, "com.elgato.streamdeck.profile.openchild");
            assert_eq!(apps.states[0]["Title"], "Apps");
            let image = loaded
                .asset(apps.states[0]["Image"].as_str().unwrap())
                .unwrap();
            assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
            // Folder references use the same UUID mapping, and contain preserved Windows paths.
            let child = load_page(&path, apps.settings["ProfileUUID"].as_str()).unwrap();
            assert!(
                child
                    .slots
                    .iter()
                    .any(|s| s.uuid == "com.elgato.streamdeck.system.open")
            );
            assert!(load_page(&path, Some("default")).unwrap().slots.is_empty());
            // Validate every embedded page, including hidden folders and their image resources.
            let page_ids: Vec<_> = loaded
                .files
                .keys()
                .filter_map(|p| p.strip_prefix(&format!("{}Profiles/", loaded.root)))
                .filter_map(|p| p.strip_suffix("/manifest.json"))
                .map(str::to_owned)
                .collect();
            assert_eq!(page_ids.len(), 8);
            for id in page_ids {
                let page = load_page(&path, Some(&id)).unwrap();
                for slot in &page.slots {
                    for state in slot.states.as_array().unwrap() {
                        if let Some(image) = state["Image"].as_str().filter(|p| !p.is_empty()) {
                            assert!(!page.asset(image).unwrap().is_empty());
                        }
                    }
                }
            }
        }
    }
}
