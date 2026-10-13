//! Plugin-global callback delivery. Credentials stay in a bounded memory queue,
//! never in profiles, session persistence, snapshots or activity messages.
use super::*;
use crate::deep_link::DeepLink;
use std::time::Instant;

const QUEUE_LIMIT: usize = 64;
const PLUGIN_LIMIT: usize = 8;
const TTL: Duration = Duration::from_secs(300);
pub(super) struct Pending {
    plugin: String,
    message: String,
    expires: Instant,
}
impl Controller {
    pub fn deliver_deep_link(&self, uri: &str) -> Result<()> {
        let link = DeepLink::parse(uri)?;
        if let Some(variant) = link.install {
            return self.marketplace_request("install", Some(variant));
        }
        let mut host = self.shared.lock().unwrap();
        let result = host.receive_deep_link(link);
        if let Err(error) = &result {
            host.system_status = format!("Plugin callback: {error}");
            let message = host.system_status.clone();
            host.log(message);
        }
        if let Some(redraw) = &host.redraw {
            redraw();
        }
        result
    }
}
impl Host {
    pub(super) fn discard_plugin_links(&mut self, plugin_id: &str) {
        self.pending_links
            .retain(|pending| pending.plugin != plugin_id);
    }
    fn receive_deep_link(&mut self, link: DeepLink) -> Result<()> {
        anyhow::ensure!(self.running, "Deckforge is shutting down");
        // The original app ignores an empty suffix. A trailing slash is a message.
        if link.message.is_empty() {
            return Ok(());
        }
        self.expire_deep_links();
        let mut matches = self
            .plugins
            .iter()
            .filter(|p| p.id.eq_ignore_ascii_case(&link.plugin));
        let plugin = matches
            .next()
            .context("The target plugin is not installed")?;
        anyhow::ensure!(matches.next().is_none(), "Plugin identifier is ambiguous");
        anyhow::ensure!(
            self.plugin_enabled(plugin),
            "The target plugin is disabled; enable it before receiving callbacks"
        );
        let id = plugin.id.clone();
        if plugin.sender.as_ref().is_some_and(|sender| {
            sender
                .send(json!({"event":"didReceiveDeepLink","payload":{"url":link.message}}))
                .is_ok()
        }) {
            self.system_status = format!("Callback sent to {id}");
            return Ok(());
        }
        anyhow::ensure!(
            self.pending_links.len() < QUEUE_LIMIT
                && self.pending_links.iter().filter(|p| p.plugin == id).count() < PLUGIN_LIMIT,
            "Too many callbacks are waiting for the plugin to connect"
        );
        self.pending_links.push_back(Pending {
            plugin: id.clone(),
            message: link.message,
            expires: Instant::now() + TTL,
        });
        self.system_status = format!("Callback queued until {id} connects");
        Ok(())
    }
    pub(super) fn expire_deep_links(&mut self) {
        let count = self.pending_links.len();
        let now = Instant::now();
        self.pending_links.retain(|pending| pending.expires > now);
        if self.pending_links.len() < count {
            self.log("A plugin callback expired before its plugin connected");
        }
    }
    pub(super) fn flush_deep_links(&mut self, plugin_id: &str) {
        self.expire_deep_links();
        let Some(sender) = self
            .plugins
            .iter()
            .find(|p| p.id == plugin_id && self.plugin_enabled(p))
            .and_then(|p| p.sender.clone())
        else {
            return;
        };
        let mut sent = 0;
        for pending in std::mem::take(&mut self.pending_links) {
            if pending.plugin == plugin_id
                && sender
                    .send(json!({"event":"didReceiveDeepLink","payload":{"url":pending.message}}))
                    .is_ok()
            {
                sent += 1;
            } else {
                self.pending_links.push_back(pending);
            }
        }
        if sent > 0 {
            self.system_status = format!("{sent} callback(s) sent to {plugin_id}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disable_discards_waiting_callbacks_and_rejects_new_ones_without_enabling() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let id = host.plugins[0].id.clone();
        let callback = format!("streamdeck://plugins/message/{id}/auth?code=PRIVATE_PENDING");
        host.receive_deep_link(DeepLink::parse(&callback).unwrap())
            .unwrap();
        assert_eq!(host.pending_links.len(), 1);
        host.set_plugin_enabled(&json!({"id":id,"enabled":false,"expected_enabled":true}))
            .unwrap();
        assert!(host.pending_links.is_empty());
        assert!(
            host.receive_deep_link(DeepLink::parse(&callback).unwrap())
                .is_err()
        );
        assert!(host.pending_links.is_empty());
        assert!(!host.plugin_enabled(&host.plugins[0]));
        assert!(
            !fs::read_to_string(&host.state_path)
                .unwrap()
                .contains("PRIVATE_PENDING")
        );
    }
    #[test]
    fn callback_waits_for_its_plugin_once_and_never_enters_saved_state() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let id = host.plugins[0].id.clone();
        let suffix = "/auth?code=PRIVATE%2FVALUE&x=1&x=2&streamdeck=hidden#done";
        host.receive_deep_link(
            DeepLink::parse(&format!(
                "streamdeck://plugins/message/{}{suffix}",
                id.to_ascii_uppercase()
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(host.pending_links.len(), 1);
        host.persist().unwrap();
        assert!(
            !fs::read_to_string(&host.state_path)
                .unwrap()
                .contains("PRIVATE")
        );
        assert!(!snapshot(&host).to_string().contains("PRIVATE"));
        let (sender, mut receiver) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.flush_deep_links("another-plugin");
        assert!(receiver.try_recv().is_err());
        host.flush_deep_links(&id);
        assert_eq!(
            receiver.try_recv().unwrap(),
            json!({"event":"didReceiveDeepLink","payload":{"url":suffix}})
        );
        host.flush_deep_links(&id);
        assert!(receiver.try_recv().is_err());
        assert!(host.pending_links.is_empty());
    }
    #[test]
    fn callback_limits_expiry_empty_message_and_unknown_plugins_are_explicit() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let id = host.plugins[0].id.clone();
        let make = |suffix: &str| {
            DeepLink::parse(&format!("streamdeck://plugins/message/{id}{suffix}")).unwrap()
        };
        host.receive_deep_link(make("")).unwrap();
        assert!(host.pending_links.is_empty());
        for _ in 0..PLUGIN_LIMIT {
            host.receive_deep_link(make("/private")).unwrap();
        }
        assert!(host.receive_deep_link(make("/excess")).is_err());
        for item in &mut host.pending_links {
            item.expires = Instant::now() - Duration::from_secs(1);
        }
        host.expire_deep_links();
        assert!(host.pending_links.is_empty());
        assert!(!host.logs.join("\n").contains("private"));
        host.receive_deep_link(make("/")).unwrap();
        assert_eq!(host.pending_links.len(), 1);
        assert!(
            host.receive_deep_link(
                DeepLink::parse("streamdeck://plugins/message/not.installed/code").unwrap()
            )
            .is_err()
        );
    }
}
