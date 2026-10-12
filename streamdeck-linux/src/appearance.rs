//! Official title-parameter field names; concrete defaults and size limits below
//! are Deckforge's Linux policy, not a claim about Windows pixel metrics.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::Read,
    os::fd::AsRawFd,
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TitleStyle {
    pub font_family: String,
    pub font_size: f32,
    pub font_style: String,
    pub font_underline: bool,
    pub show_title: bool,
    pub title_alignment: String,
    pub title_color: String,
}
impl Default for TitleStyle {
    fn default() -> Self {
        Self {
            font_family: "Arial".into(),
            font_size: 12.,
            font_style: "Regular".into(),
            font_underline: false,
            show_title: true,
            title_alignment: "bottom".into(),
            title_color: "#ffffff".into(),
        }
    }
}
const FIELDS: [(&str, &str); 7] = [
    ("FontFamily", "fontFamily"),
    ("FontSize", "fontSize"),
    ("FontStyle", "fontStyle"),
    ("FontUnderline", "fontUnderline"),
    ("ShowTitle", "showTitle"),
    ("TitleAlignment", "titleAlignment"),
    ("TitleColor", "titleColor"),
];
impl TitleStyle {
    pub fn from_state(state: &Value) -> Result<Self> {
        ensure!(
            state.is_null() || state.is_object(),
            "Action state must be an object"
        );
        let mut dto = serde_json::Map::new();
        for (official, field) in FIELDS {
            if let Some(value) = state.get(official) {
                dto.insert(field.into(), value.clone());
            }
        }
        let result: Self =
            serde_json::from_value(Value::Object(dto)).context("Invalid title style")?;
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.font_family.len() <= 256 && !self.font_family.chars().any(char::is_control),
            "Font family must be at most 256 bytes without control characters"
        );
        ensure!(
            self.font_size.is_finite() && (1.0..=96.0).contains(&self.font_size),
            "Font size must be between 1 and 96"
        );
        ensure!(
            ["", "Regular", "Bold", "Italic", "Bold Italic"].contains(&self.font_style.as_str()),
            "Invalid font style"
        );
        ensure!(
            ["top", "middle", "bottom"].contains(&self.title_alignment.as_str()),
            "Invalid title alignment"
        );
        let color = self
            .title_color
            .strip_prefix('#')
            .unwrap_or(&self.title_color);
        ensure!(
            color.len() == 6 && color.bytes().all(|c| c.is_ascii_hexdigit()),
            "Title color must be a six-digit hexadecimal RGB color"
        );
        Ok(())
    }
    pub fn write_state(&self, state: &mut Value) -> Result<()> {
        self.validate()?;
        ensure!(
            state.is_null() || state.is_object(),
            "Action state must be an object"
        );
        if state.is_null() {
            *state = json!({});
        }
        let dto = serde_json::to_value(self)?;
        for (official, field) in FIELDS {
            state[official] = dto[field].clone();
        }
        Ok(())
    }
    pub(crate) fn bold(&self) -> bool {
        self.font_style.contains("Bold")
    }
    pub(crate) fn italic(&self) -> bool {
        self.font_style.contains("Italic")
    }
    pub(crate) fn rgba(&self) -> [u8; 4] {
        let color = self
            .title_color
            .strip_prefix('#')
            .unwrap_or(&self.title_color);
        let value = u32::from_str_radix(color, 16).unwrap_or(0xffffff);
        [(value >> 16) as u8, (value >> 8) as u8, value as u8, 255]
    }
}

