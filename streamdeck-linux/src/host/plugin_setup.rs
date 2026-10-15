//! Plugin-owned setup inspectors use temporary action contexts, never deck controls.
use super::*;

impl Host {
    pub(super) fn replay_plugin_setup(&self, plugin: usize) {
        for inspector in self.inspectors.values().filter(|i| i.plugin == plugin) {
            if let Some(slot) = &inspector.setup {
                self.emit(slot, "willAppear", json!({}));
                if inspector.sender.is_some() {
                    self.emit(slot, "propertyInspectorDidAppear", json!({}));
                }
            }
        }
    }

    pub(super) fn plugin_setup_actions(&self, plugin: &Plugin) -> Vec<Value> {
        self.actions
            .iter()
            .filter(|a| a.plugin == plugin.id)
            .filter(|a| {
                a.raw["PropertyInspectorPath"]
                    .as_str()
                    .or_else(|| plugin.manifest["PropertyInspectorPath"].as_str())
                    .is_some()
            })
            .map(|a| json!({"uuid":a.uuid,"name":a.name}))
            .collect()
    }

    pub(super) fn open_plugin_setup(&mut self, request: &Value) -> Result<()> {
        let id = request["id"].as_str().context("Plugin id missing")?;
        let uuid = request["uuid"].as_str().context("Setup action missing")?;
        let plugin = self
            .plugins
            .iter()
            .find(|p| p.id == id)
            .context("Plugin not installed")?;
        anyhow::ensure!(
            self.plugin_enabled(plugin),
            "Enable this plugin before opening setup"
        );
        anyhow::ensure!(
            plugin.sender.is_some(),
            "Wait for the plugin to connect before opening setup"
        );
        let action = self
            .actions
            .iter()
            .find(|a| a.plugin == id && a.uuid == uuid)
            .context("Setup action does not belong to this plugin")?;
        // One setup session per plugin; ordinary action inspectors remain independent.
        let slot: Slot = serde_json::from_value(json!({
            "model":self.model,"context":uuid::Uuid::new_v4().to_string(),
            "controller":action.controllers.first().map(String::as_str).unwrap_or("Keypad"),
            "index":0,"uuid":uuid,"name":action.name,"settings":{},"title":"",
            "state":0,"feedback":{},"layout":"$B1","states":action.raw["States"]
        }))?;
        let existing: Vec<_> = std::iter::once(&self.inspectors)
            .chain(self.other_devices.values().map(|state| &state.inspectors))
            .flat_map(|inspectors| inspectors.iter())
            .filter(|(_, i)| i.setup.is_some() && self.plugins[i.plugin].id == id)
            .map(|(registration, _)| registration.clone())
            .collect();
        for registration in existing {
            self.close_inspector(&registration);
        }
        self.launch_inspector(slot, true)
    }

