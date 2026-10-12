//! Profile format inferred from Elgato's own Lights Out fixtures. No host code reused.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::Path,
    sync::Arc,
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
    files: Arc<BTreeMap<String, Vec<u8>>>,
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

    /// Archive path of this page's manifest, for lossless export updates.
    pub fn manifest_path(&self) -> String {
        format!("{}manifest.json", self.page_root)
    }
}

/// The entire profile, including hidden folder pages and untouched archive assets.
/// Each page shares the same immutable archive storage, so selecting a page does
/// not reread the ZIP or copy its images.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileBundle {
    pub name: String,
    pub uuid: String,
    pub raw: Value,
    pub default_page: String,
    pub current_page: String,
    /// Declared top-level page order. Folder/default pages remain in `pages`.
    pub page_order: Vec<String>,
    pub pages: Vec<LoadedProfile>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    files: Arc<BTreeMap<String, Vec<u8>>>,
}

impl ProfileBundle {
    pub fn page(&self, id: &str) -> Option<&LoadedProfile> {
        let id = if id == "default" {
            self.default_page.clone()
        } else {
            canonical_page_id(id)
        };
        self.pages.iter().find(|p| p.page == id)
    }

    pub fn asset(&self, page: &str, path: &str) -> Result<Vec<u8>> {
        self.page(page)
            .with_context(|| format!("Unknown profile page {page}"))?
            .asset(path)
    }

    /// Resolve an exported resource token through its registry. Return the safe
    /// full archive path and original bytes, not the stale source machine path.
    pub fn resource(&self, id: &str) -> Result<(String, Vec<u8>)> {
        safe_path(id)?;
        ensure!(!id.contains('/'), "Resource ID must be one path component");
        let registry = json_file(&self.files, "Resources/manifest.json")?;
        let relative = registry["resources"]
            .get(id)
            .and_then(|entry| entry.get("file"))
            .and_then(Value::as_str)
            .with_context(|| format!("Resource {id} has no file mapping"))?;
        safe_path(relative)?;
        let path = if relative.starts_with("Resources/") {
            relative.to_owned()
        } else {
            format!("Resources/{relative}")
        };
        ensure!(
            path != "Resources/manifest.json",
            "Resource cannot reference its registry"
        );
        let bytes = self
            .files
            .get(&path)
            .with_context(|| format!("Resource {id} archive file is missing: {path}"))?;
        Ok((path, bytes.clone()))
    }

    pub fn manifest_path(&self) -> String {
        format!("{}manifest.json", self.pages[0].root)
    }

    /// Original safe archive paths and bytes, including unknown files. Exporters
    /// can replace selected manifest entries while retaining every other file.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_slice()))
    }
}

pub fn load_bundle(path: &Path) -> Result<ProfileBundle> {
    let mut bundle = parse_bundle(read_files(path)?)?;
    if bundle.uuid.is_empty() && path.is_dir() {
        bundle.uuid = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .trim_end_matches(".sdProfile")
            .to_owned();
        for page in &mut bundle.pages {
            page.uuid = bundle.uuid.clone();
        }
    }
    Ok(bundle)
}

/// Load the currently selected page, falling back to the default page.
pub fn load(path: &Path) -> Result<LoadedProfile> {
    load_page(path, None)
}

/// Select a page by UUID, encoded directory name, or the literal "default".
pub fn load_page(path: &Path, selected: Option<&str>) -> Result<LoadedProfile> {
    select_page(&load_bundle(path)?, selected)
}

