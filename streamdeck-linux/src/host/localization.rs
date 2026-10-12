//! Presentation-only plugin localization. Saved actions and source manifests
//! retain their original text; the active language is fixed for this host run.
use super::*;

pub(super) fn default_preference() -> String {
    "system".into()
}

impl Host {
    pub(super) fn refresh_localizations(&mut self) {
        self.localizations = self
            .plugins
            .iter()
            .filter(|p| !p.runtime.load_failed)
            .map(|plugin| {
                (
                    plugin.id.clone(),
                    crate::plugin_localization::load(&plugin.root, &plugin.manifest, self.language),
                )
            })
            .collect();
    }
    pub(super) fn presented_manifest<'a>(&'a self, plugin: &'a Plugin) -> &'a Value {
        self.localizations
            .get(&plugin.id)
            .map(|p| &p.manifest)
            .unwrap_or(&plugin.manifest)
    }
    pub(super) fn presented_action<'a>(&'a self, action: &'a Action) -> &'a Value {
        self.localizations
            .get(&action.plugin)
            .and_then(|p| p.manifest["Actions"].as_array())
            .and_then(|actions| actions.iter().find(|a| a["UUID"] == action.uuid))
            .unwrap_or(&action.raw)
    }
    pub(super) fn presented_slot_name<'a>(&'a self, slot: &'a Slot) -> &'a str {
        let Some(action) = self.actions.iter().find(|a| a.uuid == slot.uuid) else {
            return &slot.name;
        };
        // Imported or user-supplied labels are retained. Only a name equal to
        // the original declaration is treated as the default action label.
        if slot.name == action.name {
            self.presented_action(action)["Name"]
                .as_str()
                .unwrap_or(&slot.name)
        } else {
            &slot.name
        }
    }
    pub(super) fn state_names(&self, slot: &Slot) -> Vec<String> {
        let action = self.actions.iter().find(|a| a.uuid == slot.uuid);
        (0..appearance::count(slot, &self.actions))
            .map(|index| {
                let saved = slot.states[index]["Name"].as_str();
                let original = action.and_then(|a| a.raw["States"][index]["Name"].as_str());
                if saved.is_some() && saved != original {
                    saved.unwrap_or("").to_owned()
                } else {
                    action
                        .and_then(|a| self.presented_action(a)["States"][index]["Name"].as_str())
                        .or(saved)
                        .unwrap_or("")
                        .to_owned()
                }
            })
            .collect()
    }
    pub(super) fn language_snapshot(&self) -> Value {
        let resolved = crate::plugin_localization::resolve(&self.plugin_language).unwrap_or("en");
        let options = std::iter::once(json!({"value":"system","label":"System language"}))
            .chain(
                crate::plugin_localization::LANGUAGES
                    .iter()
                    .map(|(value, label)| json!({"value":value,"label":label})),
            )
            .collect::<Vec<_>>();
        json!({"preference":self.plugin_language,"active":self.language,"resolved":resolved,
            "restart_required":resolved != self.language,"options":options})
    }
    pub(super) fn save_plugin_language(&mut self, request: &Value) -> Result<()> {
        let value = request["value"]
            .as_str()
            .context("Plugin language missing")?;
        crate::plugin_localization::resolve(value)?;
        anyhow::ensure!(
            request["baseline"].as_str() == Some(self.plugin_language.as_str()),
            "Plugin language changed; reload the current setting before saving"
        );
        let previous = std::mem::replace(&mut self.plugin_language, value.into());
        if let Err(error) = self.persist() {
            self.plugin_language = previous;
            return Err(error);
        }
        // No invented SDK language-change event. New plugins and inspectors
        // keep the same active language until a complete host restart.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localized_presentation_preserves_saved_action_and_dynamic_values() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.language = "de";
        host.plugins[0].manifest = json!({"Name":"Example","Category":"Original category","Actions":[{
            "UUID":"test.action","Name":"Action","States":[{"Name":"Off","Title":"User title"}],
            "Encoder":{"TriggerDescription":{"Rotate":"Turn"}}
        }]});
        host.actions[0].raw = host.plugins[0].manifest["Actions"][0].clone();
        host.actions[0].name = "Action".into();
        host.slots[0].name = "Action".into();
        host.slots[0].controller = "Encoder".into();
        fs::write(
            root.path().join("de.json"),
            serde_json::to_vec(&json!({"Name":"Beispiel",
            "test.action":{"Name":"Aktion","States":[{"Name":"Aus","Title":"Must not translate"}],
            "Encoder":{"TriggerDescription":{"Rotate":"Drehen"}}}}))
            .unwrap(),
        )
        .unwrap();
        host.plugins[0].root = root.path().into();
        let raw = serde_json::to_value(&host.slots).unwrap();
        host.refresh_localizations();
        assert_eq!(host.presented_slot_name(&host.slots[0]), "Aktion");
        assert_eq!(host.state_names(&host.slots[0])[0], "Aus");
        assert_eq!(host.trigger_description(&host.slots[0])["rotate"], "Drehen");
        assert_eq!(
            host.presented_action(&host.actions[0])["States"][0]["Title"],
            "User title"
        );
        assert_eq!(serde_json::to_value(&host.slots).unwrap(), raw);
        host.trigger_descriptions.insert(
            host.slots[0].context.clone(),
            json!({"rotate":"Live plugin text"}),
        );
        assert_eq!(
            host.trigger_description(&host.slots[0])["rotate"],
            "Live plugin text"
        );
        host.slots[0].name = "My custom label".into();
        assert_eq!(host.presented_slot_name(&host.slots[0]), "My custom label");
        host.slots[0].states = json!([{"Name":"My custom state"}]);
        assert_eq!(host.state_names(&host.slots[0])[0], "My custom state");
    }

    #[test]
    fn preference_requires_restart_rejects_stale_updates_and_rolls_back_failed_save() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.save_plugin_language(&json!({"value":"de","baseline":"system"}))
            .unwrap();
        assert_eq!(host.language, "en");
        assert_eq!(host.language_snapshot()["restart_required"], true);
        assert_eq!(
            registration_info(&host.plugins[0], &[], host.language)["application"]["language"],
            "en"
        );
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(saved.plugin_language, "de");
        assert!(
            host.save_plugin_language(&json!({"value":"fr","baseline":"system"}))
                .is_err()
        );
        assert!(
            host.save_plugin_language(&json!({"value":"../../outside","baseline":"de"}))
                .is_err()
        );
        host.state_path = root.path().join("absent/session.json");
        assert!(
            host.save_plugin_language(&json!({"value":"fr","baseline":"de"}))
                .is_err()
        );
        assert_eq!(host.plugin_language, "de");
    }
}
