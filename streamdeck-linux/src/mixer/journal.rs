//! Private ownership records, never application preferences. A kernel lease
//! prevents two workers from adopting the same namespace simultaneously.
use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
const MAX_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub(super) struct Saved {
    pub version: u32,
    pub namespace: String,
    pub session: String,
    pub owned: Vec<(String, Module)>,
    pub pending: BTreeMap<String, (String, String)>,
    pub moved: BTreeMap<u32, Moved>,
    pub paused: BTreeSet<String>,
}
pub(super) struct Journal {
    _directory: File,
    directory: PathBuf,
    path: PathBuf,
    _lease: File,
    last: Option<Vec<u8>>,
}
impl Journal {
    pub fn open(namespace: &str) -> Result<(Self, Option<Saved>)> {
        let base = if let Some(path) = std::env::var_os("XDG_STATE_HOME").filter(|p| !p.is_empty())
        {
            PathBuf::from(path)
        } else {
            PathBuf::from(
                std::env::var_os("HOME")
                    .context("HOME is unavailable for mixer ownership records")?,
            )
            .join(".local/state")
        };
        ensure!(base.is_absolute(), "Mixer state directory must be absolute");
        Self::open_at(&base, namespace)
    }
    fn open_at(base: &Path, namespace: &str) -> Result<(Self, Option<Saved>)> {
        valid_id(namespace)?;
        fs::create_dir_all(base).context("Create mixer state base")?;
        let directory = base.join("deckforge-mixer");
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let directory_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&directory)?;
        let metadata = directory_file.metadata()?;
        ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Mixer ownership directory must be private and owned by this user"
        );
        // Anchor subsequent operations to the verified directory inode even if
        // its parent path is renamed concurrently.
        let directory = PathBuf::from(format!("/proc/self/fd/{}", directory_file.as_raw_fd()));
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(directory.join(format!("{namespace}.lock")))
            .context("Open mixer ownership lease")?;
        check_file(&lease)?;
        lease
            .try_lock()
            .context("This mixer namespace is already owned by another running worker")?;
        let path = directory.join(format!("{namespace}.json"));
        let bytes = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
        {
            Ok(file) => {
                check_file(&file)?;
                ensure!(
                    file.metadata()?.len() <= MAX_BYTES,
                    "Mixer ownership record exceeds 16 MiB"
                );
                let mut bytes = vec![];
                file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= MAX_BYTES,
                    "Mixer ownership record exceeds 16 MiB"
                );
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("Read mixer ownership record"),
        };
        let saved = bytes
            .as_ref()
            .map(|b| serde_json::from_slice::<Saved>(b))
            .transpose()
            .context("Invalid mixer ownership record")?;
        if let Some(saved) = &saved {
            saved.validate(namespace)?;
        }
        Ok((
            Self {
                _directory: directory_file,
                directory,
                path,
                _lease: lease,
                last: bytes,
            },
            saved,
        ))
    }
    pub fn write(&mut self, saved: &Saved) -> Result<()> {
        saved.validate(&saved.namespace)?;
        if saved.owned.is_empty() && saved.pending.is_empty() && saved.moved.is_empty() {
            if self.last.take().is_some() {
                match fs::remove_file(&self.path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            return Ok(());
        }
        let bytes = serde_json::to_vec(saved)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "Mixer ownership journal capacity exceeded (16 MiB)"
        );
        if self.last.as_ref() == Some(&bytes) {
            return Ok(());
        }
        let temp = self
            .directory
            .join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temp)?;
            file.write_all(&bytes)?;
            drop(file);
            fs::rename(&temp, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.context("Persist mixer ownership before changing audio")?;
        self.last = Some(bytes);
        Ok(())
    }
}
fn check_file(file: &File) -> Result<()> {
    let m = file.metadata()?;
    ensure!(
        m.is_file() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "Mixer ownership file must be a private regular file owned by this user"
    );
    Ok(())
}
impl Saved {
    fn validate(&self, namespace: &str) -> Result<()> {
        ensure!(
            self.version == 1 && self.namespace == namespace,
            "Mixer ownership record belongs to another namespace or version"
        );
        ensure!(
            self.session.len() == 32 && self.session.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid mixer ownership session"
        );
        let prefix = format!("deckforge.{namespace}.");
        for (key, name, args) in self
            .owned
            .iter()
            .map(|(key, m)| (key, &m.name, &m.args))
            .chain(
                self.pending
                    .iter()
                    .map(|(key, (name, args))| (key, name, args)),
            )
        {
            ensure!(
                key.len() <= 4096
                    && matches!(
                        name.as_str(),
                        "module-null-sink" | "module-remap-source" | "module-loopback"
                    )
                    && args.len() <= 65536
                    && args.contains(&self.session)
                    && args.contains(&prefix)
                    && args.contains("deckforge.owner="),
                "Unrecognized module ownership record"
            );
        }
        for (id, moved) in &self.moved {
            ensure!(
                *id == moved.identity.index && moved.target.starts_with(&prefix),
                "Invalid moved-stream ownership record"
            );
            valid_external(&moved.original)?;
            ensure!(
                !moved.original.starts_with('@'),
                "Original audio output must be an exact identity"
            );
        }
        ensure!(
            self.paused.iter().all(|n| n.starts_with(&prefix)),
            "Invalid paused endpoint ownership record"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "deckforge-journal-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn saved() -> Saved {
        Saved {
            version: 1,
            namespace: "fixture".into(),
            session: "a".repeat(32),
            owned: vec![(
                "mix:one".into(),
                Module {
                    id: 42,
                    name: "module-null-sink".into(),
                    args: format!(
                        "sink_name=deckforge.fixture.mix.one deckforge.owner=fixture deckforge.session={}",
                        "a".repeat(32)
                    ),
                },
            )],
            pending: BTreeMap::new(),
            moved: BTreeMap::new(),
            paused: BTreeSet::new(),
        }
    }
    #[test]
    fn private_roundtrip_and_exclusive_lease() {
        let d = Directory::new();
        let (mut j, old) = Journal::open_at(&d.0, "fixture").unwrap();
        assert!(old.is_none());
        j.write(&saved()).unwrap();
        assert!(Journal::open_at(&d.0, "fixture").is_err());
        let metadata = fs::metadata(&j.path).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
        drop(j);
        let (_, value) = Journal::open_at(&d.0, "fixture").unwrap();
        assert_eq!(value.unwrap().owned[0].1.id, 42);
    }
    #[test]
    fn refuses_symlinked_records_and_foreign_identity() {
        let d = Directory::new();
        let (j, _) = Journal::open_at(&d.0, "fixture").unwrap();
        let path = d.0.join("deckforge-mixer/fixture.json");
        drop(j);
        std::os::unix::fs::symlink("/dev/null", &path).unwrap();
        assert!(Journal::open_at(&d.0, "fixture").is_err());
        let mut s = saved();
        s.owned[0].1.args = "sink_name=unrelated".into();
        assert!(s.validate("fixture").is_err());
    }
}
