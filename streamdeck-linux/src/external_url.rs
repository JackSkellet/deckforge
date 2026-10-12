//! Default-browser requests shared by SDK backends, inspectors and the editor.
use anyhow::{Context, Result, ensure};
use std::{
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn validate(value: &str) -> Result<String> {
    ensure!(value.len() <= 8192, "Link URL is too long");
    ensure!(
        !value.chars().any(char::is_control),
        "Link URL contains control characters"
    );
    let url = url::Url::parse(value).context("Invalid link URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "Only HTTP and HTTPS links can open externally"
    );
    Ok(url.into())
}

/// Acceptance is asynchronous, as in SDK openUrl. Reap launchers without holding
/// the host mutex or killing the user's browser when Deckforge exits.
pub fn open(value: &str) -> Result<()> {
    let url = validate(value)?;
    ACTIVE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 8).then_some(n + 1)
        })
        .map_err(|_| anyhow::anyhow!("Too many browser requests are still running"))?;
    let permit = Permit;
    std::thread::Builder::new()
        .name("deckforge-browser".into())
        .spawn(move || {
            let _permit = permit;
            match Command::new("xdg-open")
                .arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    if !child.wait().is_ok_and(|status| status.success()) {
                        eprintln!("Default-browser launcher did not complete successfully");
                    }
                }
                Err(_) => eprintln!("Unable to start the default-browser launcher"),
            }
        })
        .context("Start browser request")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_urls_are_validated_without_launching_handlers() {
        assert_eq!(
            validate("https://example.invalid/cb?x=%252F&x=a+b").unwrap(),
            "https://example.invalid/cb?x=%252F&x=a+b"
        );
        for url in [
            "javascript:alert(1)",
            "file:///tmp/private",
            "data:text/plain,x",
            "streamdeck://plugins/message/x/a",
            "https://example.invalid/\nsecret",
        ] {
            let error = validate(url).unwrap_err().to_string();
            assert!(!error.contains("secret"));
        }
    }
}
