//! Install a plugin archive as inert files, without executing plugin code.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::CString,
    fs,
    io::{Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Path, PathBuf},
};

/// Marketplace-protected manifests are binary envelopes, not malformed JSON.
/// A protected manifest requires an explicit local original-application reference.
pub(crate) fn read_manifest(bytes: &[u8]) -> Result<Value> {
    ensure!(
        !bytes.starts_with(b"ELGATO"),
        "This plugin has an Elgato-protected manifest. Supply the original Stream Deck application with --reference-app to authenticate and verify it."
    );
    serde_json::from_slice(bytes).context("Invalid plugin manifest JSON")
}

// Authentication alone is not file-integrity verification. Only the archive and
// installed-folder readers below return a manifest after that additional gate.
fn decode_manifest(bytes: &[u8], reference: Option<&Path>) -> Result<Value> {
    if bytes.starts_with(b"ELGATO")
        && let Some(reference) = reference
    {
        return crate::protected_manifest::read(bytes, reference);
    }
    read_manifest(bytes)
}

/// Revalidate protected required files on every load/restart. Directory-relative
/// opens refuse symlinks and special files without blocking on FIFO endpoints.
pub(crate) fn read_installed_manifest(root: &Path, reference: Option<&Path>) -> Result<Value> {
    let directory = fs::File::options()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .context("Open plugin directory")?;
    let mut manifest_file =
        open_plugin_file(&directory, "manifest.json").context("Read plugin manifest.json")?;
    let mut bytes = vec![];
    (&mut manifest_file)
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "Plugin manifest exceeds 32 MiB"
    );
    let manifest = decode_manifest(&bytes, reference)?;
    if bytes.starts_with(b"ELGATO") {
        verify_installed_files(&directory, &manifest)?;
    }
    Ok(manifest)
}
fn open_plugin_file(root: &fs::File, path: &str) -> Result<fs::File> {
    let mut directory = root.try_clone()?;
    let parts = path.split('/').collect::<Vec<_>>();
    ensure!(
        !parts.is_empty()
            && parts.iter().all(|s| !s.is_empty()
                && *s != "."
                && *s != ".."
                && !s.contains(['\\', ':', '\0'])),
        "Unsafe plugin file path"
    );
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let name = CString::new(*part)?;
        // SAFETY: The parent descriptor is owned/open, the relative component is
        // validated, and the returned descriptor is immediately owned by File.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC
                    | if last { 0 } else { libc::O_DIRECTORY },
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("Required plugin file unavailable or unsafe: {path}"));
        }
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    let metadata = directory.metadata()?;
    ensure!(
        metadata.is_file(),
        "Plugin file is not a regular file: {path}"
    );
    ensure!(
        metadata.len() <= MAX_FILE,
        "Plugin file exceeds 32 MiB: {path}"
    );
    Ok(directory)
}
fn verify_installed_files(root: &fs::File, manifest: &Value) -> Result<()> {
    let mut total = 0usize;
    for entry in crate::protected_manifest::required_files(manifest)? {
        let mut file = open_plugin_file(root, &entry.path)?;
        let mut hash = blake3::Hasher::new();
        let mut read = 0u64;
        let mut buffer = [0; 65536];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read += count as u64;
            total = total.checked_add(count).context("Plugin size overflow")?;
            ensure!(
                read <= MAX_FILE && total <= MAX_TOTAL,
                "Required plugin files exceed size limit"
            );
            hash.update(&buffer[..count]);
        }
        ensure!(
            hash.finalize().as_bytes() == &entry.digest,
            "Required plugin file integrity mismatch: {}",
            entry.path
        );
    }
    Ok(())
}

const MAX_ENTRIES: usize = 10_000;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: usize = 128 * 1024 * 1024;

struct Entry {
    bytes: Option<Vec<u8>>,
    executable: bool,
}

/// Validate, stage and atomically install a .streamDeckPlugin ZIP. Existing plugins
/// are never replaced. The caller decides whether and when to launch the result.
pub fn install(archive: &Path, destination: &Path) -> Result<PathBuf> {
    install_with_reference(archive, destination, None)
}

