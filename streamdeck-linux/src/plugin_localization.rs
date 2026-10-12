//! Host manifest presentation only. Plugin-owned Localization dictionaries and
//! property inspector DOM strings remain the plugin's responsibility.
//! https://docs.elgato.com/streamdeck/sdk/guides/i18n/
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use std::{
    ffi::{CString, OsStr},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::{Component, Path},
};

pub const LANGUAGES: [(&str, &str); 8] = [
    ("de", "Deutsch"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("zh_CN", "简体中文"),
    ("zh_TW", "繁體中文"),
];
const MAX_BYTES: u64 = 1024 * 1024;

pub fn resolve(preference: &str) -> Result<&'static str> {
    if preference == "system" {
        let variables = ["LC_ALL", "LC_MESSAGES", "LANG"].map(std::env::var_os);
        return Ok(system_language(
            variables.iter().flatten().map(|s| s.as_os_str()),
        ));
    }
    LANGUAGES
        .iter()
        .find(|(code, _)| *code == preference)
        .map(|(code, _)| *code)
        .context("Unsupported plugin language; choose system or a supported language code")
}
fn system_language<'a>(values: impl IntoIterator<Item = &'a OsStr>) -> &'static str {
    values
        .into_iter()
        .find(|v| !v.is_empty())
        .and_then(OsStr::to_str)
        .map(normalize)
        .unwrap_or("en")
}
fn normalize(locale: &str) -> &'static str {
    let locale = locale
        .trim()
        .split(['.', '@'])
        .next()
        .unwrap_or("")
        .replace('-', "_")
        .to_ascii_lowercase();
    let parts: Vec<_> = locale.split('_').collect();
    if parts.first() == Some(&"zh") {
        // Linux normalization policy: explicit script wins over region.
        if parts.contains(&"hant") {
            return "zh_TW";
        }
        if parts.contains(&"hans") {
            return "zh_CN";
        }
        return if parts.iter().any(|s| matches!(*s, "tw" | "hk" | "mo")) {
            "zh_TW"
        } else {
            "zh_CN"
        };
    }
    LANGUAGES
        .iter()
        .find(|(code, _)| Some(code) == parts.first())
        .map(|(code, _)| *code)
        .unwrap_or("en")
}

