//! Shipped public packages are offered explicitly; existing installs stay untouched.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
const ARCHIVE: &str = "discord-for-deckforge.streamDeckPlugin";
#[derive(Serialize)]
pub struct Bundle {
    id: String,
    name: String,
    version: String,
    path: PathBuf,
}
fn inspect(path: &Path) -> Result<Bundle> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() <= 512 * 1024 * 1024,
        "Bundled package is too large"
    );
    let mut archive = zip::ZipArchive::new(file)?;
    let entry = archive.by_name("com.elgato.discord.sdPlugin/manifest.json")?;
    ensure!(entry.size() <= 128 * 1024, "Bundled manifest is too large");
    let manifest: Value = serde_json::from_reader(entry.take(128 * 1024 + 1))?;
    ensure!(
        manifest["UUID"] == "com.elgato.discord",
        "Unexpected bundled plugin identity"
    );
    Ok(Bundle {
        id: "com.elgato.discord".into(),
        name: manifest["Name"]
            .as_str()
            .context("Missing bundled name")?
            .into(),
        version: manifest["Version"]
            .as_str()
            .context("Missing bundled version")?
            .into(),
        path: path.to_owned(),
    })
}
pub fn list() -> Result<Vec<Bundle>> {
    let executable = std::env::current_exe()?;
    let mut candidates = vec![
        executable
            .parent()
            .context("Missing executable directory")?
            .join("bundled-plugins")
            .join(ARCHIVE),
    ];
    if let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    {
        candidates.push(data.join("deckforge/bundled-plugins").join(ARCHIVE));
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(vec![inspect(&candidate)?]);
        }
    }
    Ok(vec![])
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn reads_only_the_expected_public_manifest() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(ARCHIVE);
        let mut archive = zip::ZipWriter::new(File::create(&path).unwrap());
        archive
            .start_file(
                "com.elgato.discord.sdPlugin/manifest.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"{"UUID":"com.elgato.discord","Name":"Discord for Deckforge","Version":"0.1.0"}"#).unwrap();
        archive.finish().unwrap();
        assert_eq!(inspect(&path).unwrap().id, "com.elgato.discord");
        std::fs::write(&path, b"not a package").unwrap();
        assert!(inspect(&path).is_err());
    }
}
