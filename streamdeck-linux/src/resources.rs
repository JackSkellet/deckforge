//! Bounded resource files without SDK registry or action-setting semantics.
//! Cache writes use directory-relative syscalls, refuse symlink components, and
//! atomically replace one file while preserving its original UTF-8 basename.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{CString, OsStr},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

/// Store an already-validated archive member under cache/profile/resource/name.
/// All identifiers are still checked at this filesystem boundary. Existing equal
/// content keeps its inode and modification time. No unrelated cache files change.
pub fn materialize(
    cache: &Path,
    profile_id: &str,
    resource_id: &str,
    archive_name: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    component(profile_id)?;
    component(resource_id)?;
    let basename = archive_basename(archive_name)?;
    ensure!(bytes.len() <= MAX_FILE_BYTES, "Resource exceeds 32 MiB");
    let (root, absolute) = cache_directory(cache)?;
    let profile = child_directory(&root, OsStr::new(profile_id))?;
    let directory = child_directory(&profile, OsStr::new(resource_id))?;
    let filename = CString::new(basename)?;
    let target = absolute.join(profile_id).join(resource_id).join(basename);
    match open_at(
        &directory,
        &filename,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        0,
    ) {
        Ok(existing) => {
            let metadata = existing.metadata()?;
            ensure!(
                metadata.is_file(),
                "Existing cached resource is not a regular file"
            );
            if metadata.len() == bytes.len() as u64 && read_bounded(existing)?.as_slice() == bytes {
                return Ok(target);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .context("Inspect existing cached resource without following symlinks");
        }
    }
    let mut pending = Pending::new(&directory)?;
    pending
        .file
        .write_all(bytes)
        .context("Write cached resource")?;
    pending.file.sync_all().context("Flush cached resource")?;
    // SAFETY: Both names are validated C strings, and both directory descriptors
    // remain owned/open. renameat replaces a final symlink itself, never its target.
    let result = unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            pending.name.as_ptr(),
            directory.as_raw_fd(),
            filename.as_ptr(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error()).context("Atomically replace cached resource");
    }
    pending.published = true;
    directory
        .sync_all()
        .context("Flush cached resource directory")?;
    Ok(target)
}

/// Read a user-selected file with a strict size limit. Explicit symlinks are
/// resolved first, then opened nonblocking/no-follow and checked on the open fd.
/// The returned name is the supplied path's basename, not the symlink target's.
pub fn read_regular(path: &Path) -> Result<(String, Vec<u8>)> {
    let basename = path
        .file_name()
        .and_then(OsStr::to_str)
        .context("Resource filename must be UTF-8")?;
    component(basename)?;
    let resolved = path.canonicalize().context("Resolve resource file")?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(&resolved)
        .context("Open resource file without blocking on special files")?;
    Ok((basename.to_owned(), read_bounded(file)?))
}

fn component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value != "."
            && value != ".."
            && value.len() <= 255
            && !value.contains(['/', '\\', ':', '\0'])
            && !value.chars().any(char::is_control),
        "Unsafe resource path component {value:?}"
    );
    Ok(())
}
fn archive_basename(name: &str) -> Result<&str> {
    ensure!(
        !name.is_empty() && name.len() <= 4096,
        "Invalid archive resource name"
    );
    for part in name.split('/') {
        component(part)?;
    }
    name.rsplit('/')
        .next()
        .context("Archive resource has no basename")
}

