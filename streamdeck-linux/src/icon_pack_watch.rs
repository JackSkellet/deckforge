//! Native library invalidation via Linux inotify, drained by the existing bridge
//! worker. No polling of icon bytes and no device/input thread or extra runtime.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::CString,
    fs::{self, File},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::PathBuf,
    time::{Duration, Instant},
};
const MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MODIFY
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;
const MAX_WATCHES: usize = 4096;
const QUIET: Duration = Duration::from_millis(100);
const MAX_DELAY: Duration = Duration::from_secs(1);
const RETRY: Duration = Duration::from_secs(5);
#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Parent,
    Root,
    Preferences,
    Visibility,
    Pack,
    Archive,
}
impl Role {
    fn relevant(self, name: &[u8]) -> bool {
        if name.is_empty() {
            return true;
        }
        match self {
            Self::Parent => name == b"icon-packs",
            Self::Root => {
                name.ends_with(b".sdIconPack")
                    || name == b"preferences"
                    || name.starts_with(b".deckforge-icons-")
            }
            Self::Preferences => name == b"visibility",
            Self::Visibility => name == b"hidden.json",
            Self::Pack => true,
            Self::Archive => name == b"transaction.json" || name.ends_with(b".sdIconPack"),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Directory {
    identity: (u64, u64),
    role: Role,
}
struct Watched {
    directory: Directory,
    descriptor: i32,
}
pub struct Watch {
    root: PathBuf,
    file: Option<File>,
    watches: BTreeMap<PathBuf, Watched>,
    snapshot: Snapshot,
    pending: Option<(Instant, Instant)>,
    retry: Instant,
    resync: bool,
}
impl Watch {
    pub fn new(root: PathBuf) -> Self {
        let mut this = Self {
            root,
            file: None,
            watches: BTreeMap::new(),
            snapshot: Snapshot::default(),
            pending: None,
            retry: Instant::now(),
            resync: false,
        };
        if let Err(e) = this.start() {
            this.failure(e);
        }
        this
    }
    fn failure(&mut self, error: anyhow::Error) {
        self.snapshot.error = Some(format!("Automatic icon library refresh: {error}"));
        self.retry = Instant::now() + RETRY;
    }
    fn start(&mut self) -> Result<()> {
        if self.file.is_none() {
            // SAFETY: inotify_init1 takes only flags and returns an owned fd.
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            ensure!(
                fd >= 0,
                "Cannot create filesystem watcher: {}",
                io::Error::last_os_error()
            );
            self.file = Some(unsafe { File::from_raw_fd(fd) });
        }
        self.rebuild()?;
        self.snapshot.error = None;
        Ok(())
    }
    fn watch_directory(
        &mut self,
        path: PathBuf,
        role: Role,
        desired: &mut HashSet<PathBuf>,
        seen: &mut HashSet<(u64, u64)>,
    ) -> Result<bool> {
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Ok(false);
        }
        let identity = (meta.dev(), meta.ino());
        if !seen.insert(identity) {
            return Ok(false);
        }
        ensure!(
            desired.len() < MAX_WATCHES,
            "Too many library directories to watch"
        );
        desired.insert(path.clone());
        let directory = Directory { identity, role };
        if self
            .watches
            .get(&path)
            .is_some_and(|w| w.directory == directory)
        {
            return Ok(true);
        }
        let fd = self
            .file
            .as_ref()
            .context("Watcher is unavailable")?
            .as_raw_fd();
        if let Some(previous) = self.watches.remove(&path) {
            // SAFETY: fd is live; already removed watches may return EINVAL.
            unsafe {
                libc::inotify_rm_watch(fd, previous.descriptor);
            }
        }
        ensure!(
            self.watches.len() < MAX_WATCHES,
            "Too many library directories to watch"
        );
        let name = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: live fd and NUL-terminated path. Reject non-directories and
        // observed symlinks. Watch before enumeration to cover new children.
        let descriptor = unsafe {
            libc::inotify_add_watch(
                fd,
                name.as_ptr(),
                MASK | libc::IN_ONLYDIR | libc::IN_DONT_FOLLOW,
            )
        };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOENT | libc::ENOTDIR | libc::ELOOP)
            ) {
                return Ok(false);
            }
            return Err(error).context("Cannot watch library directory");
        }
        self.watches.insert(
            path,
            Watched {
                directory,
                descriptor,
            },
        );
        Ok(true)
    }
    fn rebuild(&mut self) -> Result<()> {
        let mut desired = HashSet::new();
        let mut seen = HashSet::new();
        let root = self.root.clone();
        let parent = root.parent().context("Icon library parent missing")?;
        ensure!(
            self.watch_directory(parent.to_owned(), Role::Parent, &mut desired, &mut seen)?,
            "Icon library parent is unavailable"
        );
        if self.watch_directory(root.clone(), Role::Root, &mut desired, &mut seen)? {
            self.watch_directory(
                root.join("preferences"),
                Role::Preferences,
                &mut desired,
                &mut seen,
            )?;
            self.watch_directory(
                root.join("preferences/visibility"),
                Role::Visibility,
                &mut desired,
                &mut seen,
            )?;
            let entries = match fs::read_dir(&root) {
                Ok(entries) => Some(entries),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            for entry in entries.into_iter().flatten() {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.as_bytes();
                if name.starts_with(b".deckforge-icons-") {
                    self.watch_directory(entry.path(), Role::Archive, &mut desired, &mut seen)?;
                } else if name.ends_with(b".sdIconPack") {
                    let mut pending = vec![entry.path()];
                    while let Some(path) = pending.pop() {
                        if !self.watch_directory(
                            path.clone(),
                            Role::Pack,
                            &mut desired,
                            &mut seen,
                        )? {
                            continue;
                        }
                        let children = match fs::read_dir(path) {
                            Ok(children) => children,
                            Err(e)
                                if matches!(
                                    e.raw_os_error(),
                                    Some(libc::ENOENT | libc::ENOTDIR)
                                ) =>
                            {
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        };
                        for child in children {
                            let child = child?;
                            if child.file_type()?.is_dir() {
                                pending.push(child.path());
                            }
                        }
                    }
                }
            }
        }
        let fd = self
            .file
            .as_ref()
            .context("Watcher is unavailable")?
            .as_raw_fd();
        self.watches.retain(|path, watch| {
            if desired.contains(path) {
                true
            } else {
                unsafe {
                    libc::inotify_rm_watch(fd, watch.descriptor);
                }
                false
            }
        });
        Ok(())
    }
    fn consume(&mut self, bytes: &[u8]) -> Result<bool> {
        let mut changed = false;
        let mut offset = 0;
        while offset < bytes.len() {
            ensure!(
                bytes.len() - offset >= 16,
                "Truncated filesystem notification"
            );
            let wd = i32::from_ne_bytes(bytes[offset..offset + 4].try_into()?);
            let mask = u32::from_ne_bytes(bytes[offset + 4..offset + 8].try_into()?);
            let size = u32::from_ne_bytes(bytes[offset + 12..offset + 16].try_into()?) as usize;
            offset += 16;
            ensure!(size <= bytes.len() - offset, "Truncated filesystem name");
            let name = &bytes[offset..offset + size];
            let name = &name[..name.iter().position(|v| *v == 0).unwrap_or(name.len())];
            offset += size;
            if mask & libc::IN_Q_OVERFLOW != 0 {
                self.resync = true;
                changed = true;
                continue;
            }
            if let Some((path, watch)) = self.watches.iter().find(|(_, w)| w.descriptor == wd) {
                if watch.directory.role.relevant(name) {
                    changed = true;
                }
                if mask & libc::IN_IGNORED != 0 {
                    let path = path.clone();
                    self.watches.remove(&path);
                }
            }
        }
        Ok(changed)
    }
    fn drain(&mut self) -> Result<bool> {
        let mut changed = false;
        let mut bytes = [0u8; 32768];
        // Bounded work per bridge tick. A remaining backlog causes full resync.
        for _ in 0..16 {
            let count = match self
                .file
                .as_mut()
                .context("Watcher is unavailable")?
                .read(&mut bytes)
            {
                Ok(0) => return Ok(changed),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(changed),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            changed |= self.consume(&bytes[..count])?;
        }
        self.resync = true;
        Ok(true)
    }
    pub fn poll(&mut self) -> Snapshot {
        let now = Instant::now();
        if self.snapshot.error.is_some() && now >= self.retry {
            self.file = None;
            self.watches.clear();
            self.pending = None;
            self.resync = false;
            match self.start() {
                Ok(()) => {
                    self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
                }
                Err(e) => self.failure(e),
            }
        }
        if self.file.is_some() {
            match self.drain() {
                Ok(true) => match &mut self.pending {
                    Some((_, last)) => *last = now,
                    None => self.pending = Some((now, now)),
                },
                Ok(false) => {}
                Err(e) => {
                    self.file = None;
                    self.watches.clear();
                    self.pending = None;
                    self.failure(e);
                }
            }
        }
        if self.pending.is_some_and(|(first, last)| {
            now.duration_since(last) >= QUIET || now.duration_since(first) >= MAX_DELAY
        }) {
            self.pending = None;
            let rebuild = if self.resync {
                self.file = None;
                self.watches.clear();
                self.resync = false;
                self.start()
            } else {
                self.rebuild()
            };
            match rebuild {
                Ok(()) => self.snapshot.error = None,
                Err(e) => self.failure(e),
            }
            self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        }
        self.snapshot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;
    fn source(base: &Path, version: &str) -> PathBuf {
        let root = base.join("org.test.watch.sdIconPack");
        fs::create_dir_all(root.join("icons/nested")).unwrap();
        fs::write(root.join("manifest.json"),serde_json::to_vec(&json!({"Name":"Watched","Version":version,"Author":"Fixture","Icon":"icons/nested/a.svg"})).unwrap()).unwrap();
        fs::write(
            root.join("icons/nested/a.svg"),
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>"#,
        )
        .unwrap();
        root
    }
    fn changed(watch: &mut Watch, revision: u64) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let next = watch.poll();
            assert!(next.error.is_none(), "{:?}", next.error);
            if next.revision > revision {
                return next;
            }
            assert!(Instant::now() < deadline, "No filesystem change observed");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn stable(watch: &mut Watch, revision: u64) {
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            assert_eq!(watch.poll().revision, revision);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn watches_late_creation_assets_and_visibility_but_not_reads_or_profile_files() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let store = state.join("icon-packs");
        let mut watch = Watch::new(store.clone());
        assert!(watch.poll().error.is_none());
        fs::write(state.join("session.json"), "{}").unwrap();
        fs::create_dir(state.join("appearance-assets")).unwrap();
        stable(&mut watch, 0);
        let input = source(&temp.path().join("source"), "1");
        let p = crate::icon_pack::install(&input, &store, None).unwrap();
        let first = changed(&mut watch, 0);
        crate::icon_pack::catalog(&store, None).unwrap();
        fs::read(store.join("org.test.watch.sdIconPack/icons/nested/a.svg")).unwrap();
        stable(&mut watch, first.revision);
        crate::icon_pack::hide(&store, &p.id, &p.revision, false, true, None).unwrap();
        let next = changed(&mut watch, first.revision);
        crate::icon_pack::hide(&store, &p.id, &p.revision, true, false, None).unwrap();
        let next = changed(&mut watch, next.revision);
        use std::io::Write;
        let mut writer = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(store.join("org.test.watch.sdIconPack/icons/nested/a.svg"))
            .unwrap();
        writer.write_all(b"changed bytes").unwrap();
        writer.flush().unwrap();
        let next = changed(&mut watch, next.revision);
        stable(&mut watch, next.revision);
    }
    #[test]
    fn retargets_atomic_directory_replacement_and_observes_removal_recovery_record() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let store = state.join("icon-packs");
        let input = source(&temp.path().join("source"), "1");
        crate::icon_pack::install(&input, &store, None).unwrap();
        let mut watch = Watch::new(store.clone());
        source(&temp.path().join("source"), "2");
        let review = crate::icon_pack::review(&store, "org.test.watch", &input, None).unwrap();
        crate::icon_pack::apply(&store, &review, None).unwrap();
        let next = changed(&mut watch, 0);
        assert_eq!(
            crate::icon_pack::catalog(&store, None).unwrap().packs[0].version,
            "2"
        );
        fs::write(
            store.join("org.test.watch.sdIconPack/icons/nested/a.svg"),
            "new inode contents",
        )
        .unwrap();
        let next = changed(&mut watch, next.revision);
        let p = crate::icon_pack::catalog(&store, None)
            .unwrap()
            .packs
            .remove(0);
        crate::icon_pack::remove(&store, &p.id, &p.revision, None).unwrap();
        let next = changed(&mut watch, next.revision);
        let c = crate::icon_pack::catalog(&store, None).unwrap();
        assert!(c.packs.is_empty());
        assert_eq!(c.archives.len(), 2);
        let record = store.join(&c.archives[0].token).join("transaction.json");
        let original = fs::read(&record).unwrap();
        fs::write(record, original).unwrap();
        changed(&mut watch, next.revision);
    }
    #[test]
    fn ignores_symlink_targets_and_handles_overflow_and_truncated_events() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir_all(state.join("icon-packs")).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, state.join("icon-packs/linked.sdIconPack")).unwrap();
        let mut watch = Watch::new(state.join("icon-packs"));
        assert!(watch.poll().error.is_none());
        assert_eq!(watch.watches.len(), 2);
        fs::write(outside.join("image.svg"), "outside").unwrap();
        stable(&mut watch, 0);
        let mut event = vec![0u8; 16];
        event[..4].copy_from_slice(&(-1i32).to_ne_bytes());
        event[4..8].copy_from_slice(&libc::IN_Q_OVERFLOW.to_ne_bytes());
        assert!(watch.consume(&event).unwrap());
        event[4..8].copy_from_slice(&libc::IN_IGNORED.to_ne_bytes());
        assert!(!watch.consume(&event).unwrap());
        event[12..16].copy_from_slice(&4u32.to_ne_bytes());
        assert!(watch.consume(&event).is_err());
    }
    #[test]
    fn overflow_recreates_a_kernel_watch_whose_removed_event_was_lost() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let store = state.join("icon-packs");
        let input = source(&temp.path().join("source"), "1");
        crate::icon_pack::install(&input, &store, None).unwrap();
        let mut watch = Watch::new(store.clone());
        let directory = store.join("org.test.watch.sdIconPack/icons/nested");
        let descriptor = watch.watches[&directory].descriptor;
        // SAFETY: both descriptor and inotify fd belong only to this fixture.
        assert_eq!(
            unsafe { libc::inotify_rm_watch(watch.file.as_ref().unwrap().as_raw_fd(), descriptor) },
            0
        );
        let mut ignored = [0u8; 4096];
        let count = watch.file.as_mut().unwrap().read(&mut ignored).unwrap();
        assert!(count >= 16);
        // Discard IN_IGNORED deliberately, as can happen when a queue overflows.
        let mut event = [0u8; 16];
        event[..4].copy_from_slice(&(-1i32).to_ne_bytes());
        event[4..8].copy_from_slice(&libc::IN_Q_OVERFLOW.to_ne_bytes());
        assert!(watch.consume(&event).unwrap());
        let due = Instant::now() - QUIET;
        watch.pending = Some((due, due));
        let recovered = watch.poll();
        assert!(recovered.error.is_none());
        fs::write(directory.join("a.svg"), "after overflow").unwrap();
        changed(&mut watch, recovered.revision);
    }
}