pub fn install_with_reference(
    archive: &Path,
    destination: &Path,
    reference: Option<&Path>,
) -> Result<PathBuf> {
    let (folder, entries) = validated_archive(archive, reference)?;
    reject_symlinks(destination)?;
    fs::create_dir_all(destination).context("Create plugin collection directory")?;
    let destination = destination.canonicalize()?;
    let target = destination.join(&folder);
    ensure!(
        !target.try_exists()? && fs::symlink_metadata(&target).is_err(),
        "Plugin already exists: {}",
        target.display()
    );

    // TempDir creates a fresh owner-only directory in the same filesystem.
    let stage = tempfile::Builder::new()
        .prefix(".deckforge-install-")
        .tempdir_in(&destination)?;
    let plugin = stage.path().join(&folder);
    fs::create_dir(&plugin)?;
    for (name, entry) in entries {
        let path = stage.path().join(name);
        if let Some(bytes) = entry.bytes {
            fs::create_dir_all(path.parent().context("Package file has no parent")?)?;
            // create_new adds a second collision guard within our private staging area.
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if entry.executable { 0o700 } else { 0o600 }),
            )?;
        } else {
            fs::create_dir_all(path)?;
        }
    }
    rename_without_replacing(&plugin, &target)?;
    Ok(target)
}

fn reject_symlinks(destination: &Path) -> Result<()> {
    for path in destination.ancestors() {
        match fs::symlink_metadata(path) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "Plugin destination contains a symlink: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn validated_archive(
    path: &Path,
    reference: Option<&Path>,
) -> Result<(String, BTreeMap<String, Entry>)> {
    let mut zip = zip::ZipArchive::new(
        fs::File::open(path).with_context(|| format!("Open {}", path.display()))?,
    )?;
    validate_central_directory(path, zip.central_directory_start())?;
    ensure!(
        zip.len() <= MAX_ENTRIES,
        "Plugin exceeds 10000 archive entries"
    );
    let mut entries = BTreeMap::new();
    let mut roots = HashSet::new();
    let mut total = 0usize;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let raw_name = file.name();
        let name = raw_name.strip_suffix('/').unwrap_or(raw_name).to_owned();
        ensure!(
            !name.is_empty()
                && !name.starts_with('/')
                && !name.contains(['\\', ':', '\0'])
                && name
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != ".."),
            "Unsafe plugin archive path: {raw_name:?}"
        );
        let root = name.split('/').next().unwrap().to_owned();
        ensure!(
            root.ends_with(".sdPlugin") && root != ".sdPlugin",
            "Expected top-level .sdPlugin folder"
        );
        ensure!(
            name.contains('/') || file.is_dir(),
            "Plugin root must be a directory"
        );
        roots.insert(root);
        let mode = file.unix_mode().unwrap_or(0);
        ensure!(
            [0, 0o100000, 0o040000].contains(&(mode & 0o170000)),
            "Plugin contains a symlink or special file: {name}"
        );
        let bytes = if file.is_dir() {
            None
        } else {
            ensure!(
                file.size() <= MAX_FILE,
                "Plugin file exceeds 32 MiB: {name}"
            );
            let mut bytes = Vec::new();
            (&mut file).take(MAX_FILE + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_FILE,
                "Plugin file exceeds 32 MiB: {name}"
            );
            total = total
                .checked_add(bytes.len())
                .context("Plugin size overflow")?;
            ensure!(total <= MAX_TOTAL, "Plugin exceeds 128 MiB expanded size");
            Some(bytes)
        };
        ensure!(
            entries
                .insert(
                    name.clone(),
                    Entry {
                        bytes,
                        executable: mode & 0o111 != 0
                    }
                )
                .is_none(),
            "Duplicate plugin archive path: {name}"
        );
    }
    ensure!(
        roots.len() == 1,
        "Plugin archive must contain exactly one .sdPlugin folder"
    );
    let folder = roots.into_iter().next().unwrap();
    let manifest = entries
        .get(&format!("{folder}/manifest.json"))
        .and_then(|e| e.bytes.as_ref())
        .context("Plugin manifest.json missing")?;
    let protected = manifest.starts_with(b"ELGATO");
    let manifest = decode_manifest(manifest, reference)?;
    let uuid = manifest["UUID"]
        .as_str()
        .context("Plugin manifest UUID missing")?;
    ensure!(
        folder == format!("{uuid}.sdPlugin"),
        "Plugin manifest UUID does not match folder name"
    );
    if protected {
        for file in crate::protected_manifest::required_files(&manifest)? {
            let bytes = entries
                .get(&format!("{folder}/{}", file.path))
                .and_then(|e| e.bytes.as_deref())
                .with_context(|| format!("Required plugin file missing: {}", file.path))?;
            file.verify(bytes)?;
        }
    }
    for name in entries.keys() {
        let mut parent = Path::new(name).parent();
        while let Some(path) = parent {
            if let Some(entry) = entries.get(path.to_str().unwrap()) {
                ensure!(
                    entry.bytes.is_none(),
                    "Archive uses a file as a directory: {}",
                    path.display()
                );
            }
            parent = path.parent();
        }
    }
    Ok((folder, entries))
}