fn select_page(bundle: &ProfileBundle, selected: Option<&str>) -> Result<LoadedProfile> {
    if bundle.pages.len() == 1 && bundle.pages[0].page.is_empty() {
        ensure!(
            selected.is_none(),
            "A page-only profile cannot select another page"
        );
    }
    let page = selected.unwrap_or(&bundle.current_page);
    bundle
        .page(page)
        .cloned()
        .with_context(|| format!("Selected page {page} is missing; refusing to guess another page"))
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

#[cfg(test)]
fn parse(files: BTreeMap<String, Vec<u8>>, selected: Option<&str>) -> Result<LoadedProfile> {
    select_page(&parse_bundle(files)?, selected)
}

fn parse_bundle(files: BTreeMap<String, Vec<u8>>) -> Result<ProfileBundle> {
    let roots: Vec<_> = files
        .keys()
        .filter(|p| {
            p.as_str() == "manifest.json"
                || (p.ends_with(".sdProfile/manifest.json") && p.matches('/').count() == 1)
                || (p.starts_with("Profiles/")
                    && p.ends_with(".sdProfile/manifest.json")
                    && p.matches('/').count() == 2)
        })
        .cloned()
        .collect();
    ensure!(
        roots.len() == 1,
        "Expected one profile root manifest, found {}",
        roots.len()
    );
    let raw = json_file(&files, &roots[0])?;
    if roots[0].starts_with("Profiles/") {
        let package = json_file(&files, "package.json")?;
        ensure!(
            package["FormatVersion"] == 1,
            "Unsupported wrapped profile package version"
        );
    }
    let root = roots[0].trim_end_matches("manifest.json").to_owned();
    let uuid = root
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".sdProfile")
        .to_owned();
    let name = raw["Name"]
        .as_str()
        .unwrap_or("Imported profile")
        .to_owned();
    let files = Arc::new(files);
    let page_order: Vec<String> = raw
        .pointer("/Pages/Pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(canonical_page_id)
        .collect();
    let mut warnings = Vec::new();
    let mut pages = Vec::new();
    if raw.get("Controllers").is_some() || raw.get("Actions").is_some() {
        pages.push(make_page(
            &name,
            &uuid,
            "",
            &[],
            raw.clone(),
            raw.clone(),
            files.clone(),
            &root,
            &root,
        )?);
    } else {
        let prefix = format!("{root}Profiles/");
        let mut seen = HashSet::new();
        for manifest in files.keys() {
            let Some(directory) = manifest
                .strip_prefix(&prefix)
                .and_then(|p| p.strip_suffix("/manifest.json"))
                .filter(|p| !p.is_empty() && !p.contains('/'))
            else {
                continue;
            };
            let id = canonical_page_id(directory);
            ensure!(seen.insert(id.clone()), "Ambiguous profile page ID {id}");
            let page_root = manifest.trim_end_matches("manifest.json");
            let page_raw = json_file(&files, manifest)?;
            pages.push(make_page(
                &name,
                &uuid,
                &id,
                &page_order,
                raw.clone(),
                page_raw,
                files.clone(),
                &root,
                page_root,
            )?);
        }
        ensure!(!pages.is_empty(), "Profile contains no embedded pages");
        // Present declared pages first, then every hidden page in canonical order.
        pages.sort_by_key(|p| {
            (
                page_order
                    .iter()
                    .position(|id| id == &p.page)
                    .unwrap_or(usize::MAX),
                p.page.clone(),
            )
        });
    }
    let default_page = raw
        .pointer("/Pages/Default")
        .and_then(Value::as_str)
        .map(canonical_page_id)
        .unwrap_or_else(|| pages[0].page.clone());
    let mut current_page = raw
        .pointer("/Pages/Current")
        .and_then(Value::as_str)
        .map(canonical_page_id)
        .unwrap_or_else(|| default_page.clone());
    if current_page == "00000000-0000-0000-0000-000000000000"
        && !pages.iter().any(|page| page.page == current_page)
    {
        current_page = page_order
            .first()
            .context("Zero Current page requires a declared first page")?
            .clone();
        warnings.push(format!(
            "Current page is the zero UUID sentinel; selected first declared page {current_page}"
        ));
    }
    for id in page_order.iter().chain([&default_page, &current_page]) {
        ensure!(
            pages.iter().any(|page| &page.page == id),
            "Referenced profile page {id} is missing"
        );
    }
    // Retain the old selected-page API's list of top-level/default/current IDs.
    if !pages[0].page.is_empty() {
        let mut compatibility_pages = page_order.clone();
        for id in [&default_page, &current_page] {
            if !compatibility_pages.contains(id) {
                compatibility_pages.push(id.clone());
            }
        }
        for page in &mut pages {
            page.pages = compatibility_pages.clone();
        }
    }
    Ok(ProfileBundle {
        name,
        uuid,
        raw,
        default_page,
        current_page,
        page_order,
        pages,
        warnings,
        files,
    })
}

#[allow(clippy::too_many_arguments)]
fn make_page(
    name: &str,
    uuid: &str,
    page: &str,
    pages: &[String],
    raw: Value,
    page_data: Value,
    files: Arc<BTreeMap<String, Vec<u8>>>,
    root: &str,
    page_root: &str,
) -> Result<LoadedProfile> {
    let mut slots = Vec::new();
    let mut warnings = Vec::new();
    if let Some(controllers) = page_data.get("Controllers").and_then(Value::as_array) {
        for controller in controllers {
            let kind = controller
                .get("Type")
                .and_then(Value::as_str)
                .unwrap_or("{Unknown}");
            if !["Keypad", "Encoder"].contains(&kind) {
                warnings.push(format!("Unsupported controller {kind} is preserved"));
                let mut unknown_slots = Vec::new();
                // Unknown controller schemas must survive even when their action
                // coordinates/settings differ from current key/dial conventions.
                match parse_slots(kind, &controller["Actions"], &mut unknown_slots) {
                    Ok(()) => slots.extend(unknown_slots),
                    Err(error) => warnings.push(format!(
                        "Controller {kind} remains in raw page data: {error}"
                    )),
                }
            } else {
                parse_slots(kind, &controller["Actions"], &mut slots)?;
            }
        }
    } else if page_data.get("Actions").is_some() {
        parse_slots("Keypad", &page_data["Actions"], &mut slots)?;
    } else {
        warnings.push(
            "Page has no recognized Controllers or Actions; raw page is preserved".to_owned(),
        );
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
        name: name.to_owned(),
        uuid: uuid.to_owned(),
        page: page.to_owned(),
        pages: pages.to_vec(),
        slots,
        warnings,
        raw,
        page_raw: page_data,
        files,
        root: root.to_owned(),
        page_root: page_root.to_owned(),
    })
}