pub struct Presentation {
    pub manifest: Value,
    pub warning: Option<String>,
}
pub fn load(root: &Path, manifest: &Value, language: &str) -> Presentation {
    let mut result = Presentation {
        manifest: manifest.clone(),
        warning: None,
    };
    // load takes an already resolved language, never an arbitrary path fragment.
    if !LANGUAGES.iter().any(|(code, _)| *code == language) {
        result.warning = Some("Plugin localization language is unsupported".into());
        return result;
    }
    match read_translation(root, language) {
        Ok(None) => {}
        Ok(Some(value)) => {
            if let Some(translations) = value.as_object() {
                let mut invalid = 0;
                overlay(&mut result.manifest, translations, &mut invalid);
                if invalid > 0 {
                    result.warning = Some(format!(
                        "Plugin localization ignored {invalid} invalid display text fields"
                    ));
                }
            } else {
                result.warning = Some("Plugin localization must contain a JSON object".into());
            }
        }
        Err(error) => result.warning = Some(format!("Plugin localization unavailable: {error}")),
    }
    result
}
fn read_translation(root: &Path, language: &str) -> Result<Option<Value>> {
    let root = root
        .canonicalize()
        .context("cannot resolve plugin directory")?;
    let root_file = root.join(format!("{language}.json"));
    let candidate = match fs::symlink_metadata(&root_file) {
        Ok(_) => root_file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            root.join("locales").join(format!("{language}.json"))
        }
        Err(_) => bail!("cannot inspect language file"),
    };
    match fs::symlink_metadata(&candidate) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("cannot inspect language file"),
    }
    let resolved = candidate
        .canonicalize()
        .context("cannot resolve language file")?;
    let relative = resolved
        .strip_prefix(&root)
        .context("language file escapes plugin directory")?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&root)
        .context("cannot open plugin directory")?;
    let parts: Vec<_> = relative.components().collect();
    ensure!(!parts.is_empty(), "language file is not a regular file");
    // Pin every resolved directory, rejecting symlink substitutions between
    // containment checking and open. Internal symlinks are allowed if resolved
    // inside the plugin; no external target or nonregular file is read.
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            bail!("invalid language file path");
        };
        let name = CString::new(name.as_bytes()).context("invalid language filename")?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if index + 1 < parts.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: pinned directory FD, validated C name. Each successful result
        // transfers into File exactly once; intermediate FDs close on assignment.
        let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
        ensure!(fd >= 0, "cannot safely open language file");
        file = unsafe { File::from_raw_fd(fd) };
    }
    let metadata = file
        .metadata()
        .context("cannot inspect opened language file")?;
    ensure!(metadata.is_file(), "language file is not a regular file");
    ensure!(metadata.len() <= MAX_BYTES, "language file exceeds 1 MiB");
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("cannot read language file")?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "language file exceeds 1 MiB"
    );
    // serde_json's default recursion limit stays enabled. Never put parser
    // excerpts or translated content into diagnostics.
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("language file contains invalid JSON"))
}
fn strings(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
    keys: &[&str],
    invalid: &mut usize,
) {
    for key in keys {
        match source.get(*key) {
            None => {}
            Some(Value::String(text)) if text.is_empty() => {}
            Some(Value::String(text)) => {
                target.insert((*key).into(), Value::String(text.clone()));
            }
            Some(_) => *invalid += 1,
        }
    }
}
fn overlay(manifest: &mut Value, translations: &Map<String, Value>, invalid: &mut usize) {
    let Some(root) = manifest.as_object_mut() else {
        return;
    };
    strings(root, translations, &["Name", "Description"], invalid);
    let Some(actions) = root.get_mut("Actions").and_then(Value::as_array_mut) else {
        return;
    };
    for action in actions {
        let Some(action) = action.as_object_mut() else {
            continue;
        };
        let Some(value) = action
            .get("UUID")
            .and_then(Value::as_str)
            .and_then(|uuid| translations.get(uuid))
        else {
            continue;
        };
        let Some(local) = value.as_object() else {
            *invalid += 1;
            continue;
        };
        strings(action, local, &["Name", "Tooltip"], invalid);
        if let Some(value) = local.get("States") {
            if let Some(local_states) = value.as_array() {
                if let Some(states) = action.get_mut("States").and_then(Value::as_array_mut) {
                    for (state, local) in states.iter_mut().zip(local_states) {
                        match (state.as_object_mut(), local.as_object()) {
                            (Some(state), Some(local)) => strings(state, local, &["Name"], invalid),
                            (_, None) => *invalid += 1,
                            _ => {}
                        }
                    }
                }
            } else {
                *invalid += 1;
            }
        }
        if let Some(value) = local.get("Encoder") {
            if let Some(local_encoder) = value.as_object() {
                if let Some(value) = local_encoder.get("TriggerDescription") {
                    if let Some(local_trigger) = value.as_object() {
                        if let Some(trigger) = action
                            .get_mut("Encoder")
                            .and_then(|e| e.get_mut("TriggerDescription"))
                            .and_then(Value::as_object_mut)
                        {
                            strings(
                                trigger,
                                local_trigger,
                                &["Rotate", "Push", "Touch", "LongTouch"],
                                invalid,
                            );
                        }
                    } else {
                        *invalid += 1;
                    }
                }
            } else {
                *invalid += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::symlink;

    fn base() -> Value {
        json!({"Name":"Base","Description":"Original","Category":"Category","CodePath":"bin/main",
            "Actions":[{"UUID":"sample.action","Name":"Action","Tooltip":"Tip","Settings":{"secret":"preserved"},
                "States":[{"Name":"Off","Title":"User title","Image":"icon"},{"Name":"On"}],
                "Encoder":{"TriggerDescription":{"Rotate":"Turn","Push":"Push","Touch":"Touch","LongTouch":"Hold"}}}]})
    }
    fn write(root: &Path, name: &str, value: &Value) {
        fs::write(root.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }
    #[test]
    fn explicit_languages_and_linux_locale_normalization() {
        for (code, _) in LANGUAGES {
            assert_eq!(resolve(code).unwrap(), code);
        }
        for invalid in ["de_DE", "../en", "EN", "", "zh"] {
            assert!(resolve(invalid).is_err());
        }
        for (input, expected) in [
            ("de_DE.UTF-8", "de"),
            ("fr_CA@euro", "fr"),
            ("ja_JP.utf8", "ja"),
            ("C.UTF-8", "en"),
            ("POSIX", "en"),
            ("nb_NO.UTF-8", "en"),
            ("zh-Hant-HK", "zh_TW"),
            ("zh_Hans_TW.UTF-8", "zh_CN"),
            ("zh_TW", "zh_TW"),
            ("zh_HK", "zh_TW"),
            ("zh_MO", "zh_TW"),
            ("zh_SG", "zh_CN"),
            ("zh", "zh_CN"),
        ] {
            assert_eq!(normalize(input), expected, "{input}");
        }
        assert_eq!(
            system_language([OsStr::new(""), OsStr::new("fr_CA"), OsStr::new("de_DE")]),
            "fr"
        );
        assert_eq!(
            system_language([OsStr::new("unsupported"), OsStr::new("de_DE")]),
            "en"
        );
        assert_eq!(
            system_language([OsStr::from_bytes(&[0xff]), OsStr::new("de")]),
            "en"
        );
    }
    #[test]
    fn overlays_only_documented_strings_without_mutating_source_or_structure() {
        let root = tempfile::tempdir().unwrap();
        let original = base();
        let before = original.clone();
        write(
            root.path(),
            "de.json",
            &json!({"Name":"Plugin übersetzt","Description":"Beschreibung","Category":"BAD","CodePath":"BAD",
            "Actions":[{"UUID":"BAD"}],"sample.action":{"UUID":"BAD","Name":"Aktion","Tooltip":"Hinweis","Settings":{"secret":"BAD"},
                "States":[{"Name":"Aus","Title":"BAD","Image":"BAD"},{"Name":"An"},{"Name":"EXTRA"}],
                "Encoder":{"TriggerDescription":{"Rotate":"Drehen","Push":"Drücken","Touch":"Berühren","LongTouch":"Halten","Unknown":"BAD"}}}}),
        );
        let presentation = load(root.path(), &original, "de");
        assert!(presentation.warning.is_none());
        let mut expected = original.clone();
        expected["Name"] = json!("Plugin übersetzt");
        expected["Description"] = json!("Beschreibung");
        expected["Actions"][0]["Name"] = json!("Aktion");
        expected["Actions"][0]["Tooltip"] = json!("Hinweis");
        expected["Actions"][0]["States"][0]["Name"] = json!("Aus");
        expected["Actions"][0]["States"][1]["Name"] = json!("An");
        expected["Actions"][0]["Encoder"]["TriggerDescription"] =
            json!({"Rotate":"Drehen","Push":"Drücken","Touch":"Berühren","LongTouch":"Halten"});
        assert_eq!(presentation.manifest, expected);
        assert_eq!(original, before);
    }
    #[test]
    fn root_precedes_locales_and_missing_never_falls_back_to_english() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("locales")).unwrap();
        write(root.path(), "en.json", &json!({"Name":"English override"}));
        write(root.path(), "locales/de.json", &json!({"Name":"Nested"}));
        assert_eq!(
            load(root.path(), &base(), "en").manifest["Name"],
            "English override"
        );
        assert_eq!(load(root.path(), &base(), "fr").manifest, base());
        assert_eq!(load(root.path(), &base(), "de").manifest["Name"], "Nested");
        write(root.path(), "de.json", &json!({"Name":"Root"}));
        assert_eq!(load(root.path(), &base(), "de").manifest["Name"], "Root");
        fs::write(root.path().join("de.json"), b"malformed").unwrap();
        let invalid = load(root.path(), &base(), "de");
        assert_eq!(invalid.manifest, base());
        assert!(invalid.warning.is_some());
    }
    #[test]
    fn invalid_fields_warn_but_valid_fields_survive_and_empty_strings_fall_back() {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            "de.json",
            &json!({"Name":"","Description":null,"sample.action":{"Name":"Good","Tooltip":22,"States":[{"Name":false},null],"Encoder":{"TriggerDescription":{"Rotate":"","Push":{},"Touch":"Tip"}}}}),
        );
        let p = load(root.path(), &base(), "de");
        assert!(p.warning.unwrap().contains("5 invalid"));
        assert_eq!(p.manifest["Name"], "Base");
        assert_eq!(p.manifest["Actions"][0]["Name"], "Good");
        assert_eq!(
            p.manifest["Actions"][0]["States"],
            base()["Actions"][0]["States"]
        );
        assert_eq!(
            p.manifest["Actions"][0]["Encoder"]["TriggerDescription"]["Rotate"],
            "Turn"
        );
        assert_eq!(
            p.manifest["Actions"][0]["Encoder"]["TriggerDescription"]["Touch"],
            "Tip"
        );
    }
    #[test]
    fn rejects_escape_fifo_oversize_depth_and_invalid_language_without_reading() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "escape.json", &json!({"Name":"External"}));
        let path = root.path().join("de.json");
        symlink(outside.path().join("escape.json"), &path).unwrap();
        let escaped = load(root.path(), &base(), "de");
        assert_eq!(escaped.manifest, base());
        assert!(escaped.warning.unwrap().contains("escapes"));
        fs::remove_file(&path).unwrap();
        let c = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let start = std::time::Instant::now();
        assert!(
            load(root.path(), &base(), "de")
                .warning
                .unwrap()
                .contains("regular")
        );
        assert!(start.elapsed() < std::time::Duration::from_millis(200));
        fs::remove_file(&path).unwrap();
        File::create(&path).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert!(
            load(root.path(), &base(), "de")
                .warning
                .unwrap()
                .contains("1 MiB")
        );
        fs::write(&path, format!("{}0{}", "[".repeat(200), "]".repeat(200))).unwrap();
        assert!(
            load(root.path(), &base(), "de")
                .warning
                .unwrap()
                .contains("invalid JSON")
        );
        assert_eq!(load(root.path(), &base(), "../../escape").manifest, base());
    }
    #[test]
    fn internal_symlinks_work_but_dangling_root_does_not_fall_through() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("locales")).unwrap();
        write(root.path(), "locales/de.json", &json!({"Name":"Inside"}));
        symlink("locales/de.json", root.path().join("de.json")).unwrap();
        assert_eq!(load(root.path(), &base(), "de").manifest["Name"], "Inside");
        fs::remove_file(root.path().join("de.json")).unwrap();
        symlink("missing.json", root.path().join("de.json")).unwrap();
        let p = load(root.path(), &base(), "de");
        assert_eq!(p.manifest, base());
        assert!(p.warning.is_some());
    }
}
