//! Install a plugin archive as inert files, without executing plugin code.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::CString,
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

/// Marketplace-protected manifests are binary envelopes, not malformed JSON.
/// Keep original bytes intact until protected-package verification is supported.
pub(crate) fn read_manifest(bytes: &[u8]) -> Result<Value> {
    ensure!(
        !bytes.starts_with(b"ELGATO"),
        "This plugin has an Elgato-protected manifest. Protected Marketplace package import is not supported yet."
    );
    serde_json::from_slice(bytes).context("Invalid plugin manifest JSON")
}

pub(crate) fn read_manifest_with_reference(
    bytes: &[u8],
    reference: Option<&Path>,
) -> Result<Value> {
    if bytes.starts_with(b"ELGATO")
        && let Some(reference) = reference
    {
        let _manifest = crate::protected_manifest::read(bytes, reference)?;
        bail!(
            "Protected manifest authenticated, but package file-integrity verification is not implemented yet; installation remains blocked"
        );
    }
    read_manifest(bytes)
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
    let manifest = read_manifest_with_reference(manifest, reference)?;
    let uuid = manifest["UUID"]
        .as_str()
        .context("Plugin manifest UUID missing")?;
    ensure!(
        folder == format!("{uuid}.sdPlugin"),
        "Plugin manifest UUID does not match folder name"
    );
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
