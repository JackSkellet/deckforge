//! Desktop file targets are local paths, never network downloads or shell input.
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Action,
    Profile,
    Plugin,
    IconPack,
}

pub fn kind(path: &Path) -> Result<Kind> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("streamdeckaction") => Ok(Kind::Action),
        Some("streamdeckprofile") => Ok(Kind::Profile),
        Some("streamdeckplugin") => Ok(Kind::Plugin),
        Some("streamdeckiconpack") => Ok(Kind::IconPack),
        _ => anyhow::bail!("Open a Stream Deck action, profile, plugin or icon-pack file"),
    }
}
pub fn parse(value: &str) -> Result<PathBuf> {
    ensure!(
        value.len() <= 8192 && !value.chars().any(char::is_control),
        "Invalid desktop file path"
    );
    let path = if value
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
    {
        let url = url::Url::parse(value).context("Invalid file URI")?;
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "File URI must not have a query or fragment"
        );
        url.to_file_path()
            .map_err(|_| anyhow::anyhow!("Only local file URIs are supported"))?
    } else {
        ensure!(!value.contains("://"), "Only local files are supported");
        PathBuf::from(value)
    };
    kind(&path)?;
    let path = path.canonicalize().context("Open desktop file")?;
    kind(&path)?;
    ensure!(
        path.to_str()
            .is_some_and(|v| v.len() <= 8192 && !v.chars().any(char::is_control)),
        "Invalid desktop file path"
    );
    ensure!(path.is_file(), "Desktop import must be a regular file");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_extensions_are_local_regular_files_and_encoded_controls_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        for (extension, expected) in [
            ("STREAMDECKPLUGIN", Kind::Plugin),
            ("sTrEaMdEcKiCoNpAcK", Kind::IconPack),
        ] {
            let path = dir.path().join(format!("space # %.{extension}"));
            std::fs::write(&path, b"fixture").unwrap();
            assert_eq!(kind(&path).unwrap(), expected);
            assert_eq!(
                parse(url::Url::from_file_path(&path).unwrap().as_str()).unwrap(),
                path
            );
        }
        let path = dir.path().join("bad\nname.streamDeckPlugin");
        std::fs::write(&path, b"fixture").unwrap();
        assert!(parse(url::Url::from_file_path(path).unwrap().as_str()).is_err());
        let directory = dir.path().join("folder.streamDeckIconPack");
        std::fs::create_dir(&directory).unwrap();
        assert!(parse(directory.to_str().unwrap()).is_err());
    }

    #[test]
    fn local_paths_and_encoded_uris_keep_literal_names_and_reject_remote_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("space % # ' $(literal).streamDeckAction");
        std::fs::write(&path, b"fixture").unwrap();
        assert_eq!(parse(path.to_str().unwrap()).unwrap(), path);
        assert_eq!(
            parse(url::Url::from_file_path(&path).unwrap().as_str()).unwrap(),
            path
        );
        for bad in [
            "https://example.com/test.streamDeckAction",
            "file://remote/tmp/test.streamDeckAction",
            "file:///tmp/test.streamDeckAction?query=1",
            "/tmp/missing.streamDeckProfile",
            "/tmp/code.streamDeckPlugin",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
