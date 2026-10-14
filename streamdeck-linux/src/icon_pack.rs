//! Inert Stream Deck icon packs, read from their shipped manifest/icons format.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
mod management;
pub use management::{
    Archive, Review, apply, hide, remove, review, review_restore, selected_asset,
};
const EXT: &str = ".sdIconPack";
const MAX_FILES: usize = 10_000;
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Icon {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub icon: String,
    pub icons: Vec<Icon>,
    pub warnings: Vec<String>,
    pub revision: String,
    pub pinned: bool,
    pub can_hide: bool,
    pub hidden: bool,
}
#[derive(Serialize)]
pub struct Catalog {
    pub packs: Vec<Pack>,
    pub errors: Vec<String>,
    pub archives: Vec<Archive>,
}
fn relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.contains(['\\', ':', '\0'])
            && path
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != ".."),
        "Unsafe icon pack path"
    );
    Ok(())
}
fn pack_id(folder: &str) -> Result<String> {
    relative(folder)?;
    ensure!(
        !folder.contains('/') && folder.len() <= 255,
        "Invalid icon pack folder"
    );
    let id = folder
        .strip_suffix(EXT)
        .filter(|s| !s.is_empty() && !s.starts_with('.'))
        .context("Select a .sdIconPack folder")?;
    Ok(id.into())
}
fn folder(root: &Path) -> Result<String> {
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .context("Invalid icon pack folder name")?;
    pack_id(name)?;
    Ok(name.into())
}
fn revision(root: &Path, files: &[String]) -> Result<String> {
    let meta = fs::symlink_metadata(root)?;
    let mut hash = Sha256::new();
    hash.update(meta.dev().to_le_bytes());
    hash.update(meta.ino().to_le_bytes());
    for name in files {
        let meta = fs::symlink_metadata(root.join(name))?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Icon pack file changed"
        );
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        for value in [
            meta.dev(),
            meta.ino(),
            meta.len(),
            meta.mtime() as u64,
            meta.mtime_nsec() as u64,
            meta.ctime() as u64,
            meta.ctime_nsec() as u64,
        ] {
            hash.update(value.to_le_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn digest(root: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    for name in inventory(root)? {
        let bytes = read(root, &name)?;
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn read(root: &Path, path: &str) -> Result<Vec<u8>> {
    relative(path)?;
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    let file = crate::package::open_plugin_file(&directory, path)?;
    let mut data = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= MAX_FILE,
        "Icon pack file exceeds 32 MiB"
    );
    Ok(data)
}
fn inventory(root: &Path) -> Result<Vec<String>> {
    let metadata = fs::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Icon pack must be a real directory"
    );
    fn visit(
        root: &Path,
        relative_dir: &str,
        files: &mut Vec<String>,
        count: &mut usize,
        total: &mut u64,
    ) -> Result<()> {
        for entry in fs::read_dir(root.join(relative_dir))? {
            let entry = entry?;
            *count += 1;
            ensure!(
                *count <= MAX_FILES,
                "Icon pack exceeds 10000 files/directories"
            );
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Icon pack filename is not UTF-8"))?;
            let path = if relative_dir.is_empty() {
                name
            } else {
                format!("{relative_dir}/{name}")
            };
            relative(&path)?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                visit(root, &path, files, count, total)?;
            } else {
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "Icon pack contains a link or special file"
                );
                ensure!(metadata.len() <= MAX_FILE, "Icon pack file exceeds 32 MiB");
                *total = total
                    .checked_add(metadata.len())
                    .context("Icon pack size overflow")?;
                ensure!(*total <= MAX_TOTAL, "Icon pack exceeds 128 MiB");
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, "", &mut files, &mut 0, &mut 0)?;
    files.sort();
    Ok(files)
}
fn image(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "svg" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
            )
        })
}
pub fn load(root: &Path, reference: Option<&Path>) -> Result<Pack> {
    let folder = folder(root)?;
    let id = pack_id(&folder)?;
    let files = inventory(root)?;
    let manifest = crate::package::read_installed_manifest(root, reference)?;
    let text = |key: &str| -> Result<String> {
        Ok(manifest[key]
            .as_str()
            .with_context(|| format!("Icon pack {key} must be text"))?
            .into())
    };
    let name = text("Name")?;
    ensure!(!name.trim().is_empty(), "Icon pack name is empty");
    let icon = text("Icon")?;
    relative(&icon)?;
    let mut metadata = BTreeMap::new();
    if files.iter().any(|p| p == "icons.json") {
        let bytes = read(root, "icons.json")?;
        let entries: Vec<Icon> =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .context("Invalid icon pack icons.json")?;
        ensure!(entries.len() <= MAX_FILES, "Too many icon metadata entries");
        for entry in entries {
            relative(&entry.path)?;
            ensure!(
                metadata.insert(entry.path.clone(), entry).is_none(),
                "Duplicate icon metadata path"
            );
        }
    }
    let mut icons = Vec::new();
    for path in &files {
        if let Some(path) = path.strip_prefix("icons/").filter(|p| image(p)) {
            icons.push(metadata.remove(path).unwrap_or_else(|| {
                Icon {
                    path: path.into(),
                    name: Path::new(path)
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    tags: Vec::new(),
                }
            }));
        }
    }
    let warnings = if metadata.is_empty() {
        vec![]
    } else {
        vec![format!(
            "{} metadata entries have no supported image file",
            metadata.len()
        )]
    };
    ensure!(!icons.is_empty(), "Icon pack contains no supported images");
    let flag = |key: &str| manifest[key] == true || manifest[key] == "true";
    let pinned = flag("pinned");
    Ok(Pack {
        revision: revision(root, &files)?,
        pinned,
        can_hide: !pinned || flag("canhide"),
        hidden: false,
        id,
        name,
        author: text("Author")?,
        version: text("Version")?,
        description: manifest["Description"].as_str().unwrap_or("").into(),
        icon,
        icons,
        warnings,
    })
}
struct StoreLock(fs::File);
impl StoreLock {
    fn acquire(destination: &Path) -> Result<Self> {
        crate::package::reject_symlinks(destination)?;
        fs::create_dir_all(destination)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(destination.join(".deckforge.lock"))?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Icon library lock is not private"
        );
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "Icon library is busy; try again"
        );
        Ok(Self(file))
    }
}
impl Drop for StoreLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
struct Staged {
    stage: tempfile::TempDir,
    root: PathBuf,
    pack: Pack,
}
pub fn install(source: &Path, destination: &Path, reference: Option<&Path>) -> Result<Pack> {
    let _lock = StoreLock::acquire(destination)?;
    let mut staged = stage(source, destination, reference)?;
    staged.pack.hidden = management::visibility(destination)
        .unwrap_or_default()
        .contains(&staged.pack.id)
        && staged.pack.can_hide;
    crate::package::rename_without_replacing(
        &staged.root,
        &destination.join(folder(&staged.root)?),
    )
    .context("Icon pack already exists or cannot be installed")?;
    Ok(staged.pack)
}
fn stage(source: &Path, destination: &Path, reference: Option<&Path>) -> Result<Staged> {
    let stage = tempfile::Builder::new()
        .prefix(".deckforge-icons-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(destination)?;
    let (name, entries) = if source.is_dir() {
        let name = folder(source)?;
        let mut entries = BTreeMap::new();
        let mut total = 0u64;
        for path in inventory(source)? {
            let bytes = read(source, &path)?;
            total = total
                .checked_add(bytes.len() as u64)
                .context("Icon pack size overflow")?;
            ensure!(total <= MAX_TOTAL, "Icon pack exceeds 128 MiB");
            entries.insert(
                format!("{name}/{path}"),
                crate::package::Entry {
                    bytes: Some(bytes),
                    executable: false,
                },
            );
        }
        (name, entries)
    } else {
        crate::package::read_archive_entries(source, EXT)?
    };
    pack_id(&name)?;
    let root = stage.path().join(&name);
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    for (name, entry) in entries {
        let path = stage.path().join(name);
        if let Some(bytes) = entry.bytes {
            fs::create_dir_all(path.parent().context("Icon pack file parent missing")?)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            output.write_all(&bytes)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        } else {
            fs::create_dir_all(path)?;
        }
    }
    let pack = load(&root, reference)?;
    Ok(Staged { stage, root, pack })
}

pub fn catalog(destination: &Path, reference: Option<&Path>) -> Result<Catalog> {
    if !destination.exists() {
        return Ok(Catalog {
            packs: vec![],
            errors: vec![],
            archives: vec![],
        });
    }
    let _lock = StoreLock::acquire(destination)?;
    let mut errors = Vec::new();
    let hidden = match management::visibility(destination) {
        Ok(hidden) => hidden,
        Err(e) => {
            errors.push(format!("Visibility preferences: {e}"));
            Default::default()
        }
    };
    let mut packs = Vec::new();
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().ends_with(EXT) {
            continue;
        }
        match load(&entry.path(), reference) {
            Ok(mut pack) => {
                pack.hidden = hidden.contains(&pack.id) && pack.can_hide;
                packs.push(pack);
            }
            Err(e) => errors.push(format!("{}: {e}", entry.file_name().to_string_lossy())),
        }
    }
    packs.sort_by_cached_key(|p| (!p.pinned, p.name.to_lowercase(), p.id.clone()));
    let archives = management::archives(destination, &mut errors)?;
    errors.sort();
    Ok(Catalog {
        packs,
        errors,
        archives,
    })
}
fn asset(
    destination: &Path,
    id: &str,
    path: &str,
    reference: Option<&Path>,
    expected: Option<&str>,
) -> Result<PathBuf> {
    let name = format!("{id}{EXT}");
    ensure!(pack_id(&name)? == id, "Invalid icon pack ID");
    relative(path)?;
    let root = destination.join(name);
    let pack = load(&root, reference)?;
    ensure!(
        expected.is_none_or(|e| e == pack.revision),
        "Icon pack changed; reload the library"
    );
    ensure!(
        pack.icons.iter().any(|icon| icon.path == path),
        "Icon is not in this pack"
    );
    Ok(root.join("icons").join(path))
}
pub fn thumbnail(
    destination: &Path,
    id: &str,
    path: &str,
    reference: Option<&Path>,
    expected: Option<&str>,
) -> Result<String> {
    use base64::Engine;
    let _lock = StoreLock::acquire(destination)?;
    let path = asset(destination, id, path, reference, expected)?;
    let root = destination.join(format!("{id}{EXT}"));
    let relative = path
        .strip_prefix(&root)?
        .to_str()
        .context("Invalid icon path")?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD
            .encode(crate::action_icons::png(&root, relative)?)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="#a03264"/></svg>"##;
    pub(super) fn fixture(root: &Path) -> PathBuf {
        let pack = root.join("org.test.icons.sdIconPack");
        fs::create_dir_all(pack.join("icons")).unwrap();
        fs::write(pack.join("manifest.json"),serde_json::to_vec(&json!({"Name":"Test icons","Author":"Test author","Version":"1.0","Icon":"cover.svg","Unknown":{"keep":true},"UUID":"opaque-extra-field"})).unwrap()).unwrap();
        fs::write(pack.join("cover.svg"), SVG).unwrap();
        fs::write(pack.join("icons/first.svg"), SVG).unwrap();
        fs::write(pack.join("icons/unlisted.svg"), SVG).unwrap();
        fs::write(
            pack.join("icons.json"),
            br#"[{"path":"first.svg","name":"Named icon","tags":["tag-search"]}]"#,
        )
        .unwrap();
        pack
    }
    fn zip(root: &Path, path: &Path, extra: Option<(&str, &[u8])>) {
        let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let folder = folder(root).unwrap();
        for file in inventory(root).unwrap() {
            writer
                .start_file(
                    format!("{folder}/{file}"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(&read(root, &file).unwrap()).unwrap();
        }
        if let Some((name, bytes)) = extra {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    #[test]
    fn directory_and_zip_install_preserve_bytes_metadata_and_unlisted_icons() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path());
        let archive = temp.path().join("pack.streamDeckIconPack");
        zip(&root, &archive, None);
        for (i, source) in [root.clone(), archive].iter().enumerate() {
            let dest = temp.path().join(format!("installed-{i}"));
            let pack = install(source, &dest, None).unwrap();
            assert_eq!(pack.id, "org.test.icons");
            assert_eq!(pack.icons.len(), 2);
            assert_eq!(pack.icons[0].tags, vec!["tag-search"]);
            assert_eq!(pack.icons[1].name, "unlisted");
            let installed = dest.join(root.file_name().unwrap());
            for file in inventory(&root).unwrap() {
                assert_eq!(
                    read(&root, &file).unwrap(),
                    read(&installed, &file).unwrap()
                );
            }
            assert_eq!(catalog(&dest, None).unwrap().packs.len(), 1);
            assert!(install(source, &dest, None).is_err());
            assert_eq!(
                fs::read_dir(&dest)
                    .unwrap()
                    .filter(|e| e.as_ref().unwrap().file_name() != ".deckforge.lock")
                    .count(),
                1
            );
            assert!(
                thumbnail(&dest, &pack.id, "first.svg", None, None)
                    .unwrap()
                    .starts_with("data:image/png;base64,")
            );
            assert!(asset(&dest, "../outside", "first.svg", None, None).is_err());
            assert!(asset(&dest, &pack.id, "../manifest.json", None, None).is_err());
        }
    }
    #[test]
    fn invalid_metadata_and_links_never_publish_a_pack() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path());
        let dest = temp.path().join("installed");
        for metadata in [
            br#"[{"path":"../cover.svg","name":"Bad"}]"#.as_slice(),
            br#"[{"path":"first.svg","name":"One"},{"path":"first.svg","name":"Two"}]"#,
            br#"{"not":"an array"}"#,
        ] {
            fs::write(root.join("icons.json"), metadata).unwrap();
            assert!(install(&root, &dest, None).is_err());
            assert_eq!(
                fs::read_dir(&dest)
                    .unwrap()
                    .filter(|e| e.as_ref().unwrap().file_name() != ".deckforge.lock")
                    .count(),
                0
            );
        }
        fs::remove_file(root.join("icons.json")).unwrap();
        std::os::unix::fs::symlink(root.join("cover.svg"), root.join("icons/link.svg")).unwrap();
        assert!(install(&root, &dest, None).is_err());
        assert_eq!(
            fs::read_dir(&dest)
                .unwrap()
                .filter(|e| e.as_ref().unwrap().file_name() != ".deckforge.lock")
                .count(),
            0
        );
    }
    #[test]
    fn unsafe_archive_entries_and_special_source_files_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path());
        let archive = temp.path().join("unsafe.zip");
        let dest = temp.path().join("installed");
        zip(
            &root,
            &archive,
            Some(("org.test.icons.sdIconPack/../escape", b"bad")),
        );
        assert!(install(&archive, &dest, None).is_err());
        assert_eq!(
            fs::read_dir(&dest)
                .unwrap()
                .filter(|e| e.as_ref().unwrap().file_name() != ".deckforge.lock")
                .count(),
            0
        );
        let fifo = temp.path().join("blocked.streamDeckIconPack");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(install(&fifo, &dest, None).is_err());
    }
    #[test]
    #[ignore = "requires supplied Stream Deck 7.6 Key Creator artifact"]
    fn original_six_packs_install_with_all_2361_images_and_exact_bytes() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../evidence/elgato-7.6/raw/flsy_lZh6qrewVZPcqeNQmr2lhuQjI");
        let mut zip = zip::ZipArchive::new(fs::File::open(source).unwrap()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let origin = temp.path().join("original");
        fs::create_dir(&origin).unwrap();
        for index in 0..zip.len() {
            let mut file = zip.by_index(index).unwrap();
            let Some(path) = file
                .name()
                .strip_prefix("com.elgato.keycreator.sdPlugin/static/")
                .filter(|p| p.contains(".sdIconPack/"))
            else {
                continue;
            };
            let path = path.trim_end_matches('/').to_owned();
            relative(&path).unwrap();
            let dest = origin.join(&path);
            if file.is_dir() {
                fs::create_dir_all(dest).unwrap();
            } else {
                fs::create_dir_all(dest.parent().unwrap()).unwrap();
                let mut out = fs::File::create(dest).unwrap();
                std::io::copy(&mut file, &mut out).unwrap();
            }
        }
        let destination = temp.path().join("installed");
        let mut count = 0;
        let mut packs = 0;
        for root in fs::read_dir(&origin).unwrap() {
            let root = root.unwrap().path();
            if !root.join("manifest.json").is_file() {
                continue;
            }
            let pack = install(&root, &destination, None).unwrap();
            count += pack.icons.len();
            packs += 1;
            let installed = destination.join(root.file_name().unwrap());
            for file in inventory(&root).unwrap() {
                assert_eq!(
                    read(&root, &file).unwrap(),
                    read(&installed, &file).unwrap()
                );
            }
            assert!(pack.warnings.is_empty());
            assert!(
                thumbnail(&destination, &pack.id, &pack.icons[0].path, None, None)
                    .unwrap()
                    .starts_with("data:image/png;base64,")
            );
        }
        assert_eq!(packs, 6);
        assert_eq!(count, 2361);
        assert_eq!(catalog(&destination, None).unwrap().packs.len(), 6);
    }
}
