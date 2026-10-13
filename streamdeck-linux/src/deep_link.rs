//! Plugin callback routing. Keep the encoded suffix intact for the SDK to parse.
use anyhow::{Context, Result, ensure};

pub const MAX_BYTES: usize = 8192;

#[derive(Clone)] // Deliberately no Debug: callbacks can contain authorization codes.
pub struct DeepLink {
    pub install: Option<uuid::Uuid>,
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
        if let Some(id) = path.strip_prefix("install/") {
            let id = uuid::Uuid::parse_str(id).context("Invalid Marketplace plugin identifier")?;
            ensure!(
                path == format!("install/{}", id.hyphenated()),
                "Invalid Marketplace install route"
            );
            return Ok(Self {
                install: Some(id),
                plugin: String::new(),
                message: String::new(),
                hidden: false,
            });
        }
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
            install: None,
            plugin: plugin.into(),
            message: path[end..].into(),
            hidden,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn marketplace_install_is_distinct_from_plugin_callbacks() {
        let id = "30bbf05a-ee28-45cd-b3a6-b072d3d56351";
        let link = super::DeepLink::parse(&format!("streamdeck://plugins/install/{id}")).unwrap();
        assert_eq!(link.install.unwrap().to_string(), id);
        assert!(!link.hidden);
        assert!(link.message.is_empty());
        for suffix in [
            "/extra",
            "?url=https://evil.invalid",
            "#fragment",
            "?streamdeck=hidden",
        ] {
            assert!(
                super::DeepLink::parse(&format!("streamdeck://plugins/install/{id}{suffix}"))
                    .is_err()
            );
        }
        assert!(super::DeepLink::parse("streamdeck://plugins/install/not-a-uuid").is_err());
    }
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
