//! Plugin callback routing. Keep the encoded suffix intact for the SDK to parse.
use anyhow::{Context, Result, ensure};

pub const MAX_BYTES: usize = 8192;

#[derive(Clone)] // Deliberately no Debug: callbacks can contain authorization codes.
pub struct DeepLink {
    pub plugin: String,
    pub message: String,
    pub hidden: bool,
}
impl DeepLink {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(value.len() <= MAX_BYTES, "Plugin link exceeds 8 KiB");
        ensure!(
            !value.chars().any(char::is_control),
            "Plugin link contains control characters"
        );
        let (scheme, rest) = value.split_once("://").context("Invalid plugin link")?;
        ensure!(
            scheme.eq_ignore_ascii_case("streamdeck"),
            "Unsupported plugin link scheme"
        );
        let (authority, path) = rest.split_once('/').context("Invalid plugin link route")?;
        ensure!(
            authority.eq_ignore_ascii_case("plugins"),
            "Unsupported plugin link destination"
        );
        let path = path
            .strip_prefix("message/")
            .context("Unsupported plugin link route")?;
        let end = path.find(['/', '?', '#']).unwrap_or(path.len());
        let plugin = &path[..end];
        ensure!(
            !plugin.is_empty()
                && plugin.len() <= 255
                && plugin
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')),
            "Invalid plugin identifier in link"
        );
        // Use a parsed query only for foreground policy, never to reconstruct
        // the delivered message or collapse its repeated parameters.
        let parsed = url::Url::parse(value).context("Invalid plugin link")?;
        let hidden = parsed
            .query_pairs()
            .find(|(key, _)| key == "streamdeck")
            .is_some_and(|(_, value)| value.eq_ignore_ascii_case("hidden"));
        Ok(Self {
            plugin: plugin.into(),
            message: path[end..].into(),
            hidden,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suffix_is_preserved_separately_from_passive_query_policy() {
        for suffix in [
            "",
            "/",
            "?code=A%2FB",
            "#done",
            "/Hello%20world?x=%252F&x=a+b&streamdeck=HiDdEn#%2F",
            "/a/../b?",
        ] {
            let link = DeepLink::parse(&format!(
                "streamdeck://plugins/message/Com.Example.Plugin{suffix}"
            ))
            .unwrap();
            assert_eq!(link.plugin, "Com.Example.Plugin");
            assert_eq!(link.message, suffix);
            assert_eq!(link.hidden, suffix.contains("HiDdEn"));
        }
        assert!(
            !DeepLink::parse("streamdeck://plugins/message/a/?Streamdeck=hidden")
                .unwrap()
                .hidden
        );
    }
    #[test]
    fn unrelated_or_ambiguous_routes_are_rejected_without_echoing_credentials() {
        for value in [
            "https://plugins/message/a/code",
            "streamdeck://plugins@host/message/a/code",
            "streamdeck://plugins:1/message/a/code",
            "streamdeck://plugins/install/a/code",
            "streamdeck://plugins/message//secret",
            "streamdeck://plugins/message/a%2Fb/secret",
            "streamdeck://plugins/message/a/secret\n",
        ] {
            let error = DeepLink::parse(value).err().unwrap().to_string();
            assert!(!error.contains("secret"));
        }
        assert!(DeepLink::parse(&"x".repeat(MAX_BYTES + 1)).is_err());
    }
}
