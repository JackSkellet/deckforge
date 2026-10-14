//! Pack changes keep recoverable predecessors and check the reviewed revisions.
use super::*;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};
const RECORD: &str = "transaction.json";
const VISIBILITY: &str = "preferences/visibility/hidden.json";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u8,
    id: String,
    name: String,
    version: String,
    icons: usize,
    kind: String,
    digest: String,
    dev: u64,
    ino: u64,
    candidate: Option<(u64, u64)>,
    created: u64,
    #[serde(default)]
    created_ns: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Archive {
    pub token: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub icons: usize,
    pub kind: String,
    pub created: u64,
    #[serde(skip_serializing)]
    created_ns: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub name: String,
    pub version: String,
    pub icons: usize,
}
impl From<&Pack> for Summary {
    fn from(p: &Pack) -> Self {
        Self {
            name: p.name.clone(),
            version: p.version.clone(),
            icons: p.icons.len(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub id: String,
    pub source: PathBuf,
    pub expected_revision: Option<String>,
    pub candidate_digest: String,
    pub current: Option<Summary>,
    pub replacement: Summary,
}
fn root(destination: &Path, id: &str) -> Result<PathBuf> {
    let name = format!("{id}{EXT}");
    ensure!(pack_id(&name)? == id, "Invalid icon pack ID");
    Ok(destination.join(name))
}
fn checked(root: &Path, expected: &str, reference: Option<&Path>) -> Result<Pack> {
    let p = load(root, reference)?;
    ensure!(
        p.revision == expected,
        "Icon pack changed; reload the library and try again"
    );
    Ok(p)
}
pub(super) fn visibility(destination: &Path) -> Result<BTreeSet<String>> {
    match fs::symlink_metadata(destination.join(VISIBILITY)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e.into()),
        Ok(_) => {
            let bytes = read(destination, VISIBILITY)?;
            ensure!(
                bytes.len() <= 64 * 1024,
                "Icon visibility preferences are too large"
            );
            let values: BTreeSet<String> = serde_json::from_slice(&bytes)?;
            ensure!(values.len() <= MAX_FILES, "Too many hidden icon packs");
            for id in &values {
                root(destination, id)?;
            }
            Ok(values)
        }
    }
}
pub fn hide(
    destination: &Path,
    id: &str,
    revision: &str,
    expected_hidden: bool,
    hidden: bool,
    reference: Option<&Path>,
) -> Result<()> {
    let _lock = StoreLock::acquire(destination)?;
    let pack = checked(&root(destination, id)?, revision, reference)?;
    ensure!(
        !hidden || pack.can_hide,
        "This pinned pack must stay visible"
    );
    let mut preferences = visibility(destination)?;
    ensure!(
        (preferences.contains(id) && pack.can_hide) == expected_hidden,
        "Pack visibility changed; reload the library"
    );
    if hidden {
        preferences.insert(id.into());
    } else {
        preferences.remove(id);
    }
    let bytes = serde_json::to_vec_pretty(&preferences)?;
    ensure!(
        bytes.len() <= 64 * 1024,
        "Icon visibility preferences are too large"
    );
    crate::resources::materialize(
        destination,
        "preferences",
        "visibility",
        "hidden.json",
        &bytes,
    )?;
    Ok(())
}
fn write_record(stage: &Path, record: &Record) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)?;
    ensure!(
        bytes.len() <= 64 * 1024,
        "Pack recovery record is too large"
    );
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(stage.join(RECORD))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::File::open(stage)?.sync_all()?;
    Ok(())
}
fn record(old: &Pack, path: &Path, kind: &str, candidate: Option<&Path>) -> Result<Record> {
    let meta = fs::symlink_metadata(path)?;
    let candidate = candidate
        .map(|p| fs::symlink_metadata(p).map(|m| (m.dev(), m.ino())))
        .transpose()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(Record {
        format: 1,
        id: old.id.clone(),
        name: old.name.clone(),
        version: old.version.clone(),
        icons: old.icons.len(),
        kind: kind.into(),
        digest: digest(path)?,
        dev: meta.dev(),
        ino: meta.ino(),
        candidate,
        created: now.as_secs(),
        created_ns: u64::try_from(now.as_nanos())?,
    })
}
fn archive_info(token: String, record: &Record) -> Archive {
    Archive {
        token,
        id: record.id.clone(),
        name: record.name.clone(),
        version: record.version.clone(),
        icons: record.icons,
        kind: record.kind.clone(),
        created: record.created,
        created_ns: record
            .created_ns
            .max(record.created.saturating_mul(1_000_000_000)),
    }
}
fn transaction(destination: &Path, token: &str) -> Result<PathBuf> {
    ensure!(
        token.starts_with(".deckforge-icons-") && !token.contains('/') && token.len() <= 255,
        "Invalid pack archive"
    );
    relative(token)?;
    let path = destination.join(token);
    let m = fs::symlink_metadata(&path)?;
    ensure!(
        m.is_dir()
            && !m.file_type().is_symlink()
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "Pack archive is not private"
    );
    Ok(path)
}
fn read_record(path: &Path) -> Result<Record> {
    let bytes = read(path, RECORD)?;
    ensure!(bytes.len() <= 64 * 1024, "Pack recovery record too large");
    let record: Record = serde_json::from_slice(&bytes)?;
    ensure!(
        record.format == 1
            && matches!(record.kind.as_str(), "update" | "remove")
            && record.icons <= MAX_FILES,
        "Invalid pack recovery record"
    );
    root(path, &record.id)?;
    Ok(record)
}
fn completed(path: &Path, record: &Record) -> Result<bool> {
    match fs::symlink_metadata(root(path, &record.id)?) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(m) => {
            ensure!(
                m.is_dir() && !m.file_type().is_symlink(),
                "Archived pack is not a directory"
            );
            let identity = (m.dev(), m.ino());
            if identity == (record.dev, record.ino) {
                return Ok(true);
            }
            ensure!(
                Some(identity) == record.candidate,
                "Archived pack directory changed"
            );
            Ok(false) // Prepared before an exchange that never committed.
        }
    }
}
pub(super) fn archives(destination: &Path, errors: &mut Vec<String>) -> Result<Vec<Archive>> {
    let mut result = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        let token = entry.file_name().to_string_lossy().into_owned();
        if !token.starts_with(".deckforge-icons-") {
            continue;
        }
        count += 1;
        ensure!(count <= 4096, "Too many icon pack recovery records");
        let inspected = (|| -> Result<Option<Archive>> {
            let path = transaction(destination, &token)?;
            if !path.join(RECORD).try_exists()? {
                return Ok(None);
            }
            let record = read_record(&path)?;
            Ok(completed(&path, &record)?.then(|| archive_info(token.clone(), &record)))
        })();
        match inspected {
            Ok(Some(archive)) => result.push(archive),
            Ok(None) => {}
            Err(e) => errors.push(format!("Pack recovery {token}: {e}")),
        }
    }
    result.sort_by(|a, b| b.created_ns.cmp(&a.created_ns).then(a.token.cmp(&b.token)));
    Ok(result)
}
pub fn remove(
    destination: &Path,
    id: &str,
    revision: &str,
    reference: Option<&Path>,
) -> Result<Archive> {
    let _lock = StoreLock::acquire(destination)?;
    let target = root(destination, id)?;
    let pack = checked(&target, revision, reference)?;
    ensure!(
        !pack.pinned,
        "Pinned packs cannot be removed; hide the pack instead"
    );
    let stage = tempfile::Builder::new()
        .prefix(".deckforge-icons-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(destination)?;
    let record = record(&pack, &target, "remove", None)?;
    write_record(stage.path(), &record)?;
    checked(&target, revision, reference)?;
    crate::package::rename_without_replacing(&target, &root(stage.path(), id)?)?;
    let retained = stage.keep();
    let _ = fs::File::open(destination).and_then(|f| f.sync_all());
    Ok(archive_info(
        retained.file_name().unwrap().to_string_lossy().into_owned(),
        &record,
    ))
}
fn review_locked(
    destination: &Path,
    id: &str,
    source: &Path,
    reference: Option<&Path>,
) -> Result<Review> {
    let target = root(destination, id)?;
    let current = match fs::symlink_metadata(&target) {
        Ok(_) => Some(load(&target, reference)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let staged = stage(source, destination, reference)?;
    ensure!(
        staged.pack.id == id,
        "Replacement pack ID does not match the selected pack"
    );
    Ok(Review {
        id: id.into(),
        source: source.canonicalize()?,
        expected_revision: current.as_ref().map(|p| p.revision.clone()),
        candidate_digest: digest(&staged.root)?,
        current: current.as_ref().map(Summary::from),
        replacement: Summary::from(&staged.pack),
    })
}
pub fn review(
    destination: &Path,
    id: &str,
    source: &Path,
    reference: Option<&Path>,
) -> Result<Review> {
    let _lock = StoreLock::acquire(destination)?;
    review_locked(destination, id, source, reference)
}
pub fn review_restore(destination: &Path, token: &str, reference: Option<&Path>) -> Result<Review> {
    let _lock = StoreLock::acquire(destination)?;
    let path = transaction(destination, token)?;
    let record = read_record(&path)?;
    ensure!(completed(&path, &record)?, "Pack archive was not committed");
    let source = root(&path, &record.id)?;
    ensure!(
        digest(&source)? == record.digest,
        "Archived pack content changed"
    );
    review_locked(destination, &record.id, &source, reference)
}
pub fn apply(destination: &Path, review: &Review, reference: Option<&Path>) -> Result<Pack> {
    let _lock = StoreLock::acquire(destination)?;
    let target = root(destination, &review.id)?;
    let current = if let Some(expected) = &review.expected_revision {
        Some(checked(&target, expected, reference)?)
    } else {
        ensure!(
            fs::symlink_metadata(&target).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "A pack with this ID was installed after review"
        );
        None
    };
    let mut staged = stage(&review.source, destination, reference)?;
    staged.pack.hidden = visibility(destination)
        .unwrap_or_default()
        .contains(&staged.pack.id)
        && staged.pack.can_hide;
    ensure!(
        staged.pack.id == review.id && digest(&staged.root)? == review.candidate_digest,
        "Replacement pack changed after review; choose it again"
    );
    if let Some(old) = current {
        let record = record(&old, &target, "update", Some(&staged.root))?;
        write_record(staged.stage.path(), &record)?;
        checked(
            &target,
            review.expected_revision.as_deref().unwrap(),
            reference,
        )?;
        let parent = fs::File::open(destination)?;
        let stage_dir = fs::File::open(staged.stage.path())?;
        crate::plugin_update::exchange(
            &parent,
            &format!("{}{EXT}", review.id),
            &stage_dir,
            &format!("{}{EXT}", review.id),
        )?;
        // No fallible operation between exchange and retaining the predecessor.
        let _ = staged.stage.keep();
        let _ = parent.sync_all();
        let _ = stage_dir.sync_all();
    } else {
        crate::package::rename_without_replacing(&staged.root, &target)?;
    }
    Ok(staged.pack)
}
pub fn selected_asset(
    destination: &Path,
    id: &str,
    path: &str,
    revision: &str,
    reference: Option<&Path>,
) -> Result<PathBuf> {
    let _lock = StoreLock::acquire(destination)?;
    let root = root(destination, id)?;
    let source = asset(destination, id, path, reference, Some(revision))?;
    let bytes = read(
        &root,
        source
            .strip_prefix(&root)?
            .to_str()
            .context("Invalid icon path")?,
    )?;
    ensure!(bytes.len() <= 8 * 1024 * 1024, "Action icon exceeds 8 MiB");
    let hash = format!("{:x}", Sha256::digest(&bytes));
    crate::resources::materialize(
        &destination
            .parent()
            .context("Icon library state directory missing")?
            .join("appearance-assets"),
        "icons",
        &hash,
        path,
        &bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(root: &Path) -> PathBuf {
        super::super::tests::fixture(root)
    }
    fn manifest(root: &Path, version: &str, pinned: bool, can_hide: bool) {
        let mut m: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        m["Version"] = json!(version);
        m["pinned"] = json!(pinned.to_string());
        m["canhide"] = json!(can_hide.to_string());
        fs::write(root.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    }
    #[test]
    fn hidden_preferences_persist_with_pinned_rules_and_stale_checks() {
        let dir = tempfile::tempdir().unwrap();
        let source = fixture(dir.path());
        manifest(&source, "1", true, true);
        let dest = dir.path().join("installed");
        let p = install(&source, &dest, None).unwrap();
        assert!(p.pinned && p.can_hide);
        hide(&dest, &p.id, &p.revision, false, true, None).unwrap();
        assert!(catalog(&dest, None).unwrap().packs[0].hidden);
        assert!(hide(&dest, &p.id, &p.revision, false, false, None).is_err());
        assert!(remove(&dest, &p.id, &p.revision, None).is_err());
        hide(&dest, &p.id, &p.revision, true, false, None).unwrap();
        assert!(!catalog(&dest, None).unwrap().packs[0].hidden);
        let other = dir.path().join("other");
        manifest(&source, "1", true, false);
        let p = install(&source, &other, None).unwrap();
        assert!(!p.can_hide);
        assert!(hide(&other, &p.id, &p.revision, false, true, None).is_err());
    }
    #[test]
    fn removal_and_restore_keep_selected_bytes_and_recoverable_archive() {
        let dir = tempfile::tempdir().unwrap();
        let source = fixture(dir.path());
        let dest = dir.path().join("installed");
        let p = install(&source, &dest, None).unwrap();
        let selected = selected_asset(&dest, &p.id, "first.svg", &p.revision, None).unwrap();
        let bytes = fs::read(&selected).unwrap();
        let archive = remove(&dest, &p.id, &p.revision, None).unwrap();
        assert_eq!(fs::read(&selected).unwrap(), bytes);
        assert!(!root(&dest, &p.id).unwrap().exists());
        let c = catalog(&dest, None).unwrap();
        assert!(c.packs.is_empty());
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(c.archives.len(), 1);
        assert_eq!(c.archives[0].token, archive.token);
        let restore = review_restore(&dest, &archive.token, None).unwrap();
        assert!(restore.current.is_none());
        let restored = apply(&dest, &restore, None).unwrap();
        assert_eq!(restored.id, p.id);
        assert_eq!(
            fs::read(root(&dest, &p.id).unwrap().join("icons/first.svg")).unwrap(),
            bytes
        );
        assert_eq!(catalog(&dest, None).unwrap().archives.len(), 1);
        assert!(apply(&dest, &restore, None).is_err());
    }
    #[test]
    fn replacement_rechecks_source_and_target_and_preserves_predecessor_for_restore() {
        let dir = tempfile::tempdir().unwrap();
        let source = fixture(dir.path());
        let dest = dir.path().join("installed");
        let p = install(&source, &dest, None).unwrap();
        let old = root(&dest, &p.id).unwrap();
        let old_digest = digest(&old).unwrap();
        let selected = selected_asset(&dest, &p.id, "first.svg", &p.revision, None).unwrap();
        let original = fs::read(&selected).unwrap();
        manifest(&source, "2", false, true);
        let review1 = review(&dest, &p.id, &source, None).unwrap();
        let changed = String::from_utf8(original.clone())
            .unwrap()
            .replace("#a03264", "#254679");
        fs::write(source.join("icons/first.svg"), changed).unwrap();
        assert!(apply(&dest, &review1, None).is_err());
        assert_eq!(digest(&old).unwrap(), old_digest);
        assert!(catalog(&dest, None).unwrap().archives.is_empty());
        let review2 = review(&dest, &p.id, &source, None).unwrap();
        fs::write(old.join("manual.txt"), b"changed installed pack").unwrap();
        assert!(apply(&dest, &review2, None).is_err());
        fs::remove_file(old.join("manual.txt")).unwrap();
        let review3 = review(&dest, &p.id, &source, None).unwrap();
        let replacement = apply(&dest, &review3, None).unwrap();
        assert_eq!(replacement.version, "2");
        assert_eq!(fs::read(selected).unwrap(), original);
        assert!(selected_asset(&dest, &p.id, "first.svg", &p.revision, None).is_err());
        let c = catalog(&dest, None).unwrap();
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(c.archives.len(), 1);
        assert_eq!(c.archives[0].kind, "update");
        assert_eq!(
            digest(&root(&dest.join(&c.archives[0].token), &p.id).unwrap()).unwrap(),
            old_digest
        );
        let restore = review_restore(&dest, &c.archives[0].token, None).unwrap();
        assert_eq!(restore.current.as_ref().unwrap().version, "2");
        let old = apply(&dest, &restore, None).unwrap();
        assert_eq!(old.version, "1.0");
        let archives = catalog(&dest, None).unwrap().archives;
        assert_eq!(archives.len(), 2);
        assert_eq!(archives[0].version, "2");
    }
    #[test]
    fn unfinished_exchange_is_ignored_then_recognized_after_atomic_swap() {
        let dir = tempfile::tempdir().unwrap();
        let source = fixture(dir.path());
        let dest = dir.path().join("installed");
        let p = install(&source, &dest, None).unwrap();
        let target = root(&dest, &p.id).unwrap();
        manifest(&source, "2", false, true);
        let candidate = stage(&source, &dest, None).unwrap();
        let r = record(&p, &target, "update", Some(&candidate.root)).unwrap();
        write_record(candidate.stage.path(), &r).unwrap();
        let retained = candidate.stage.keep();
        assert!(catalog(&dest, None).unwrap().archives.is_empty());
        crate::plugin_update::exchange(
            &fs::File::open(&dest).unwrap(),
            &format!("{}{EXT}", p.id),
            &fs::File::open(&retained).unwrap(),
            &format!("{}{EXT}", p.id),
        )
        .unwrap();
        let c = catalog(&dest, None).unwrap();
        assert_eq!(c.packs[0].version, "2");
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(c.archives.len(), 1);
        assert_eq!(c.archives[0].version, "1.0");
        fs::write(
            root(&retained, &p.id).unwrap().join("icons/first.svg"),
            b"modified backup",
        )
        .unwrap();
        assert!(review_restore(&dest, &c.archives[0].token, None).is_err());
    }
    #[test]
    fn library_lock_rejects_competing_writes_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let source = fixture(dir.path());
        let dest = dir.path().join("installed");
        let _lock = StoreLock::acquire(&dest).unwrap();
        assert!(
            install(&source, &dest, None)
                .unwrap_err()
                .to_string()
                .contains("busy")
        );
    }
}