/// Cached inventory only; rendering font selection has its own bounded cache.
pub fn font_families() -> Result<Vec<String>> {
    static FAMILIES: OnceLock<std::result::Result<Vec<String>, String>> = OnceLock::new();
    match FAMILIES.get_or_init(|| {
        let result = (|| -> Result<Vec<String>> {
            let bytes = fontconfig(
                "fc-list",
                &["--format", "%{family}\n"],
                1024 * 1024,
                Duration::from_secs(2),
            )?;
            let text = String::from_utf8(bytes).context("Font inventory is not UTF-8")?;
            let families: BTreeSet<String> = text
                .lines()
                .flat_map(|line| line.split(','))
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
                .map(str::to_owned)
                .collect();
            ensure!(!families.is_empty(), "No font families found");
            Ok(families.into_iter().collect())
        })();
        result.map_err(|e| format!("{e:#}"))
    }) {
        Ok(value) => Ok(value.clone()),
        Err(error) => bail!("{error}"),
    }
}
/// Fontconfig's documented family-name escaping; never accept a family as a
/// property expression such as ':file=/path'. Arguments never pass through a shell.
pub(crate) fn font_pattern(style: &TitleStyle) -> String {
    let family = if style.font_family.is_empty() {
        "Arial"
    } else {
        &style.font_family
    };
    let mut escaped = String::new();
    for c in family.chars() {
        if ['\\', '-', ':', ','].contains(&c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!(
        "{escaped}:style={}",
        if style.font_style.is_empty() {
            "Regular"
        } else {
            &style.font_style
        }
    )
}
pub(crate) fn fontconfig(
    program: &str,
    args: &[&str],
    limit: usize,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .env_remove("FC_DEBUG")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Start {program}"))?;
    let result = (|| -> Result<Vec<u8>> {
        let mut pipe = child
            .stdout
            .take()
            .context("Font command stdout unavailable")?;
        // SAFETY: fcntl acts only on the owned pipe descriptor; no foreign fd is changed.
        let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
        ensure!(
            flags >= 0
                && unsafe {
                    libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
                } >= 0,
            "Cannot bound font command output"
        );
        let deadline = Instant::now() + timeout;
        let mut bytes = vec![];
        let mut exited = None;
        loop {
            let mut buffer = [0; 8192];
            let eof = match pipe.read(&mut buffer) {
                Ok(0) => true,
                Ok(n) => {
                    ensure!(
                        bytes.len() + n <= limit,
                        "Font command output exceeds limit"
                    );
                    bytes.extend_from_slice(&buffer[..n]);
                    false
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            if exited.is_none() {
                exited = child.try_wait()?;
            }
            if eof && let Some(status) = exited {
                ensure!(status.success(), "Font command failed");
                return Ok(bytes);
            }
            ensure!(Instant::now() < deadline, "Font command timed out");
            thread::sleep(Duration::from_millis(2));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_roundtrip_preserves_empty_family_and_unknown_fields() {
        let mut state = json!({"FontFamily":"","FontStyle":"","FontSize":9,"Image":"asset.png","future":{"x":1}});
        let style = TitleStyle::from_state(&state).unwrap();
        assert_eq!(style.font_family, "");
        style.write_state(&mut state).unwrap();
        assert_eq!(state["FontFamily"], "");
        assert_eq!(state["FontStyle"], "");
        assert_eq!(state["future"], json!({"x":1}));
        assert_eq!(state["Image"], "asset.png");
        assert_eq!(serde_json::to_value(style).unwrap()["fontSize"], 9.0);
    }
    #[test]
    fn malformed_fields_are_rejected_without_mutation() {
        for state in [
            json!({"FontSize":1e20}),
            json!({"FontFamily":"bad\nname"}),
            json!({"FontStyle":"Unknown"}),
            json!({"TitleColor":"red"}),
            json!({"TitleAlignment":"left"}),
        ] {
            assert!(TitleStyle::from_state(&state).is_err());
        }
    }
    #[test]
    fn font_family_is_not_a_fontconfig_expression() {
        let style = TitleStyle {
            font_family: "A-B:file=/tmp/untrusted,font".into(),
            ..TitleStyle::default()
        };
        assert_eq!(
            font_pattern(&style),
            "A\\-B\\:file=/tmp/untrusted\\,font:style=Regular"
        );
    }
}
