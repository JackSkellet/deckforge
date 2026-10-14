//! Read-only current-user executable inventory, sampled every 500 ms.
//! Source: https://docs.kernel.org/filesystems/proc.html (process directories,
//! stat start_time, exe, and lifetime of open proc directory descriptors).
//! Sampling can miss processes that begin and exit between polls. This is not
//! an audit log, and executable paths are deliberately not mapped to app IDs.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, OsStr},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const POLL: Duration = Duration::from_millis(500);
const MAX_PROCESSES: usize = 65_536;
const MAX_STAT: usize = 4096;
const MAX_PATH: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Process {
    pub pid: u32,
    /// Kernel start time in clock ticks since boot; not a wall-clock timestamp.
    pub started: u64,
    pub executable: PathBuf,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub processes: BTreeSet<Process>,
    pub error: Option<String>,
}
pub struct Worker {
    latest: Arc<Mutex<Option<Snapshot>>>,
    artwork_requests: Arc<Mutex<crate::application_artwork::Requests>>,
    artwork: Arc<Mutex<crate::application_artwork::Artwork>>,
    cancelled: Arc<AtomicBool>,
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn new() -> Self {
        // SAFETY: getuid has no preconditions and changes no process state.
        Self::start(PathBuf::from("/proc"), unsafe { libc::getuid() })
    }
    fn start(root: PathBuf, uid: u32) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = mpsc::channel();
        let artwork_requests = Arc::new(Mutex::new(Vec::new()));
        let requests = artwork_requests.clone();
        let artwork = Arc::new(Mutex::new(Default::default()));
        let visual_output = artwork.clone();
        let output = latest.clone();
        let cancel = cancelled.clone();
        let thread = thread::Builder::new()
            .name("deckforge-applications".into())
            .spawn(move || {
                let mut previous = Snapshot::default();
                let mut visual_cache = crate::application_artwork::Cache::default();
                loop {
                    match scan(&root, uid, &previous, &cancel) {
                        Ok(next) => previous = next,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => break,
                        Err(e) => previous.error = Some(format!("Application inventory: {e}")),
                    }
                    let requests = requests.lock().unwrap().clone();
                    let visuals = visual_cache.sample(&requests, &previous, &root);
                    *visual_output.lock().unwrap() = visuals;
                    *output.lock().unwrap() = Some(previous.clone());
                    if stopped.recv_timeout(POLL) != Err(mpsc::RecvTimeoutError::Timeout) {
                        break;
                    }
                }
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(e) => {
                *latest.lock().unwrap() = Some(Snapshot {
                    error: Some(format!("Application observer thread: {e}")),
                    ..Snapshot::default()
                });
                None
            }
        };
        Self {
            latest,
            artwork_requests,
            artwork,
            cancelled,
            stop,
            thread,
        }
    }
    pub fn watch_artwork(&self, requests: crate::application_artwork::Requests) {
        *self.artwork_requests.lock().unwrap() = requests;
    }
    pub fn artwork(&self) -> crate::application_artwork::Artwork {
        self.artwork.lock().unwrap().clone()
    }
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.latest.lock().unwrap().clone()
    }
}
impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Stat {
    started: u64,
    dead: bool,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn parse_stat(bytes: &[u8], expected_pid: u32) -> io::Result<Stat> {
    // comm may contain spaces, newlines, ')' and non-UTF8 bytes. Its final ') '
    // precedes the ASCII state/numeric fields; never split the whole record.
    let open = bytes
        .iter()
        .position(|b| *b == b'(')
        .ok_or_else(|| invalid("stat lacks comm"))?;
    let close = bytes
        .windows(2)
        .rposition(|s| s == b") ")
        .ok_or_else(|| invalid("stat lacks comm end"))?;
    if close < open {
        return Err(invalid("invalid stat comm"));
    }
    let pid = std::str::from_utf8(&bytes[..open])
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    if pid != Some(expected_pid) {
        return Err(invalid("stat PID mismatch"));
    }
    let tail =
        std::str::from_utf8(&bytes[close + 2..]).map_err(|_| invalid("invalid stat fields"))?;
    let mut fields = tail.split_ascii_whitespace();
    let state = fields
        .next()
        .filter(|s| s.len() == 1)
        .ok_or_else(|| invalid("missing stat state"))?;
    // State is field 3; starttime is field 22, hence skip fields 4..21.
    let started = fields
        .nth(18)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid("missing stat start time"))?;
    Ok(Stat {
        started,
        dead: matches!(state, "Z" | "X" | "x"),
    })
}
fn stat_at(dir: &File, pid: u32) -> io::Result<Stat> {
    // SAFETY: the directory FD and static C string are valid. The returned FD
    // is transferred exactly once into File. NONBLOCK rejects blocking FIFOs.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c"stat".as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(invalid("stat is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take((MAX_STAT + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_STAT {
        return Err(invalid("stat exceeds limit"));
    }
    parse_stat(&bytes, pid)
}
fn link_at(dir: &File, name: &CStr) -> io::Result<PathBuf> {
    let mut bytes = [0u8; MAX_PATH + 1];
    // SAFETY: valid FD/name and writable bounded buffer; readlinkat adds no NUL.
    let size = unsafe {
        libc::readlinkat(
            dir.as_raw_fd(),
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    if size == 0 || size as usize > MAX_PATH {
        return Err(invalid("executable path exceeds limit or is empty"));
    }
    let mut length = size as usize;
    const DELETED: &[u8] = b" (deleted)";
    if bytes[..length].ends_with(DELETED) {
        // proc's exe is a magic link: following it still reaches the executable
        // inode after unlink. Text alone is ambiguous because a real filename
        // can end in this suffix. Strip exactly one only for an unlinked inode.
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: live directory FD, valid name, correctly sized writable stat;
        // flags=0 follows exe. The output is read only after successful fstatat.
        if unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), metadata.as_mut_ptr(), 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { metadata.assume_init() }.st_nlink == 0 {
            length -= DELETED.len();
        }
    }
    let path = PathBuf::from(OsStr::from_bytes(&bytes[..length]));
    if !path.is_absolute() {
        return Err(invalid("executable path is not absolute"));
    }
    Ok(path)
}

enum Observation {
    Present(Process),
    Gone,
    Unreadable(Option<u64>),
}
fn observe(path: &Path, pid: u32, uid: u32) -> Observation {
    observe_with(path, pid, uid, || {})
}
fn observe_with(path: &Path, pid: u32, uid: u32, mut between: impl FnMut()) -> Observation {
    let dir = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Observation::Gone,
        Err(_) => return Observation::Unreadable(None),
    };
    match dir.metadata() {
        Ok(meta) if meta.uid() != uid => return Observation::Gone,
        Err(_) => return Observation::Unreadable(None),
        _ => {}
    }
    let first = match stat_at(&dir, pid) {
        Ok(stat) if stat.dead => return Observation::Gone,
        Ok(stat) => stat,
        Err(_) => {
            return if path.exists() {
                Observation::Unreadable(None)
            } else {
                Observation::Gone
            };
        }
    };
    let executable = link_at(&dir, c"exe");
    between();
    let second = match stat_at(&dir, pid) {
        Ok(stat) if stat.dead => return Observation::Gone,
        Ok(stat) => stat,
        Err(_) => {
            return if path.exists() {
                Observation::Unreadable(Some(first.started))
            } else {
                Observation::Gone
            };
        }
    };
    if first.started != second.started {
        return Observation::Unreadable(Some(second.started));
    }
    // A new exec keeps its start time. Require the executable to remain stable
    // too; retry on the next poll instead of reporting a half-observed change.
    match (executable, link_at(&dir, c"exe")) {
        (Ok(first), Ok(last)) if first == last => Observation::Present(Process {
            pid,
            started: second.started,
            executable: last,
        }),
        _ => Observation::Unreadable(Some(second.started)),
    }
}
fn scan(
    root: &Path,
    uid: u32,
    previous: &Snapshot,
    cancelled: &AtomicBool,
) -> io::Result<Snapshot> {
    let old: BTreeMap<_, _> = previous.processes.iter().map(|p| (p.pid, p)).collect();
    let mut processes = BTreeSet::new();
    let mut count = 0;
    let mut unreadable = 0;
    for entry in fs::read_dir(root)? {
        if cancelled.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|p| *p > 0)
        else {
            continue;
        };
        count += 1;
        if count > MAX_PROCESSES {
            return Err(invalid("process inventory exceeds 65536 entries"));
        }
        match observe(&entry.path(), pid, uid) {
            Observation::Present(process) => {
                processes.insert(process);
            }
            Observation::Gone => {}
            Observation::Unreadable(started) => {
                unreadable += 1;
                if let Some(old) = old
                    .get(&pid)
                    .filter(|old| started.is_none_or(|s| s == old.started))
                {
                    processes.insert((*old).clone());
                }
            }
        }
    }
    Ok(Snapshot {
        processes,
        error: (unreadable > 0)
            .then(|| format!("{unreadable} process identities were temporarily unreadable")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::symlink, process::Command, time::Instant};

    fn uid() -> u32 {
        unsafe { libc::getuid() }
    }
    fn record(pid: u32, started: u64, state: &str) -> Vec<u8> {
        format!(
            "{pid} (odd ) command\n) {state} {} {started} 0 0\n",
            vec!["0"; 18].join(" ")
        )
        .into_bytes()
    }
    fn fixture(root: &Path, pid: u32, started: u64, executable: &str) -> PathBuf {
        let path = root.join(pid.to_string());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("stat"), record(pid, started, "S")).unwrap();
        let _ = fs::remove_file(path.join("exe"));
        symlink(executable, path.join("exe")).unwrap();
        path
    }
    fn scanned(root: &Path, previous: &Snapshot) -> Snapshot {
        scan(root, uid(), previous, &AtomicBool::new(false)).unwrap()
    }
    #[test]
    fn stat_parses_hostile_comm_and_rejects_incomplete_or_wrong_identity() {
        let mut raw = record(51, 987654321, "S");
        raw[5] = 0xff;
        assert_eq!(
            parse_stat(&raw, 51).unwrap(),
            Stat {
                started: 987654321,
                dead: false
            }
        );
        assert!(parse_stat(&raw, 52).is_err());
        assert!(parse_stat(b"51 (broken) S 0 0", 51).is_err());
        assert!(parse_stat(&record(51, 1, "Z"), 51).unwrap().dead);
        assert!(parse_stat(&record(51, 1, "X"), 51).unwrap().dead);
    }
    #[test]
    fn inventory_tracks_pid_reuse_exec_and_gone_without_cmdline_or_environment() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path(), 42, 10, "/apps/first app");
        // Neither file may be opened: an accidental read would block.
        for name in ["cmdline", "environ"] {
            let c = std::ffi::CString::new(path.join(name).as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        }
        let first = scanned(root.path(), &Snapshot::default());
        assert_eq!(first.processes.len(), 1);
        assert_eq!(
            first.processes.first().unwrap().executable,
            Path::new("/apps/first app")
        );
        fixture(root.path(), 42, 10, "/apps/replaced");
        let exec = scanned(root.path(), &first);
        assert_ne!(exec.processes, first.processes);
        fixture(root.path(), 42, 20, "/apps/replaced");
        let reused = scanned(root.path(), &exec);
        assert_ne!(reused.processes, exec.processes);
        fs::remove_dir_all(path).unwrap();
        assert!(scanned(root.path(), &reused).processes.is_empty());
    }
    #[test]
    fn unreadable_retains_only_same_identity_but_zombie_drops() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path(), 42, 10, "/app");
        let first = scanned(root.path(), &Snapshot::default());
        fs::remove_file(path.join("exe")).unwrap();
        let uncertain = scanned(root.path(), &first);
        assert_eq!(uncertain.processes, first.processes);
        assert!(uncertain.error.is_some());
        fs::write(path.join("stat"), record(42, 20, "S")).unwrap();
        assert!(scanned(root.path(), &first).processes.is_empty());
        fs::write(path.join("stat"), record(42, 10, "Z")).unwrap();
        let dead = scanned(root.path(), &first);
        assert!(dead.processes.is_empty());
        assert!(dead.error.is_none());
        fs::write(path.join("stat"), b"temporary bad stat").unwrap();
        assert_eq!(scanned(root.path(), &first).processes, first.processes);
    }
    #[test]
    fn owner_filter_and_symlinked_pid_never_read_foreign_stat() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), 42, 10, "/app");
        let filtered = scan(
            root.path(),
            uid().wrapping_add(1),
            &Snapshot::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(filtered.processes.is_empty());
        assert!(filtered.error.is_none());
        let outside = tempfile::tempdir().unwrap();
        fixture(outside.path(), 99, 1, "/outside");
        symlink(outside.path().join("99"), root.path().join("99")).unwrap();
        let result = scanned(root.path(), &Snapshot::default());
        assert_eq!(result.processes.len(), 1);
        assert!(result.processes.iter().all(|p| p.pid == 42));
    }
    #[test]
    fn races_do_not_mix_pid_or_executable_generations() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path(), 42, 10, "/first");
        let obs = observe_with(&path, 42, uid(), || {
            fs::write(path.join("stat"), record(42, 20, "S")).unwrap();
        });
        assert!(matches!(obs, Observation::Unreadable(Some(20))));
        fixture(root.path(), 42, 10, "/first");
        let obs = observe_with(&path, 42, uid(), || {
            fs::remove_file(path.join("exe")).unwrap();
            symlink("/second", path.join("exe")).unwrap();
        });
        assert!(matches!(obs, Observation::Unreadable(Some(10))));
        let obs = observe_with(&path, 42, uid(), || {
            fs::remove_dir_all(&path).unwrap();
            fixture(root.path(), 42, 30, "/third");
        });
        // Directory FD remains on the removed identity, never the replacement.
        assert!(matches!(obs, Observation::Unreadable(Some(10))));
    }
    #[test]
    fn malformed_bounded_stat_and_fifo_are_rejected_without_blocking() {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path(), 42, 10, "/app");
        fs::write(path.join("stat"), vec![b'0'; MAX_STAT + 1]).unwrap();
        assert!(matches!(
            observe(&path, 42, uid()),
            Observation::Unreadable(None)
        ));
        fs::remove_file(path.join("stat")).unwrap();
        let c = std::ffi::CString::new(path.join("stat").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let start = Instant::now();
        assert!(matches!(
            observe(&path, 42, uid()),
            Observation::Unreadable(None)
        ));
        assert!(start.elapsed() < Duration::from_millis(200));
    }
    fn wait_snapshot(worker: &Worker, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(s) = worker.snapshot().filter(|s| predicate(s)) {
                return s;
            }
            assert!(Instant::now() < deadline, "snapshot timeout");
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn failed_scan_preserves_snapshot_and_shutdown_interrupts_poll() {
        let root = tempfile::tempdir().unwrap();
        let proc = root.path().join("proc");
        fixture(&proc, 42, 10, "/app");
        let worker = Worker::start(proc.clone(), uid());
        let first = wait_snapshot(&worker, |s| !s.processes.is_empty());
        fs::rename(&proc, root.path().join("offline")).unwrap();
        let failed = wait_snapshot(&worker, |s| s.error.is_some());
        assert_eq!(failed.processes, first.processes);
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(
            scan(
                root.path(),
                uid(),
                &Snapshot::default(),
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
    }
    #[test]
    fn observes_only_owned_short_child_then_removes_reaped_identity() {
        // No signals, desktop app, command-line or environment reads. Only the
        // PID we just spawned is inspected, then it exits naturally and is reaped.
        let mut child = Command::new("/usr/bin/sleep").arg("0.2").spawn().unwrap();
        let pid = child.id();
        let path = PathBuf::from(format!("/proc/{pid}"));
        let observation = observe(&path, pid, uid());
        child.wait().unwrap();
        let Observation::Present(process) = observation else {
            panic!("owned child was not readable");
        };
        assert_eq!(process.pid, pid);
        assert_eq!(
            process.executable,
            fs::canonicalize("/usr/bin/sleep").unwrap()
        );
        assert!(matches!(observe(&path, pid, uid()), Observation::Gone));
    }

    #[test]
    fn unlinked_running_executable_keeps_identity_and_literal_deleted_name() {
        let root = tempfile::tempdir().unwrap();
        // The second filename already has a literal suffix. Once unlinked,
        // Linux adds another; exactly the added suffix must be removed.
        for filename in ["owned-sleep", "owned-sleep (deleted)"] {
            let executable = root.path().join(filename);
            fs::copy("/usr/bin/sleep", &executable).unwrap();
            // A parallel test can fork while fs::copy's writable descriptor is
            // open. Its inherited CLOEXEC descriptor briefly causes ETXTBSY;
            // wait only for that kernel race, without relaxing identity checks.
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            let mut child = loop {
                match Command::new(&executable).arg("0.2").spawn() {
                    Ok(child) => break child,
                    Err(error)
                        if error.raw_os_error() == Some(libc::ETXTBSY)
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Could not start owned executable fixture: {error}"),
                }
            };
            let pid = child.id();
            let path = PathBuf::from(format!("/proc/{pid}"));
            let before = observe(&path, pid, uid());
            let linked_count = fs::metadata(path.join("exe")).map(|m| m.nlink());
            let removed = fs::remove_file(&executable);
            let raw_deleted = fs::read_link(path.join("exe"));
            let unlinked_count = fs::metadata(path.join("exe")).map(|m| m.nlink());
            let after = observe(&path, pid, uid());
            // Reap our child before any assertions, including failure cases.
            child.wait().unwrap();
            removed.unwrap();
            assert_eq!(linked_count.unwrap(), 1);
            assert_eq!(unlinked_count.unwrap(), 0);
            assert_eq!(
                raw_deleted.unwrap().as_os_str().as_bytes(),
                [executable.as_os_str().as_bytes(), b" (deleted)"].concat()
            );
            let (Observation::Present(before), Observation::Present(after)) = (before, after)
            else {
                panic!("owned running executable was not readable");
            };
            assert_eq!(before.executable, executable);
            assert_eq!(before, after);
        }
    }
}

/// Read arguments only after confirming the sampled process still owns this directory.
pub fn matches_arguments(root: &Path, process: &Process, args: &[String]) -> Option<bool> {
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join(process.pid.to_string()))
        .ok()?;
    let matches = || -> Option<bool> {
        if dir.metadata().ok()?.uid() != unsafe { libc::getuid() } {
            return Some(false);
        }
        let stat = stat_at(&dir, process.pid).ok()?;
        Some(
            !stat.dead
                && stat.started == process.started
                && link_at(&dir, c"exe").ok()? == process.executable,
        )
    };
    if !matches()? {
        return None;
    }
    if args.is_empty() {
        return Some(true);
    }
    // SAFETY: live directory FD and static name; ownership is transferred once.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c"cmdline".as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return None;
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 65536 || !matches()? {
        return None;
    }
    let actual: Vec<_> = bytes.split(|b| *b == 0).skip(1).take(args.len()).collect();
    Some(actual.len() == args.len() && actual.iter().zip(args).all(|(a, b)| *a == b.as_bytes()))
}

#[cfg(test)]
mod argument_tests {
    use super::*;
    #[test]
    fn sampled_identity_and_literal_argument_prefix_must_both_match() {
        let child = std::process::Command::new("/usr/bin/sleep")
            .arg("15")
            .spawn()
            .unwrap();
        struct Owned(std::process::Child);
        impl Drop for Owned {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let child = Owned(child);
        let Observation::Present(process) = observe(
            &PathBuf::from(format!("/proc/{}", child.0.id())),
            child.0.id(),
            unsafe { libc::getuid() },
        ) else {
            panic!("Owned process not observed")
        };
        assert_eq!(
            matches_arguments(Path::new("/proc"), &process, &["15".into()]),
            Some(true)
        );
        assert_eq!(
            matches_arguments(Path::new("/proc"), &process, &["1".into()]),
            Some(false)
        );
        let changed = Process {
            started: process.started + 1,
            ..process
        };
        assert_eq!(matches_arguments(Path::new("/proc"), &changed, &[]), None);
    }
}
