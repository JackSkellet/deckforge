//! Per-action property inspector sessions and their restricted SDK message routing.
use super::*;

pub(super) struct Inspector {
    pub context: String,
    pub plugin: usize,
    pub child: Option<Child>,
    pub sender: Option<mpsc::UnboundedSender<Value>>,
}
impl Drop for Inspector {
    fn drop(&mut self) {
        stop_child(&mut self.child);
    }
}

impl Host {
    /// Reap failed helpers even when they never reached WebSocket registration.
    pub(super) fn monitor_children(&mut self) {
        self.expire_deep_links();
        for device in self.device_ids() {
            if let Err(error) = self.with_device(&device, |host| {
                host.monitor_inspectors();
                Ok(())
            }) {
                self.log(format!("Inspector monitor for {device}: {error:#}"));
            }
        }
        self.monitor_plugins();
    }
    fn monitor_inspectors(&mut self) {
        let mut exited = Vec::new();
        for (registration, inspector) in &mut self.inspectors {
            if let Some(child) = &mut inspector.child
                && let Ok(Some(status)) = child.try_wait()
            {
                exited.push((registration.clone(), status));
            }
        }
        for (registration, status) in exited {
            if !status.success() {
                self.log(format!(
                    "Property inspector failed ({status}): {}",
                    self.child_log(&format!("webview-{registration}.log"))
                ));
            }
            self.close_inspector(&registration);
        }
    }
    pub(super) fn child_log(&self, name: &str) -> String {
        use std::io::{Read, Seek, SeekFrom};
        let Some(directory) = self.state_path.parent() else {
            return String::new();
        };
        let Ok(mut file) = File::open(directory.join(name)) else {
            return String::new();
        };
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = file.seek(SeekFrom::Start(size.saturating_sub(4096)));
        let mut bytes = Vec::new();
        let _ = file.take(4096).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).trim().to_owned()
    }
    pub(super) fn open_inspector(&mut self, context: &str) -> Result<()> {
        if let Some(device) = self
            .context_device(context)
            .filter(|device| device != &self.device)
        {
            return self.with_device(&device, |host| host.open_inspector(context));
        }
        let slot = self
            .visible_slot(context)
            .context("Action no longer exists")?;
        let action = self
            .actions
            .iter()
            .find(|a| a.uuid == slot.uuid)
            .context("Action plugin is not installed")?;
        let plugin_index = self
            .plugins
            .iter()
            .position(|p| p.id == action.plugin)
            .context("Plugin missing")?;
        let plugin = &self.plugins[plugin_index];
        anyhow::ensure!(
            self.plugin_enabled(plugin),
            "Enable this plugin before opening its settings"
        );
        let path = action.raw["PropertyInspectorPath"]
            .as_str()
            .or_else(|| plugin.manifest["PropertyInspectorPath"].as_str())
            .context("This plugin has no property inspector")?;
        let html = safe_file(&plugin.root, path)?;
        let info = registration_info(plugin, &self.registration_devices(), self.language);
        let action_info = json!({"action":slot.uuid,"context":slot.context,"device":self.device,"payload":action_payload(slot)});
        let registration = uuid::Uuid::new_v4().to_string();
        let directory = self
            .state_path
            .parent()
            .context("Missing state directory")?;
        let child = spawn_webview(
            &html,
            &plugin.root,
            self.port,
            &registration,
            "registerPropertyInspector",
            &info,
            Some(&action_info),
            true,
            directory,
        )?;
        self.close_inspectors(Some(context));
        self.inspectors.insert(
            registration,
            Inspector {
                context: context.to_owned(),
                plugin: plugin_index,
                child: Some(child),
                sender: None,
            },
        );
        self.log(format!("Opening property inspector for {context}"));
        Ok(())
    }

    pub(super) fn close_inspectors(&mut self, context: Option<&str>) {
        let registrations: Vec<_> = self
            .inspectors
            .iter()
            .filter(|(_, i)| context.is_none_or(|c| i.context == c))
            .map(|(id, _)| id.clone())
            .collect();
        for registration in registrations {
            self.close_inspector(&registration);
        }
    }

    pub(super) fn close_inspector(&mut self, registration: &str) {
        if let Some(device) = self
            .inspector_device(registration)
            .filter(|device| device != &self.device)
        {
            if let Err(error) = self.with_device(&device, |host| {
                host.close_inspector(registration);
                Ok(())
            }) {
                self.log(format!("Close inspector for {device}: {error:#}"));
            }
            return;
        }
        if let Some(inspector) = self.inspectors.remove(registration) {
            if inspector.sender.is_some()
                && let Some(slot) = self.visible_slot(&inspector.context)
            {
                self.emit(slot, "propertyInspectorDidDisappear", json!({}));
            }
            self.log(format!(
                "Property inspector closed for {}",
                inspector.context
            ));
            drop(inspector);
        }
    }

    pub(super) fn settings_event(&self, slot: &Slot, id: Option<&Value>) -> Value {
        let device = self
            .context_device(&slot.context)
            .unwrap_or_else(|| self.device.clone());
        let mut event = json!({"event":"didReceiveSettings","action":slot.uuid,"context":slot.context,"device":device,"payload":action_payload(slot)});
        if let Some(id) = id {
            event["id"] = id.clone();
        }
        event
    }
    pub(super) fn notify_inspectors(&mut self, context: &str, event: Value, except: Option<&str>) {
        if let Some(device) = self
            .context_device(context)
            .filter(|device| device != &self.device)
        {
            if let Err(error) = self.with_device(&device, |host| {
                host.notify_inspectors(context, event, except);
                Ok(())
            }) {
                self.log(format!("Inspector notification for {device}: {error:#}"));
            }
            return;
        }
        for (registration, inspector) in &self.inspectors {
            if inspector.context == context
                && except != Some(registration)
                && let Some(sender) = &inspector.sender
            {
                let _ = sender.send(event.clone());
            }
        }
    }
    pub(super) fn notify_global_settings(
        &mut self,
        plugin: usize,
        except: Option<&str>,
        notify_plugin: bool,
    ) {
        let owner = &self.plugins[plugin];
        let event = json!({"event":"didReceiveGlobalSettings","payload":{"settings":self.globals.get(&owner.id).cloned().unwrap_or(json!({}))}});
        if notify_plugin && let Some(sender) = &owner.sender {
            let _ = sender.send(event.clone());
        }
        for device in self.device_ids() {
            if let Err(error) = self.with_device(&device, |host| {
                for (registration, inspector) in &host.inspectors {
                    if inspector.plugin == plugin
                        && except != Some(registration.as_str())
                        && let Some(sender) = &inspector.sender
                    {
                        let _ = sender.send(event.clone());
                    }
                }
                Ok(())
            }) {
                self.log(format!(
                    "Global settings notification for {device}: {error:#}"
                ));
            }
        }
    }

    pub(super) fn inspector_message(&mut self, registration: &str, message: Value) -> Result<()> {
        if let Some(device) = self
            .inspector_device(registration)
            .filter(|device| device != &self.device)
        {
            return self.with_device(&device, |host| {
                host.inspector_message(registration, message)
            });
        }
        let inspector = self
            .inspectors
            .get(registration)
            .context("Inspector session expired")?;
        let context = inspector.context.clone();
        let plugin = inspector.plugin;
        let sender = inspector
            .sender
            .clone()
            .context("Inspector not registered")?;
        let mut slot = self
            .visible_slot(&context)
            .cloned()
            .context("Inspector action no longer exists")?;
        let event = message["event"].as_str().context("Missing event")?;
        if let Some(supplied) = message.get("context") {
            let supplied = supplied.as_str().context("Invalid context")?;
            if supplied != registration && supplied != context {
                bail!("Inspector cannot access another action");
            }
        }
        if let Some(target) = message.get("targetContext")
            && target.as_str() != Some(&context)
        {
            bail!("Inspector targetContext is outside its action");
        }
        if let Some(action) = message.get("action")
            && action.as_str() != Some(&slot.uuid)
        {
            bail!("Inspector action UUID does not match its session");
        }
        if message
            .get("device")
            .is_some_and(|device| device.as_str() != Some(self.device.as_str()))
        {
            bail!("Inspector device does not match its bound action");
        }
        let payload = message.get("payload").cloned().unwrap_or(json!({}));
        match event {
            "openUrl" => {
                crate::external_url::open(payload["url"].as_str().context("Missing link URL")?)?;
            }
            "getResources" => {
                let _ = sender.send(self.resources_event(&slot, message.get("id")));
            }
            "setResources" => {
                self.editable_context(&context)?;
                let changed = self.set_resources(&slot, payload)?;
                self.emit(&changed, "didReceiveResources", json!({}));
            }
            "getSettings" => {
                let _ = sender.send(self.settings_event(&slot, message.get("id")));
            }
            "setSettings" => {
                self.editable_context(&context)?;
                if !payload.is_object() {
                    bail!("Settings must be an object");
                }
                slot.settings = payload;
                self.store_slot(slot.clone());
                self.persist()?;
                self.emit(&slot, "didReceiveSettings", json!({}));
                self.notify_inspectors(
                    &context,
                    self.settings_event(&slot, None),
                    Some(registration),
                );
            }
            "getGlobalSettings" => {
                let mut reply = json!({"event":"didReceiveGlobalSettings","payload":{"settings":self.globals.get(&self.plugins[plugin].id).cloned().unwrap_or(json!({}))}});
                if let Some(id) = message.get("id") {
                    reply["id"] = id.clone();
                }
                let _ = sender.send(reply);
            }
            "setGlobalSettings" => {
                if !payload.is_object() {
                    bail!("Global settings must be an object");
                }
                self.globals
                    .insert(self.plugins[plugin].id.clone(), payload);
                self.persist()?;
                self.notify_global_settings(plugin, Some(registration), true);
            }
            "sendToPlugin" => {
                let slot = &slot;
                if let Some(sender) = &self.plugins[plugin].sender {
                    let _=sender.send(json!({"event":"sendToPlugin","action":slot.uuid,"context":slot.context,"device":self.device,"payload":payload}));
                }
            }
            "logMessage" => self.log(format!(
                "Inspector {}: {}",
                self.plugins[plugin].id,
                payload["message"].as_str().unwrap_or("")
            )),
            _ => bail!("Unsupported property inspector command: {event}"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod multi_device_tests {
    use super::*;
    fn pair() -> (
        tempfile::TempDir,
        Host,
        String,
        String,
        mpsc::UnboundedReceiver<Value>,
        mpsc::UnboundedReceiver<Value>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.slots[0].index = 5;
        host.sync_visible();
        let a = host.slots[0].context.clone();
        let (ta, ra) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "pi-a".into(),
            Inspector {
                context: a.clone(),
                plugin: 0,
                child: None,
                sender: Some(ta),
            },
        );
        host.add_simulated_device("device-b", Model::Mini).unwrap();
        let (tb, rb) = mpsc::unbounded_channel();
        let b = host
            .with_device("device-b", |h| {
                h.assign("Keypad", 0, "test.action")?;
                let b = h.slots[0].context.clone();
                h.inspectors.insert(
                    "pi-b".into(),
                    Inspector {
                        context: b.clone(),
                        plugin: 0,
                        child: None,
                        sender: Some(tb),
                    },
                );
                Ok(b)
            })
            .unwrap();
        host.select_device("device-b").unwrap();
        (temp, host, a, b, ra, rb)
    }
    #[test]
    fn inspector_edits_and_readonly_checks_use_owner_not_selected_device() {
        let (_temp, mut host, a, b, mut ra, mut rb) = pair();
        host.library.current_mut().unwrap().readonly = true;
        host.inspector_message(
            "pi-a",
            json!({"event":"setSettings","context":"pi-a","payload":{"owner":"a"}}),
        )
        .unwrap();
        assert_eq!(host.device, "device-b");
        assert_eq!(
            host.library.slot(&a).unwrap().settings,
            json!({"owner":"a"})
        );
        assert_eq!(host.library.slot(&b).unwrap().settings, json!({}));
        host.inspector_message(
            "pi-a",
            json!({"event":"getSettings","context":"pi-a","id":"request-a"}),
        )
        .unwrap();
        let response = ra.try_recv().unwrap();
        assert_eq!(response["device"], "test-device");
        assert_eq!(response["context"], a);
        assert_eq!(response["id"], "request-a");
        assert_eq!(
            response["payload"]["coordinates"],
            json!({"column":1,"row":1})
        );
        assert!(rb.try_recv().is_err());
        host.library.current_mut().unwrap().readonly = false;
        host.with_device("test-device", |h| {
            h.library.current_mut()?.readonly = true;
            Ok(())
        })
        .unwrap();
        assert!(
            host.inspector_message(
                "pi-a",
                json!({"event":"setSettings","payload":{"bad":true}})
            )
            .is_err()
        );
        assert_eq!(host.device, "device-b");
        assert_eq!(
            host.library.slot(&a).unwrap().settings,
            json!({"owner":"a"})
        );
        assert!(
            host.inspector_message(
                "pi-a",
                json!({"event":"setSettings","context":b,"payload":{}})
            )
            .is_err()
        );
    }
    #[test]
    fn plugin_global_settings_fan_out_once_across_device_inspectors() {
        let (_temp, mut host, _a, _b, mut ra, mut rb) = pair();
        let (sender, mut plugin) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.inspector_message(
            "pi-a",
            json!({"event":"setGlobalSettings","context":"pi-a","payload":{"shared":31}}),
        )
        .unwrap();
        assert_eq!(host.device, "device-b");
        assert_eq!(host.globals["test.plugin"], json!({"shared":31}));
        assert!(ra.try_recv().is_err());
        let message = rb.try_recv().unwrap();
        assert_eq!(message["event"], "didReceiveGlobalSettings");
        assert_eq!(message["payload"]["settings"]["shared"], 31);
        assert!(message.get("device").is_none());
        assert!(rb.try_recv().is_err());
        assert_eq!(
            plugin.try_recv().unwrap()["event"],
            "didReceiveGlobalSettings"
        );
        assert!(plugin.try_recv().is_err());
    }
    #[test]
    fn parked_inspector_close_emits_original_device_lifecycle() {
        let (_temp, mut host, a, _b, _ra, _rb) = pair();
        let (sender, mut plugin) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(sender);
        host.close_inspector("pi-a");
        assert_eq!(host.device, "device-b");
        assert!(host.inspector_device("pi-a").is_none());
        assert_eq!(host.inspector_device("pi-b").as_deref(), Some("device-b"));
        let event = plugin.try_recv().unwrap();
        assert_eq!(event["event"], "propertyInspectorDidDisappear");
        assert_eq!(event["device"], "test-device");
        assert_eq!(event["context"], a);
    }
}
