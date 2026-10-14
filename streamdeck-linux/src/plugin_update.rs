//! Inert managed-plugin update transactions. The caller holds the session lock
//! and stops the plugin before commit/restore/reapply. Directory inode checks
//! detect observed replacement, not adversarial concurrent rename races.
//!
//! Journal-before-exchange enables recovery after a process crash. This is not
//! a power-loss durability guarantee for every extracted file/filesystem.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
use tempfile::TempDir;

const JOURNAL: &str = "transaction.json";
const MAX_JOURNAL: u64 = 16 * 1024;
const MAX_RECORDS: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    dev: u64,
    ino: u64,
}
impl Identity {
    fn of(meta: &fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    state: Identity,
    transaction: Identity,
    target: String,
    candidate: String,
    old: Identity,
    new: Identity,
}
#[derive(Clone, Debug)]
pub struct Backup {
    state: PathBuf,
    transaction: PathBuf,
    target: PathBuf,
    path: PathBuf,
    journal: Journal,
}
pub struct Prepared {
    stage: Option<TempDir>,
    backup: Backup,
}
impl Prepared {
    pub fn new(
        archive: &Path,
        target: &Path,
        state: &Path,
        reference: Option<&Path>,
    ) -> Result<Self> {
        let managed = managed(state, target)?;
        let old_dir = child(&managed.plugins, &managed.name, true, false)?;
        let old = Identity::of(&old_dir.metadata()?);
        create_updates(&managed.state_dir)?;
        let updates = child(&managed.state_dir, "plugin-updates", true, true)?;
        let updates_path = managed.state.join("plugin-updates");
        ensure!(
            Identity::of(&fs::symlink_metadata(&updates_path)?)
                == Identity::of(&updates.metadata()?),
            "Update directory changed"
        );
        let stage = tempfile::Builder::new()
            .prefix("update-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&updates_path)?;
        let stage_dir = open_directory(stage.path(), true)?;
        let candidate = crate::package::install_with_reference(archive, stage.path(), reference)?;
        ensure!(
            candidate.parent() == Some(stage.path()),
            "Candidate is not a direct staged child"
        );
        let candidate_name = component(
            candidate
                .file_name()
                .and_then(|v| v.to_str())
                .context("Candidate name is not UTF-8")?,
        )?;
        let new_dir = child(&stage_dir, &candidate_name, true, false)?;
        let new = Identity::of(&new_dir.metadata()?);
        ensure!(
            old.dev == new.dev,
            "Atomic plugin updates require staging and installation on the same filesystem"
        );
        ensure!(old != new, "Update candidate aliases the installed plugin");
        let old_manifest = crate::package::read_installed_manifest(&managed.target, reference)?;
        let new_manifest = crate::package::read_installed_manifest(&candidate, reference)?;
        let id = old_manifest["UUID"]
            .as_str()
            .filter(|v| !v.is_empty())
            .context("Installed plugin UUID missing")?;
        ensure!(
            new_manifest["UUID"].as_str() == Some(id),
            "Update package UUID does not match the installed plugin"
        );
        let journal = Journal {
            version: 1,
            state: Identity::of(&managed.state_dir.metadata()?),
            transaction: Identity::of(&stage_dir.metadata()?),
            target: managed.name,
            candidate: candidate_name,
            old,
            new,
        };
        let bytes = serde_json::to_vec(&journal)?;
        ensure!(
            bytes.len() as u64 <= MAX_JOURNAL,
            "Update journal exceeds limit"
        );
        let mut file = create_journal(&stage_dir)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        stage_dir.sync_all()?;
        updates.sync_all()?;
        let backup = Backup {
            state: managed.state,
            transaction: stage.path().to_owned(),
            target: managed.target,
            path: candidate,
            journal,
        };
        Ok(Self {
            stage: Some(stage),
            backup,
        })
    }
    pub fn path(&self) -> &Path {
        &self.backup.path
    }
    pub fn commit(mut self) -> Result<Backup> {
        let (plugins, transaction) = self
            .backup
            .checked(self.backup.journal.old, self.backup.journal.new)?;
        // Allocate/clone everything before exchange. On success keep() is the
        // immediate next operation: TempDir must never delete the old version.
        let backup = self.backup.clone();
        let stage = self.stage.take().context("Update staging is unavailable")?;
        if let Err(error) = exchange(
            &plugins,
            &backup.journal.target,
            &transaction,
            &backup.journal.candidate,
        ) {
            self.stage = Some(stage);
            return Err(error);
        }
        let _ = stage.keep();
        // Failure here cannot turn a committed exchange into an Err. Process
        // recovery is proven by identity; power-loss guarantees are separate.
        let _ = plugins.sync_all();
        let _ = transaction.sync_all();
        Ok(backup)
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(stage) = self.stage.take() {
            // A replaced or already-exchanged candidate may contain the old
            // installation. Keep it rather than recursively removing unknown data.
            let safe = (|| -> Result<bool> {
                let dir = open_directory(stage.path(), true)?;
                if Identity::of(&dir.metadata()?) != self.backup.journal.transaction {
                    return Ok(false);
                }
                let candidate = child(&dir, &self.backup.journal.candidate, true, false)?;
                if Identity::of(&candidate.metadata()?) != self.backup.journal.new {
                    return Ok(false);
                }
                for entry in fs::read_dir(stage.path())? {
                    let name = entry?.file_name();
                    if name != JOURNAL && name != self.backup.journal.candidate.as_str() {
                        return Ok(false);
                    }
                }
                Ok(read_journal(&dir)? == self.backup.journal)
            })()
            .unwrap_or(false);
            if safe {
                drop(stage);
            } else {
                let _ = stage.keep();
            }
        }
    }
}
impl Backup {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn target(&self) -> &Path {
        &self.target
    }
    pub fn restore(&self) -> Result<()> {
        self.swap(self.journal.new, self.journal.old)
    }
    pub fn reapply(&self) -> Result<()> {
        self.swap(self.journal.old, self.journal.new)
    }
    fn swap(&self, target: Identity, candidate: Identity) -> Result<()> {
        let (plugins, transaction) = self.checked(target, candidate)?;
        exchange(
            &plugins,
            &self.journal.target,
            &transaction,
            &self.journal.candidate,
        )?;
        // The exchange succeeded; never report a later fsync error as no change.
        let _ = plugins.sync_all();
        let _ = transaction.sync_all();
        Ok(())
    }
    fn checked(&self, target: Identity, candidate: Identity) -> Result<(File, File)> {
        let managed = managed(&self.state, &self.target)?;
        ensure!(
            Identity::of(&managed.state_dir.metadata()?) == self.journal.state,
            "Plugin state directory changed"
        );
        let updates = child(&managed.state_dir, "plugin-updates", true, true)?;
        ensure!(
            self.transaction.parent() == Some(self.state.join("plugin-updates").as_path()),
            "Transaction escaped update directory"
        );
        let transaction_name = component(
            self.transaction
                .file_name()
                .and_then(|v| v.to_str())
                .context("Invalid transaction name")?,
        )?;
        let transaction = child(&updates, &transaction_name, true, true)?;
        ensure!(
            Identity::of(&transaction.metadata()?) == self.journal.transaction,
            "Update transaction directory changed"
        );
        ensure!(
            read_journal(&transaction)? == self.journal,
            "Update journal changed"
        );
        ensure!(
            managed.name == self.journal.target
                && self.path == self.transaction.join(&self.journal.candidate),
            "Update journal path mismatch"
        );
        let actual_target = child(&managed.plugins, &self.journal.target, true, false)?;
        let actual_candidate = child(&transaction, &self.journal.candidate, true, false)?;
        ensure!(
            Identity::of(&actual_target.metadata()?) == target,
            "Installed plugin directory changed"
        );
        ensure!(
            Identity::of(&actual_candidate.metadata()?) == candidate,
            "Staged plugin directory changed"
        );
        ensure!(
            target.dev == candidate.dev,
            "Plugin versions are on different filesystems"
        );
        Ok((managed.plugins, transaction))
    }
}
pub fn find_backup(target: &Path, state: &Path) -> Result<Option<Backup>> {
    let managed = managed(state, target)?;
    let updates_path = managed.state.join("plugin-updates");
    if !updates_path.try_exists()? {
        ensure!(
            fs::symlink_metadata(&updates_path).is_err(),
            "Update directory is a dangling symlink"
        );
        return Ok(None);
    }
    let updates = child(&managed.state_dir, "plugin-updates", true, true)?;
    let mut found = None;
    for (index, entry) in
        fs::read_dir(format!("/proc/self/fd/{}", updates.as_raw_fd()))?.enumerate()
    {
        ensure!(index < MAX_RECORDS, "Too many plugin update transactions");
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("update-") {
            continue;
        }
        let record = (|| -> Result<Backup> {
            let transaction = child(&updates, &component(&name)?, true, true)?;
            let journal = read_journal(&transaction)?;
            ensure!(journal.target == managed.name, "Different update target");
            let transaction_path = updates_path.join(&name);
            let backup = Backup {
                state: managed.state.clone(),
                target: managed.target.clone(),
                path: transaction_path.join(&journal.candidate),
                transaction: transaction_path,
                journal,
            };
            // Only the post-exchange orientation is a rollback. Pre-exchange
            // orphan journals (target=old, candidate=new) must never be offered.
            backup.checked(backup.journal.new, backup.journal.old)?;
            Ok(backup)
        })();
        // Incomplete pre-commit/corrupt records are inert history, never backups.
        if let Ok(backup) = record {
            ensure!(found.is_none(), "Ambiguous plugin rollback records");
            found = Some(backup);
        }
    }
    Ok(found)
}

struct Managed {
    state: PathBuf,
    state_dir: File,
    plugins: File,
    target: PathBuf,
    name: String,
}
fn managed(state: &Path, target: &Path) -> Result<Managed> {
    let state = state
        .canonicalize()
        .context("Resolve plugin state directory")?;
    let state_dir = open_directory(&state, false)?;
    let plugins = child(&state_dir, "plugins", true, false)?;
    let metadata = fs::symlink_metadata(target).context("Installed plugin unavailable")?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Update target must be a real managed directory"
    );
    let target = target.canonicalize()?;
    ensure!(
        target.parent() == Some(state.join("plugins").as_path()),
        "Updates require a direct managed state/plugins child"
    );
    let name = component(
        target
            .file_name()
            .and_then(|v| v.to_str())
            .context("Target name is not UTF-8")?,
    )?;
    let directory = child(&plugins, &name, true, false)?;
    ensure!(
        Identity::of(&directory.metadata()?) == Identity::of(&metadata),
        "Managed target changed"
    );
    Ok(Managed {
        state,
        state_dir,
        plugins,
        target,
        name,
    })
}
fn component(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty()
            && name.len() <= 255
            && name != "."
            && name != ".."
            && !name.contains(['/', '\\', '\0'])
            && !name.chars().any(char::is_control),
        "Unsafe update path component"
    );
    Ok(name.into())
}
fn owned(meta: &fs::Metadata, directory: bool, private: bool) -> Result<()> {
    ensure!(
        if directory {
            meta.is_dir()
        } else {
            meta.is_file()
        },
        "Invalid update file type"
    );
    // SAFETY: getuid reads the caller's real user identity.
    ensure!(
        meta.uid() == unsafe { libc::getuid() },
        "Update path is owned by another user"
    );
    ensure!(
        !private || meta.mode() & 0o077 == 0,
        "Update journal directory/file must be owner-private"
    );
    Ok(())
}
fn open_directory(path: &Path, private: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    owned(&file.metadata()?, true, private)?;
    Ok(file)
}
fn child(parent: &File, name: &str, directory: bool, private: bool) -> Result<File> {
    let name = CString::new(component(name)?)?;
    // SAFETY: pinned directory FD and validated C component. Returned FD is owned.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    ensure!(
        fd >= 0,
        "Cannot open managed update path: {}",
        std::io::Error::last_os_error()
    );
    let file = unsafe { File::from_raw_fd(fd) };
    owned(&file.metadata()?, directory, private)?;
    Ok(file)
}
fn create_updates(state: &File) -> Result<()> {
    // SAFETY: valid state directory FD and static C name.
    let rc = unsafe { libc::mkdirat(state.as_raw_fd(), c"plugin-updates".as_ptr(), 0o700) };
    if rc < 0 {
        let error = std::io::Error::last_os_error();
        ensure!(
            error.kind() == std::io::ErrorKind::AlreadyExists,
            "Create update directory: {error}"
        );
    }
    Ok(())
}
fn create_journal(parent: &File) -> Result<File> {
    // SAFETY: static C name, pinned directory. O_EXCL refuses collisions/symlinks.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c"transaction.json".as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    ensure!(fd >= 0, "Cannot create private update journal");
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn read_journal(parent: &File) -> Result<Journal> {
    let mut file = child(parent, JOURNAL, false, true)?;
    ensure!(
        file.metadata()?.len() <= MAX_JOURNAL,
        "Update journal exceeds limit"
    );
    let mut bytes = vec![];
    (&mut file).take(MAX_JOURNAL + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_JOURNAL,
        "Update journal exceeds limit"
    );
    let record: Journal = serde_json::from_slice(&bytes).context("Invalid update journal")?;
    ensure!(record.version == 1, "Unsupported update journal version");
    component(&record.target)?;
    component(&record.candidate)?;
    Ok(record)
}
pub(crate) fn exchange(a: &File, a_name: &str, b: &File, b_name: &str) -> Result<()> {
    let a_name = CString::new(component(a_name)?)?;
    let b_name = CString::new(component(b_name)?)?;
    // SAFETY: live parent FDs and validated names. Linux performs the directory
    // exchange atomically or leaves both unchanged; no recursive file copying.
    let result = unsafe {
        libc::renameat2(
            a.as_raw_fd(),
            a_name.as_ptr(),
            b.as_raw_fd(),
            b_name.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    ensure!(
        result == 0,
        "Atomic directory exchange failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    };
    const ID: &str = "com.example.update";
    struct Fixture {
        root: TempDir,
        state: PathBuf,
        target: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let state = root.path().join("state");
            let target = state.join("plugins/com.example.update.sdPlugin");
            fs::create_dir_all(&target).unwrap();
            fs::write(target.join("manifest.json"), manifest(ID, "old")).unwrap();
            fs::write(target.join("plugin.js"), "old").unwrap();
            Self {
                root,
                state,
                target,
            }
        }
        fn archive(&self, id: &str, version: &str) -> PathBuf {
            let path = self.root.path().join(format!("{version}.streamDeckPlugin"));
            let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
            for (name, bytes) in [
                ("manifest.json", manifest(id, version)),
                ("plugin.js", version.as_bytes().to_vec()),
            ] {
                writer
                    .start_file(
                        format!("{id}.sdPlugin/{name}"),
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
                writer.write_all(&bytes).unwrap();
            }
            writer.finish().unwrap();
            path
        }
        fn prepare(&self, version: &str) -> Prepared {
            Prepared::new(&self.archive(ID, version), &self.target, &self.state, None).unwrap()
        }
    }
    fn manifest(id: &str, version: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"UUID":id,"Name":"Inert update fixture","Version":version,"CodePath":"plugin.js","Actions":[]})).unwrap()
    }
    fn contents(path: &Path) -> String {
        fs::read_to_string(path.join("plugin.js")).unwrap()
    }
    #[test]
    fn stages_without_touching_target_and_drop_removes_only_incoming() {
        let f = Fixture::new();
        let before = Identity::of(&fs::metadata(&f.target).unwrap());
        let p = f.prepare("new");
        let stage = p.path().parent().unwrap().to_owned();
        assert_eq!(contents(p.path()), "new");
        assert_eq!(contents(&f.target), "old");
        assert_eq!(Identity::of(&fs::metadata(&f.target).unwrap()), before);
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        assert_eq!(fs::metadata(stage.join(JOURNAL)).unwrap().mode() & 0o077, 0);
        drop(p);
        assert!(!stage.exists());
        assert_eq!(contents(&f.target), "old");
    }
    #[test]
    fn commit_recovers_by_inode_then_restore_and_reapply_exchange_without_copying() {
        let f = Fixture::new();
        let p = f.prepare("new");
        let old = p.backup.journal.old;
        let new = p.backup.journal.new;
        let b = p.commit().unwrap();
        assert_eq!(b.target(), f.target);
        assert_eq!(contents(b.target()), "new");
        assert_eq!(contents(b.path()), "old");
        assert_eq!(Identity::of(&fs::metadata(b.path()).unwrap()), old);
        assert_eq!(Identity::of(&fs::metadata(b.target()).unwrap()), new);
        let recovered = find_backup(&f.target, &f.state).unwrap().unwrap();
        assert_eq!(recovered.path(), b.path());
        recovered.restore().unwrap();
        assert_eq!(contents(&f.target), "old");
        assert_eq!(contents(b.path()), "new");
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        assert!(recovered.restore().is_err());
        recovered.reapply().unwrap();
        assert_eq!(contents(&f.target), "new");
        assert_eq!(contents(b.path()), "old");
        assert!(recovered.reapply().is_err());
    }
    #[test]
    fn preswap_orphan_is_not_rollback_but_postswap_unwind_keeps_old_version() {
        let f = Fixture::new();
        let mut p = f.prepare("incoming");
        let orphan = p.stage.take().unwrap().keep();
        drop(p);
        assert!(orphan.exists());
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        let p = f.prepare("next");
        let b = p.backup.clone();
        let (target, stage) = b.checked(b.journal.old, b.journal.new).unwrap();
        exchange(&target, &b.journal.target, &stage, &b.journal.candidate).unwrap();
        // Simulates an unwind immediately after exchange, before explicit keep.
        drop(p);
        assert_eq!(contents(b.path()), "old");
        assert_eq!(contents(&f.target), "next");
        assert_eq!(
            find_backup(&f.target, &f.state).unwrap().unwrap().path(),
            b.path()
        );
    }
    #[test]
    fn competing_preparations_reject_changed_target_without_deleting_first_backup() {
        let f = Fixture::new();
        let one = f.prepare("one");
        let two = f.prepare("two");
        let discarded = two.path().parent().unwrap().to_owned();
        let b = one.commit().unwrap();
        assert!(two.commit().is_err());
        assert!(!discarded.exists());
        assert_eq!(contents(&f.target), "one");
        assert_eq!(contents(b.path()), "old");
        assert_eq!(
            find_backup(&f.target, &f.state).unwrap().unwrap().path(),
            b.path()
        );
    }
    #[test]
    fn staged_identity_replacement_and_extra_files_are_preserved_on_drop() {
        let f = Fixture::new();
        let p = f.prepare("one");
        let stage = p.path().parent().unwrap().to_owned();
        fs::rename(p.path(), stage.join("original-new")).unwrap();
        fs::create_dir(p.path()).unwrap();
        fs::write(p.path().join("keep"), "unrecognized version").unwrap();
        assert!(p.commit().is_err());
        assert!(stage.join("original-new").exists());
        assert_eq!(contents(&f.target), "old");
        let p = f.prepare("two");
        let stage = p.path().parent().unwrap().to_owned();
        fs::write(stage.join("unrelated"), "keep").unwrap();
        drop(p);
        assert!(stage.join("unrelated").exists());
    }
    #[test]
    fn rejects_foreign_uuid_external_target_symlinks_and_nonprivate_journals() {
        let f = Fixture::new();
        assert!(
            Prepared::new(
                &f.archive("com.example.other", "other"),
                &f.target,
                &f.state,
                None
            )
            .is_err()
        );
        let outside = f.root.path().join("external.sdPlugin");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("manifest.json"), manifest(ID, "old")).unwrap();
        assert!(Prepared::new(&f.archive(ID, "new"), &outside, &f.state, None).is_err());
        let alias = f.state.join("plugins/alias.sdPlugin");
        symlink(&f.target, &alias).unwrap();
        assert!(Prepared::new(&f.archive(ID, "new2"), &alias, &f.state, None).is_err());
        let p = f.prepare("three");
        let record = p.path().parent().unwrap().join(JOURNAL);
        fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(p.commit().is_err());
        assert_eq!(contents(&f.target), "old");
        let g = Fixture::new();
        symlink(&outside, g.state.join("plugin-updates")).unwrap();
        assert!(Prepared::new(&g.archive(ID, "new"), &g.target, &g.state, None).is_err());
    }
    #[test]
    fn corrupt_traversal_fifo_and_oversized_journals_never_become_backups() {
        let f = Fixture::new();
        let b = f.prepare("new").commit().unwrap();
        let path = b.transaction.join(JOURNAL);
        let original = fs::read(&path).unwrap();
        let mut record: serde_json::Value = serde_json::from_slice(&original).unwrap();
        record["candidate"] = json!("../outside");
        fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        assert!(b.restore().is_err());
        assert_eq!(contents(&f.target), "new");
        fs::write(&path, &original).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_JOURNAL + 1)
            .unwrap();
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        fs::remove_file(&path).unwrap();
        let outside = f.root.path().join("journal-copy.json");
        fs::write(&outside, &original).unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        assert!(b.restore().is_err());
        assert_eq!(fs::read(&outside).unwrap(), original);
        fs::remove_file(&path).unwrap();
        let c = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let start = std::time::Instant::now();
        assert!(find_backup(&f.target, &f.state).unwrap().is_none());
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
        assert_eq!(contents(b.path()), "old");
    }
    #[test]
    fn changed_installed_inode_blocks_restore_and_restoring_latest_exposes_predecessor() {
        let f = Fixture::new();
        let one = f.prepare("one").commit().unwrap();
        let two = f.prepare("two").commit().unwrap();
        assert_eq!(
            find_backup(&f.target, &f.state).unwrap().unwrap().path(),
            two.path()
        );
        two.restore().unwrap();
        assert_eq!(
            find_backup(&f.target, &f.state).unwrap().unwrap().path(),
            one.path()
        );
        fs::rename(&f.target, f.state.join("retained-other-version")).unwrap();
        fs::create_dir(&f.target).unwrap();
        fs::write(f.target.join("plugin.js"), "replacement").unwrap();
        assert!(one.restore().is_err());
        assert_eq!(contents(one.path()), "old");
        assert_eq!(contents(&f.target), "replacement");
    }
}