/// Walk the caller's cache path using directory capabilities. Parent traversal
/// and symlinks are refused, including in existing cache ancestors. Relative cache
/// roots are resolved against the process's physical current working directory.
fn cache_directory(cache: &Path) -> Result<(File, PathBuf)> {
    let absolute = if cache.is_absolute() {
        cache.to_owned()
    } else {
        std::env::current_dir()?.join(cache)
    };
    ensure!(
        !cache.as_os_str().is_empty(),
        "Cache directory path is empty"
    );
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open("/")?;
    let mut normalized = PathBuf::from("/");
    // Validate before creating anything: a rejected parent component must not
    // leave a partially created directory tree behind.
    ensure!(
        absolute.components().all(|part| matches!(
            part,
            Component::RootDir | Component::CurDir | Component::Normal(_)
        )),
        "Cache directory must not contain parent traversal"
    );
    for part in absolute.components() {
        if let Component::Normal(name) = part {
            directory = child_directory(&directory, name)?;
            normalized.push(name);
        }
    }
    Ok((directory, normalized))
}
fn child_directory(parent: &File, name: &OsStr) -> Result<File> {
    let name = CString::new(name.as_bytes()).context("Directory component contains NUL")?;
    // SAFETY: The parent is an owned directory fd; name contains one component.
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
    if created < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error).context("Create resource cache directory");
        }
    }
    open_at(
        parent,
        &name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        0,
    )
    .context("Open resource cache directory without following symlinks")
}
fn open_at(
    directory: &File,
    name: &CString,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> io::Result<File> {
    // SAFETY: The fd is live, name is NUL terminated, and a returned descriptor
    // is immediately transferred into File, which closes it exactly once.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn read_bounded(mut file: File) -> Result<Vec<u8>> {
    let metadata = file.metadata().context("Inspect open resource file")?;
    ensure!(metadata.is_file(), "Resource is not a regular file");
    ensure!(
        metadata.len() <= MAX_FILE_BYTES as u64,
        "Resource exceeds 32 MiB"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let capacity = buffer.len().min(MAX_FILE_BYTES - bytes.len() + 1);
        let count = match file.read(&mut buffer[..capacity]) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("Read resource file"),
        };
        if count == 0 {
            break;
        }
        ensure!(
            count <= MAX_FILE_BYTES - bytes.len(),
            "Resource grew beyond 32 MiB while reading"
        );
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

struct Pending<'a> {
    directory: &'a File,
    name: CString,
    file: File,
    published: bool,
}
impl<'a> Pending<'a> {
    fn new(directory: &'a File) -> Result<Self> {
        for _ in 0..8 {
            let name = CString::new(format!(".deckforge-resource-{}.tmp", uuid::Uuid::new_v4()))?;
            match open_at(
                directory,
                &name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
                0o600,
            ) {
                Ok(file) => {
                    return Ok(Self {
                        directory,
                        name,
                        file,
                        published: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("Create temporary cached resource"),
            }
        }
        bail!("Could not reserve a unique cached-resource temporary file")
    }
}
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if !self.published {
            // SAFETY: Remove only this helper's uniquely created temporary name
            // within its still-open directory, regardless of path renames outside.
            unsafe {
                libc::unlinkat(self.directory.as_raw_fd(), self.name.as_ptr(), 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        os::unix::{
            ffi::OsStringExt,
            fs::{MetadataExt, symlink},
        },
        sync::mpsc,
        thread,
        time::Duration,
    };
    #[test]
    fn materialization_preserves_basename_path_and_unchanged_inode() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let path = materialize(
            &cache,
            "profile",
            "resource",
            "Resources/source/My clip æ #1.mp3",
            b"original",
        )
        .unwrap();
        assert!(path.is_absolute());
        assert_eq!(path, cache.join("profile/resource/My clip æ #1.mp3"));
        let before = path.metadata().unwrap();
        let again = materialize(
            &cache,
            "profile",
            "resource",
            "source/My clip æ #1.mp3",
            b"original",
        )
        .unwrap();
        let after = again.metadata().unwrap();
        assert_eq!(again, path);
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(
            read_regular(&path).unwrap(),
            ("My clip æ #1.mp3".into(), b"original".to_vec())
        );
    }
    #[test]
    fn different_content_is_atomically_replaced_without_changing_an_open_reader() {
        let temp = tempfile::tempdir().unwrap();
        let path = materialize(temp.path(), "p", "r", "clip.wav", b"before").unwrap();
        let mut retained = File::open(&path).unwrap();
        let inode = retained.metadata().unwrap().ino();
        assert_eq!(
            materialize(temp.path(), "p", "r", "clip.wav", b"after").unwrap(),
            path
        );
        assert_ne!(path.metadata().unwrap().ino(), inode);
        let mut old = Vec::new();
        retained.read_to_end(&mut old).unwrap();
        assert_eq!(old, b"before");
        assert_eq!(std::fs::read(&path).unwrap(), b"after");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }
    #[test]
    fn identical_basenames_are_isolated_by_profile_and_resource_identity() {
        let temp = tempfile::tempdir().unwrap();
        let a = materialize(temp.path(), "p1", "r1", "one/file.bin", b"one").unwrap();
        let b = materialize(temp.path(), "p1", "r2", "two/file.bin", b"two").unwrap();
        let c = materialize(temp.path(), "p2", "r1", "file.bin", b"three").unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(std::fs::read(a).unwrap(), b"one");
        assert_eq!(std::fs::read(b).unwrap(), b"two");
        assert_eq!(std::fs::read(c).unwrap(), b"three");
    }
    #[test]
    fn unsafe_components_and_oversized_input_fail_before_creating_cache() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("absent");
        for value in [
            "",
            ".",
            "..",
            "../escape",
            "a/b",
            "a\\b",
            "C:",
            "bad\0id",
            "bad\nname",
        ] {
            assert!(materialize(&cache, value, "r", "file", b"x").is_err());
            assert!(materialize(&cache, "p", value, "file", b"x").is_err());
        }
        for name in [
            "/file",
            "../file",
            "a/../file",
            "a//file",
            "a/./file",
            "C:/file",
            "a\\file",
            "a/",
        ] {
            assert!(materialize(&cache, "p", "r", name, b"x").is_err());
        }
        assert!(materialize(&cache, "p", "r", "file", &vec![0; MAX_FILE_BYTES + 1]).is_err());
        assert!(!cache.exists());
    }
    #[test]
    fn cache_symlinks_cannot_redirect_resource_writes() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let cache = temp.path().join("cache");
        symlink(&outside, &cache).unwrap();
        assert!(materialize(&cache, "p", "r", "file", b"x").is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        std::fs::remove_file(&cache).unwrap();
        std::fs::create_dir(&cache).unwrap();
        symlink(&outside, cache.join("p")).unwrap();
        assert!(materialize(&cache, "p", "r", "file", b"x").is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        std::fs::remove_file(cache.join("p")).unwrap();
        std::fs::create_dir(cache.join("p")).unwrap();
        symlink(&outside, cache.join("p/r")).unwrap();
        assert!(materialize(&cache, "p", "r", "file", b"x").is_err());
        std::fs::remove_file(cache.join("p/r")).unwrap();
        std::fs::create_dir(cache.join("p/r")).unwrap();
        let victim = outside.join("victim");
        std::fs::write(&victim, b"untouched").unwrap();
        symlink(&victim, cache.join("p/r/file")).unwrap();
        assert!(materialize(&cache, "p", "r", "file", b"x").is_err());
        assert_eq!(std::fs::read(victim).unwrap(), b"untouched");
    }
    #[test]
    fn explicit_read_symlink_preserves_the_supplied_name_and_file_bound() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("internal.bin");
        std::fs::write(&target, b"payload").unwrap();
        let alias = temp.path().join("Original name.flac");
        symlink(&target, &alias).unwrap();
        assert_eq!(
            read_regular(&alias).unwrap(),
            ("Original name.flac".into(), b"payload".to_vec())
        );
        File::create(&target)
            .unwrap()
            .set_len(MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(read_regular(&alias).is_err());
        assert!(read_regular(temp.path()).is_err());
        let invalid = temp.path().join(OsString::from_vec(vec![0xff]));
        std::fs::write(&invalid, b"x").unwrap();
        assert!(read_regular(&invalid).is_err());
    }
    #[test]
    fn fifo_is_rejected_without_waiting_for_a_writer() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fifo");
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = mpsc::channel();
        let reader_path = path.clone();
        let reader = thread::spawn(move || {
            let _ = sender.send(read_regular(&reader_path));
        });
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => assert!(result.is_err()),
            Err(error) => {
                // Unblock a regressed blocking open before failing the test.
                let _writer = OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&path);
                let _ = reader.join();
                panic!("FIFO read blocked: {error}");
            }
        }
        reader.join().unwrap();
    }
}