    /// Handle setup replies before any rendering, profile storage or hardware routing.
    pub(super) fn setup_plugin_message(&mut self, plugin: usize, message: &Value) -> Result<bool> {
        let context = message["context"].as_str().unwrap_or("");
        let Some((registration, inspector)) = self
            .inspectors
            .iter()
            .find(|(_, i)| i.context == context && i.setup.is_some())
        else {
            return Ok(false);
        };
        anyhow::ensure!(
            inspector.plugin == plugin,
            "Plugin does not own this setup context"
        );
        let registration = registration.clone();
        let mut slot = inspector.setup.clone().unwrap();
        let payload = message.get("payload").cloned().unwrap_or(json!({}));
        match message["event"].as_str().unwrap_or("") {
            "getSettings" => {
                if let Some(sender) = &self.plugins[plugin].sender {
                    let _ = sender.send(self.settings_event(&slot, message.get("id")));
                }
            }
            "setSettings" => {
                anyhow::ensure!(payload.is_object(), "Settings must be an object");
                slot.settings = payload;
                self.inspectors.get_mut(&registration).unwrap().setup = Some(slot.clone());
                self.notify_inspectors(context, self.settings_event(&slot, None), None);
            }
            "sendToPropertyInspector" => self.notify_inspectors(context,
                json!({"event":"sendToPropertyInspector","action":slot.uuid,"context":context,"device":self.device,"payload":payload}), None),
            "getResources" => {
                if let Some(sender) = &self.plugins[plugin].sender {
                    let _ = sender.send(self.resources_event(&slot, message.get("id")));
                }
            }
            // Setup has no physical control to paint or flash.
            "setTitle" | "setImage" | "setState" | "setFeedback" | "setFeedbackLayout"
            | "setTriggerDescription" | "showOk" | "showAlert" => {},
            _ => bail!("This command requires an assigned action"),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_is_ephemeral_routes_replies_and_saves_only_shared_settings() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let before = serde_json::to_value(&host.library).unwrap();
        let mut slot = host.slots[0].clone();
        slot.context = "setup-context".into();
        let (sender, mut messages) = mpsc::unbounded_channel();
        let (pi, mut replies) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.inspectors.insert(
            "setup-pi".into(),
            Inspector {
                setup: Some(slot),
                context: "setup-context".into(),
                plugin: 0,
                child: None,
                sender: Some(pi),
            },
        );
        host.replay_plugin_setup(0);
        assert_eq!(messages.try_recv().unwrap()["event"], "willAppear");
        assert_eq!(
            messages.try_recv().unwrap()["event"],
            "propertyInspectorDidAppear"
        );
        host.inspector_message(
            "setup-pi",
            json!({"event":"setSettings","payload":{"temporary":42}}),
        )
        .unwrap();
        assert_eq!(messages.try_recv().unwrap()["event"], "didReceiveSettings");
        host.plugin_message(0, json!({"event":"setTitle","context":"setup-context","payload":{"title":"must not draw"}})).unwrap();
        host.plugin_message(
            0,
            json!({"event":"setSettings","context":"setup-context","payload":{"temporary":43}}),
        )
        .unwrap();
        assert_eq!(
            replies.try_recv().unwrap()["payload"]["settings"]["temporary"],
            43
        );
        host.plugin_message(0,json!({"event":"sendToPropertyInspector","context":"setup-context","payload":{"ready":true}})).unwrap();
        assert_eq!(replies.try_recv().unwrap()["payload"]["ready"], true);
        assert!(
            host.inspector_message(
                "setup-pi",
                json!({"event":"setSettings","context":host.slots[0].context,"payload":{}})
            )
            .is_err()
        );
        host.inspector_message(
            "setup-pi",
            json!({"event":"setGlobalSettings","payload":{"shared":"saved"}}),
        )
        .unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.globals["test.plugin"]["shared"], "saved");
        assert_eq!(serde_json::to_value(&host.library).unwrap(), before);
        assert!(
            !fs::read_to_string(&host.state_path)
                .unwrap()
                .contains("setup-context")
        );
        host.close_inspector("setup-pi");
        let events: Vec<_> = std::iter::from_fn(|| messages.try_recv().ok())
            .map(|v| v["event"].as_str().unwrap().to_owned())
            .collect();
        assert!(events.ends_with(&[
            "propertyInspectorDidDisappear".into(),
            "willDisappear".into()
        ]));
        assert!(host.context_device("setup-context").is_none());
    }
    #[test]
    fn setup_rejects_wrong_plugin_and_follows_its_device() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let mut slot = host.slots[0].clone();
        slot.context = "setup-context".into();
        host.inspectors.insert(
            "setup-pi".into(),
            Inspector {
                setup: Some(slot),
                context: "setup-context".into(),
                plugin: 0,
                child: None,
                sender: None,
            },
        );
        assert!(
            host.setup_plugin_message(1, &json!({"event":"setImage","context":"setup-context"}))
                .is_err()
        );
        host.add_simulated_device("device-b", Model::Mini).unwrap();
        host.select_device("device-b").unwrap();
        assert_eq!(
            host.context_device("setup-context").as_deref(),
            Some("test-device")
        );
        host.plugin_message(
            0,
            json!({"event":"setSettings","context":"setup-context","payload":{"crossDevice":true}}),
        )
        .unwrap();
        host.with_device("test-device", |h| {
            assert_eq!(
                h.inspectors["setup-pi"].setup.as_ref().unwrap().settings["crossDevice"],
                true
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(host.device, "device-b");
    }
}
