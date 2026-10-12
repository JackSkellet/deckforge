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
        let mut failures = Vec::new();
        for plugin in &mut self.plugins {
            if let Some(child) = &mut plugin.child
                && let Ok(Some(status)) = child.try_wait()
            {
                plugin.child = None;
                let html = plugin.manifest["CodePath"]
                    .as_str()
                    .is_some_and(|p| p.ends_with(".html") || p.ends_with(".htm"));
                failures.push((
                    plugin.id.clone(),
                    status,
                    format!(
                        "{}-{}.log",
                        if html { "webview" } else { "plugin" },
                        plugin.registration
                    ),
                ));
            }
        }
        for (id, status, path) in failures {
            self.log(format!(
                "Plugin {id} exited ({status}): {}",
                self.child_log(&path)
            ));
        }
    }
    fn child_log(&self, name: &str) -> String {
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
        let path = action.raw["PropertyInspectorPath"]
            .as_str()
            .or_else(|| plugin.manifest["PropertyInspectorPath"].as_str())
            .context("This plugin has no property inspector")?;
        let html = safe_file(&plugin.root, path)?;
        let info = registration_info(plugin, &self.device);
        let action_info = json!({"action":slot.uuid,"context":slot.context,"device":self.device,"payload":action_payload(slot)});
        let registration = uuid::Uuid::new_v4().to_string();
        let directory = self
            .state_path
            .parent()
            .context("Missing state directory")?;
        let child = spawn_webview(
            &html,
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
        let mut event = json!({"event":"didReceiveSettings","action":slot.uuid,"context":slot.context,"device":self.device,"payload":action_payload(slot)});
        if let Some(id) = id {
            event["id"] = id.clone();
        }
        event
    }
    pub(super) fn notify_inspectors(&self, context: &str, event: Value, except: Option<&str>) {
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
        &self,
        plugin: usize,
        except: Option<&str>,
        notify_plugin: bool,
    ) {
        let owner = &self.plugins[plugin];
        let event = json!({"event":"didReceiveGlobalSettings","payload":{"settings":self.globals.get(&owner.id).cloned().unwrap_or(json!({}))}});
        if notify_plugin && let Some(sender) = &owner.sender {
            let _ = sender.send(event.clone());
        }
        for (registration, inspector) in &self.inspectors {
            if inspector.plugin == plugin
                && except != Some(registration)
                && let Some(sender) = &inspector.sender
            {
                let _ = sender.send(event.clone());
            }
        }
    }

    pub(super) fn inspector_message(&mut self, registration: &str, message: Value) -> Result<()> {
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
        let payload = message.get("payload").cloned().unwrap_or(json!({}));
        match event {
            "getSettings" => {
                let _ = sender.send(self.settings_event(&slot, message.get("id")));
            }
            "setSettings" => {
                self.editable()?;
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