// zip 2.x stores entries in a map and silently hides repeated names. Inspect only
// central directory metadata before using its decompression/path APIs.
pub(crate) fn validate_central_directory(path: &Path, offset: u64) -> Result<()> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut names = HashSet::new();
    loop {
        let mut signature = [0u8; 4];
        file.read_exact(&mut signature)?;
        if matches!(&signature, b"PK\x05\x06" | b"PK\x06\x06") {
            break;
        }
        ensure!(
            &signature == b"PK\x01\x02",
            "Unexpected ZIP central directory record"
        );
        let mut header = [0u8; 42];
        file.read_exact(&mut header)?;
        let name_len = u16::from_le_bytes([header[24], header[25]]) as usize;
        let extra_len = u16::from_le_bytes([header[26], header[27]]) as i64;
        let comment_len = u16::from_le_bytes([header[28], header[29]]) as i64;
        let mut name = vec![0; name_len];
        file.read_exact(&mut name)?;
        ensure!(
            names.insert(name),
            "Duplicate ZIP central directory filename"
        );
        ensure!(names.len() <= MAX_ENTRIES, "Archive exceeds 10000 entries");
        file.seek(SeekFrom::Current(extra_len + comment_len))?;
    }
    Ok(())
}

fn rename_without_replacing(source: &Path, target: &Path) -> Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    // Linux renameat2 provides an atomic no-replace operation for directories.
    // Both C strings are owned, NUL-terminated and live throughout this call.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        bail!(
            "Publish plugin without overwrite: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn protected_manifest_is_distinguished_from_broken_json() {
        let error = super::read_manifest(b"ELGATO\x01\x00test envelope").unwrap_err();
        assert!(error.to_string().contains("Elgato-protected manifest"));
        assert!(!error.to_string().contains("Invalid plugin manifest JSON"));
        assert!(
            super::read_manifest(b"{broken")
                .unwrap_err()
                .to_string()
                .contains("Invalid plugin manifest JSON")
        );
        assert_eq!(
            super::read_manifest(br#"{"UUID":"test"}"#).unwrap()["UUID"],
            "test"
        );
    }

    use super::*;
    fn integrity_manifest(bytes: &[u8]) -> Value {
        use base64::Engine as _;
        serde_json::json!({"Files":[{"Path":"sub/file","Hash":base64::engine::general_purpose::STANDARD.encode(blake3::hash(bytes).as_bytes()),"Required":true},{"Path":"optional","Required":false}]})
    }
    #[test]
    fn installed_required_files_recheck_contents_and_optional_absence() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub/file"), b"good").unwrap();
        let root = fs::File::open(tmp.path()).unwrap();
        let manifest = integrity_manifest(b"good");
        verify_installed_files(&root, &manifest).unwrap();
        fs::write(tmp.path().join("sub/file"), b"tampered").unwrap();
        assert!(
            verify_installed_files(&root, &manifest)
                .unwrap_err()
                .to_string()
                .contains("integrity mismatch")
        );
        fs::remove_file(tmp.path().join("sub/file")).unwrap();
        assert!(verify_installed_files(&root, &manifest).is_err());
    }
    #[test]
    fn installed_integrity_refuses_symlinks_fifo_and_oversize() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("file"), b"good").unwrap();
        let root = fs::File::open(tmp.path()).unwrap();
        let manifest = integrity_manifest(b"good");
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("sub")).unwrap();
        assert!(verify_installed_files(&root, &manifest).is_err());
        fs::remove_file(tmp.path().join("sub")).unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        let path = tmp.path().join("sub/file");
        std::os::unix::fs::symlink(outside.path().join("file"), &path).unwrap();
        assert!(verify_installed_files(&root, &manifest).is_err());
        fs::remove_file(&path).unwrap();
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(verify_installed_files(&root, &manifest).is_err());
        fs::remove_file(&path).unwrap();
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert!(verify_installed_files(&root, &manifest).is_err());
    }
    use std::io::Write;

    fn make_archive(path: &Path, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    #[ignore = "requires the official Layouts plugin archive in ../evidence/official-samples"]
    fn installs_official_layouts_and_refuses_collision() {
        let temporary = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../evidence/official-samples/layouts/com.elgato.layouts.streamDeckPlugin");
        let destination = temporary.path().join("plugins");
        let plugin = install(&fixture, &destination).unwrap();
        assert_eq!(plugin.file_name().unwrap(), "com.elgato.layouts.sdPlugin");
        let original = fs::read(plugin.join("bin/plugin.js")).unwrap();
        assert!(!original.is_empty());
        assert!(
            install(&fixture, &destination)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert_eq!(fs::read(plugin.join("bin/plugin.js")).unwrap(), original);
        assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
    }

    #[test]
    fn protected_manifest_import_leaves_no_partial_installation() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("protected.streamDeckPlugin");
        let destination = temporary.path().join("plugins");
        make_archive(
            &archive,
            &[("com.test.sdPlugin/manifest.json", b"ELGATO\x01\x00envelope")],
        );
        assert!(
            install(&archive, &destination)
                .unwrap_err()
                .to_string()
                .contains("Elgato-protected")
        );
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_bad_paths_and_uuid_before_writing() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("test.zip");
        let destination = temporary.path().join("plugins");
        for path in [
            "../escape",
            "/absolute",
            "C:/escape",
            "com.test.sdPlugin/../escape",
            "com.test.sdPlugin\\bad",
        ] {
            make_archive(&archive, &[(path, b"payload")]);
            assert!(install(&archive, &destination).is_err(), "{path}");
            assert!(!destination.exists());
        }
        make_archive(
            &archive,
            &[(
                "com.test.sdPlugin/manifest.json",
                br#"{"UUID":"com.other"}"#,
            )],
        );
        assert!(
            install(&archive, &destination)
                .unwrap_err()
                .to_string()
                .contains("UUID")
        );
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_multiple_roots_and_file_directory_conflicts() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("test.zip");
        let destination = temporary.path().join("plugins");
        make_archive(
            &archive,
            &[
                ("com.test.sdPlugin/manifest.json", br#"{"UUID":"com.test"}"#),
                ("com.other.sdPlugin/x", b"x"),
            ],
        );
        assert!(install(&archive, &destination).is_err());
        make_archive(
            &archive,
            &[
                ("com.test.sdPlugin/manifest.json", br#"{"UUID":"com.test"}"#),
                ("com.test.sdPlugin/a", b"x"),
                ("com.test.sdPlugin/a/b", b"x"),
            ],
        );
        assert!(
            install(&archive, &destination)
                .unwrap_err()
                .to_string()
                .contains("file as a directory")
        );
        assert!(!destination.exists());
    }

    #[test]
    fn rejects_duplicates_hidden_by_zip_library() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("test.zip");
        let destination = temporary.path().join("plugins");
        make_archive(
            &archive,
            &[
                ("com.test.sdPlugin/manifest.json", br#"{"UUID":"com.test"}"#),
                ("com.test.sdPlugin/a", b"x"),
                ("com.test.sdPlugin/b", b"y"),
            ],
        );
        // Change the second name in both headers without changing its size/CRC.
        let mut bytes = fs::read(&archive).unwrap();
        let name = b"com.test.sdPlugin/b";
        for index in 0..=bytes.len() - name.len() {
            if &bytes[index..index + name.len()] == name {
                bytes[index + name.len() - 1] = b'a';
            }
        }
        fs::write(&archive, bytes).unwrap();
        assert!(
            install(&archive, &destination)
                .unwrap_err()
                .to_string()
                .contains("Duplicate ZIP")
        );
        assert!(!destination.exists());
    }

    #[test]
    fn refuses_symlinks_and_atomic_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("test.zip");
        let destination = temporary.path().join("plugins");
        make_archive(
            &archive,
            &[("com.test.sdPlugin/manifest.json", br#"{"UUID":"com.test"}"#)],
        );
        std::os::unix::fs::symlink(temporary.path(), &destination).unwrap();
        assert!(
            install(&archive, &destination)
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        let source = temporary.path().join("source");
        let target = temporary.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        assert!(rename_without_replacing(&source, &target).is_err());
        assert!(source.is_dir() && target.is_dir());
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.add_symlink(
            "com.test.sdPlugin/link",
            "/etc/passwd",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        assert!(
            validated_archive(&archive, None)
                .err()
                .unwrap()
                .to_string()
                .contains("symlink")
        );
    }
}