/// Normalize UUID references and Elgato's encoded page-directory tokens. Opaque
/// non-UUID IDs stay verbatim, so future profile formats retain their identity.
pub fn canonical_page_id(id: &str) -> String {
    decoded_uuid(id)
        .or_else(|| uuid::Uuid::parse_str(id).ok().map(|u| u.to_string()))
        .unwrap_or_else(|| id.to_owned())
}

fn decoded_uuid(token: &str) -> Option<String> {
    let token = token.to_ascii_uppercase();
    let encoded = token.strip_suffix('Z')?;
    if encoded.len() != 26 {
        return None;
    }
    let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTVW";
    let (mut buffer, mut bits) = (0u32, 0u32);
    let mut bytes = Vec::with_capacity(16);
    for ch in encoded.bytes() {
        let value = alphabet.iter().position(|&c| c == ch)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    let id = uuid::Uuid::from_slice(&bytes).ok()?.to_string();
    // The final two bits are padding; reject aliases with nonzero padding.
    (encoded_uuid(&id)? == token).then_some(id)
}

fn parse_slots(controller: &str, actions: &Value, slots: &mut Vec<Slot>) -> Result<()> {
    if actions.is_null() {
        return Ok(());
    }
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
        let settings = action
            .get("Settings")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
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
pub(crate) fn encoded_uuid(uuid: &str) -> Option<String> {
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

    #[test]
    fn wrapped_resource_registry_and_current_sentinel_are_strict() {
        let id = "37aea345-7227-420a-bff0-988d1b05aeaa";
        let root = "Profiles/example.sdProfile/manifest.json";
        let mut files=BTreeMap::from([
            ("package.json".into(),br#"{"FormatVersion":1}"#.to_vec()),
            (root.into(),serde_json::to_vec(&json!({"Name":"Wrapped","Version":"3.0","Pages":{"Default":id,"Current":"00000000-0000-0000-0000-000000000000","Pages":[id]}})).unwrap()),
            (format!("Profiles/example.sdProfile/Profiles/{id}/manifest.json"),br#"{"Controllers":[{"Type":"Keypad","Actions":null}]}"#.to_vec()),
            ("Resources/manifest.json".into(),br#"{"resources":{"media":{"file":"media/sound.wav"}}}"#.to_vec()),
            ("Resources/media/sound.wav".into(),b"original-media".to_vec()),
        ]);
        let bundle = parse_bundle(files.clone()).unwrap();
        assert_eq!(bundle.uuid, "example");
        assert_eq!(bundle.current_page, id);
        assert!(
            bundle
                .warnings
                .iter()
                .any(|w| w.contains("zero UUID sentinel"))
        );
        assert_eq!(
            bundle.resource("media").unwrap(),
            (
                "Resources/media/sound.wav".into(),
                b"original-media".to_vec()
            )
        );
        assert!(bundle.resource("../media").is_err());
        assert!(bundle.resource("missing").is_err());
        for path in [
            "../outside.wav",
            "/etc/passwd",
            "C:\\media.wav",
            "Resources/manifest.json",
            "media/missing.wav",
        ] {
            files.insert(
                "Resources/manifest.json".into(),
                serde_json::to_vec(&json!({"resources":{"media":{"file":path}}})).unwrap(),
            );
            assert!(
                parse_bundle(files.clone())
                    .unwrap()
                    .resource("media")
                    .is_err(),
                "{path}"
            );
        }
        files.insert(
            "Resources/manifest.json".into(),
            br#"{"resources":{"media":{"file":"Resources/media/sound.wav"}}}"#.to_vec(),
        );
        assert!(
            parse_bundle(files.clone())
                .unwrap()
                .resource("media")
                .is_ok()
        );
        let mut raw: Value = serde_json::from_slice(&files[root]).unwrap();
        raw["Pages"]["Current"] = json!("missing-page");
        files.insert(root.into(), serde_json::to_vec(&raw).unwrap());
        assert!(
            parse_bundle(files)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
    }

    #[test]
    #[ignore = "requires Elgato 7.6 resource profile artifacts"]
    fn actual_wrapped_soundboard_resources_match_original_mp3_bytes() {
        let root = std::env::var_os("DECKFORGE_ELGATO_RAW")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../evidence/elgato-7.6/raw")
            });
        let mut count = 0;
        for file in [
            "flsKaSPXYxUUvmKtC3TdqgYiqQcKKY",
            "flsLUxNAo3HxRBo8yay7EbNtFyJC9c",
        ] {
            let path = root.join(file);
            let bundle = load_bundle(&path).unwrap();
            assert!(bundle.manifest_path().starts_with("Profiles/"));
            assert!(!bundle.uuid.contains('/'));
            assert_eq!(bundle.current_page, bundle.page_order[0]);
            assert!(
                bundle
                    .warnings
                    .iter()
                    .any(|w| w.contains("zero UUID sentinel"))
            );
            let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
            let registry: Value =
                serde_json::from_reader(archive.by_name("Resources/manifest.json").unwrap())
                    .unwrap();
            for action in bundle
                .pages
                .iter()
                .flat_map(|p| &p.slots)
                .filter(|s| s.uuid == "com.elgato.streamdeck.soundboard.playaudio")
            {
                let id = action.raw["Resources"]["file"].as_str().unwrap();
                let expected = format!(
                    "Resources/{}",
                    registry["resources"][id]["file"].as_str().unwrap()
                );
                let mut original = Vec::new();
                archive
                    .by_name(&expected)
                    .unwrap()
                    .read_to_end(&mut original)
                    .unwrap();
                assert_eq!(bundle.resource(id).unwrap(), (expected, original));
                assert!(
                    action.settings["path"]
                        .as_str()
                        .unwrap()
                        .starts_with("/Users/")
                );
                count += 1;
            }
        }
        assert_eq!(count, 15);
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/official-profiles")
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
    fn whole_official_profiles_share_storage_and_resolve_every_page_alias() {
        for name in ["sdp", "sd-mini", "sd-mobile", "sd-neo", "sd", "sdxl"] {
            let bundle = load_bundle(&fixture(&format!("{name}.streamDeckProfile"))).unwrap();
            assert_eq!(bundle.pages.len(), 2);
            assert_eq!(bundle.page_order, vec![bundle.current_page.clone()]);
            assert!(bundle.page("default").unwrap().slots.is_empty());
            assert!(bundle.warnings.is_empty());
            for page in &bundle.pages {
                assert!(Arc::ptr_eq(&bundle.files, &page.files));
                assert_eq!(page.page, canonical_page_id(&page.page.to_uppercase()));
                let encoded = encoded_uuid(&page.page).unwrap();
                assert_eq!(bundle.page(&encoded).unwrap().page, page.page);
                assert_eq!(
                    bundle.page(&page.page.to_uppercase()).unwrap().page,
                    page.page
                );
                let bytes = bundle
                    .files()
                    .find(|(path, _)| *path == page.manifest_path())
                    .unwrap()
                    .1;
                assert_eq!(
                    serde_json::from_slice::<Value>(bytes).unwrap(),
                    page.page_raw
                );
            }
            let root = bundle
                .files()
                .find(|(path, _)| *path == bundle.manifest_path())
                .unwrap()
                .1;
            assert_eq!(serde_json::from_slice::<Value>(root).unwrap(), bundle.raw);
        }
    }

    #[test]
    fn bundle_keeps_unlisted_folders_unknown_schemas_and_all_assets() {
        let current = "37aea345-7227-420a-bff0-988d1b05aeaa";
        let child = "75560e74-1759-4cf4-8cfb-91d738b906c4";
        let first = encoded_uuid(current).unwrap();
        let second = encoded_uuid(child).unwrap();
        let mut files = BTreeMap::new();
        files.insert(
            "manifest.json".into(),
            serde_json::to_vec(&json!({"Name":"Tree","FutureRoot":{"x":1},
            "Pages":{"Default":current,"Current":current,"Pages":[current]}}))
            .unwrap(),
        );
        files.insert(format!("Profiles/{first}/manifest.json"),serde_json::to_vec(&json!({"Controllers":[{"Type":"Keypad",
            "Actions":{"0,0":{"UUID":"com.elgato.streamdeck.profile.openchild","Settings":{"ProfileUUID":child},"States":[]}}}]})).unwrap());
        let unknown = json!({"FuturePage":true,"Controllers":[{"Type":"FutureTouch","Actions":[{"arbitrary":"format"}],"Unrecognized":7}]});
        files.insert(
            format!("Profiles/{second}/manifest.json"),
            serde_json::to_vec(&unknown).unwrap(),
        );
        files.insert(format!("Profiles/{second}/Images/icon.png"), vec![1, 2, 3]);
        files.insert("Images/global.png".into(), vec![4, 5, 6]);
        files.insert("Unrecognized/file.dat".into(), vec![7, 8]);
        let bundle = parse_bundle(files).unwrap();
        assert_eq!(bundle.pages.len(), 2);
        assert_eq!(bundle.page_order, vec![current]);
        assert_eq!(bundle.page(child).unwrap().page_raw, unknown);
        assert_eq!(bundle.raw["FutureRoot"]["x"], 1);
        assert_eq!(bundle.asset(child, "Images/icon.png").unwrap(), [1, 2, 3]);
        assert_eq!(bundle.asset(child, "Images/global.png").unwrap(), [4, 5, 6]);
        assert!(bundle.asset(child, "../Unrecognized/file.dat").is_err());
        assert!(
            bundle
                .files()
                .any(|(path, bytes)| path == "Unrecognized/file.dat" && bytes == [7, 8])
        );
        assert_eq!(
            bundle.page(current).unwrap().slots[0].settings["ProfileUUID"],
            child
        );
        assert!(bundle.page(child).unwrap().slots.is_empty());
        assert_eq!(bundle.page(child).unwrap().warnings.len(), 2);
        assert!(Arc::ptr_eq(&bundle.pages[0].files, &bundle.pages[1].files));
    }

    #[test]
    fn encoded_ids_reject_noncanonical_padding_and_ambiguous_pages() {
        let id = "37aea345-7227-420a-bff0-988d1b05aeaa";
        let encoded = encoded_uuid(id).unwrap();
        assert_eq!(decoded_uuid(&encoded).unwrap(), id);
        assert!(decoded_uuid("6VNA6HBI4T10LFWGJ26HM1DEL9Z").is_none());
        let mut files = BTreeMap::new();
        files.insert(
            "manifest.json".into(),
            serde_json::to_vec(&json!({"Pages":{"Current":id}})).unwrap(),
        );
        for directory in [id, &encoded] {
            files.insert(
                format!("Profiles/{directory}/manifest.json"),
                serde_json::to_vec(&json!({"Actions":{}})).unwrap(),
            );
        }
        assert!(
            parse_bundle(files)
                .unwrap_err()
                .to_string()
                .contains("Ambiguous profile page ID")
        );
    }

    #[test]
    fn preserves_unknown_dials_and_assets() {
        let mut files = BTreeMap::new();
        files.insert("manifest.json".into(),serde_json::to_vec(&json!({"Name":"Custom","Controllers":[{"Type":"Encoder","Actions":{"2,0":{"UUID":"unknown.action","Settings":{"value":42},"States":[{"Image":"Images/dial.png","Title":"Dial"}],"FutureField":true}}}]})).unwrap());
        files.insert("Images/dial.png".into(), vec![1, 2, 3]);
        let loaded = parse(files, None).unwrap();
        assert_eq!(loaded.slots[0].uuid, "unknown.action");
        assert_eq!(loaded.slots[0].column, 2);
        assert_eq!(loaded.slots[0].raw["FutureField"], true);
        assert_eq!(loaded.slots[0].states[0]["Title"], "Dial");
        assert_eq!(loaded.asset("Images/dial.png").unwrap(), vec![1, 2, 3]);
        assert!(loaded.asset("../secret").is_err());
        assert!(loaded.warnings.is_empty());
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
            let bundle = load_bundle(&path).unwrap();
            assert_eq!(bundle.pages.len(), 8);
            assert_eq!(bundle.page_order.len(), 2);
            let mut counts: Vec<_> = bundle.pages.iter().map(|p| p.slots.len()).collect();
            counts.sort_unstable();
            assert_eq!(
                counts,
                if file == "flsysF8Fq6CoTq162E1zc6fyeIlKcw" {
                    vec![0, 0, 5, 10, 10, 11, 11, 15]
                } else {
                    vec![0, 0, 7, 10, 10, 11, 11, 15]
                }
            );
            for page in &bundle.pages {
                assert!(uuid::Uuid::parse_str(&page.page).is_ok());
                assert!(Arc::ptr_eq(&page.files, &bundle.files));
                for action in &page.slots {
                    if action.uuid == "com.elgato.streamdeck.profile.openchild" {
                        let target = action.settings["ProfileUUID"].as_str().unwrap();
                        assert!(bundle.page(target).is_some(), "Missing folder {target}");
                    }
                }
                println!("{file} {}: {} slots", page.page, page.slots.len());
            }
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
