//! One pending shared-action choice per native editor. Inspection is inert;
//! selection rechecks source contents and the captured destination on commit.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub controller: String,
    pub index: u8,
    pub expected_target: Option<String>,
    pub expected_profile: String,
    pub expected_page: String,
}
#[derive(Clone, Serialize)]
pub struct Pending {
    pub id: String,
    pub device: String,
    pub destination: Option<Destination>,
    pub info: crate::shared_action::Info,
}
#[derive(Default)]
pub struct Manager {
    pending: Option<Pending>,
}
impl Manager {
    pub fn snapshot(&self) -> Value {
        json!(self.pending)
    }
    pub fn prepare(
        &mut self,
        controller: &crate::host::Controller,
        path: &Path,
        device: String,
        destination: Option<Destination>,
    ) -> Result<String> {
        ensure!(
            self.pending.is_none(),
            "Finish or cancel the current action import before opening another action file"
        );
        let info = crate::shared_action::inspect(path)?;
        let pending = Pending {
            id: uuid::Uuid::new_v4().to_string(),
            device,
            destination,
            info,
        };
        if pending.info.roots.len() == 1 {
            Self::apply(controller, &pending, 0)
        } else {
            let message = format!(
                "Choose one of {} actions from this file.",
                pending.info.roots.len()
            );
            self.pending = Some(pending);
            Ok(message)
        }
    }
    fn apply(
        controller: &crate::host::Controller,
        pending: &Pending,
        root: usize,
    ) -> Result<String> {
        ensure!(
            root < pending.info.roots.len(),
            "Choose an action from this file"
        );
        let mut request = json!({"command":if pending.destination.is_some(){"slot_import"}else{"slot_file_clipboard"},"device":pending.device,
            "path":pending.info.path,"source_hash":pending.info.source_hash,"root":root});
        if let Some(destination) = &pending.destination {
            request.as_object_mut().unwrap().extend(
                serde_json::to_value(destination)?
                    .as_object()
                    .unwrap()
                    .clone(),
            );
        }
        controller.command(request)?;
        Ok(if pending.destination.is_some() {
            "Action imported."
        } else {
            "Action copied from file. Select a control and paste."
        }
        .into())
    }
    pub fn choose(
        &mut self,
        controller: &crate::host::Controller,
        id: &str,
        root: usize,
    ) -> Result<String> {
        let pending = self
            .pending
            .as_ref()
            .context("Action import is no longer pending")?;
        ensure!(
            pending.id == id,
            "Action import changed; reopen the chooser"
        );
        let message = Self::apply(controller, pending, root)?;
        self.pending = None;
        Ok(message)
    }
    pub fn cancel(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.pending.as_ref().is_some_and(|p| p.id == id),
            "Action import changed; reopen the chooser"
        );
        self.pending = None;
        Ok(())
    }
}
