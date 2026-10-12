//! Dial Stack routing. Presses belong to the container; touch/turn belong to its selected child.
use super::*;
use crate::dial_stack::{self, Rotation};
use std::time::Instant;

pub(super) fn action() -> Action {
    Action {
        uuid: dial_stack::UUID.into(),
        name: "Dial Stack".into(),
        plugin: "deckforge.dials".into(),
        controllers: vec!["Encoder".into()],
        raw: json!({"SupportedInMultiActions":false,"Encoder":{"layout":"$X1"},"States":[{"Title":"Dial Stack"}]}),
    }
}
pub(super) fn defaults() -> Value {
    json!({"CurrentIdx":0,"FastNavigation":false,"ShowStackIcon":true})
}
fn model(slot: &Slot) -> Result<dial_stack::Stack> {
    let mut raw = slot.raw.clone();
    raw["Settings"] = slot.settings.clone();
    dial_stack::parse(&raw)
}

impl Host {
    pub(super) fn stack_button(&mut self, slot: &Slot, pressed: bool) -> Result<()> {
        let mut stack = model(slot)?;
        let gesture = self.stack_gestures.entry(slot.context.clone()).or_default();
        if pressed {
            gesture.down(&stack);
            return Ok(());
        }
        let next = gesture.up(&mut stack);
        self.stack_gestures.remove(&slot.context);
        if stack.fast_navigation {
            self.stack_cooldown.insert(
                slot.context.clone(),
                Instant::now() + Duration::from_millis(300),
            );
        }
        if let Some(index) = next {
            self.select_stack(&slot.context, index, false)?;
        }
        Ok(())
    }
    pub(super) fn stack_input(&mut self, slot: &Slot, name: &str, payload: Value) -> Result<()> {
        let stack = model(slot)?;
        if stack.current().is_none() {
            return Ok(());
        }
        let selected = stack.current;
        let index = if name == "dialRotate" {
            let ticks = i8::try_from(payload["ticks"].as_i64().context("Missing dial ticks")?)?;
            let pressed = payload["pressed"].as_bool().unwrap_or(false);
            let owns_press = self.held.get(&HeldControl::Dial(slot.index)) == Some(&slot.context);
            if !(stack.fast_navigation && pressed && owns_press)
                && self
                    .stack_cooldown
                    .get(&slot.context)
                    .is_some_and(|until| Instant::now() < *until)
            {
                return Ok(());
            }
            match self
                .stack_gestures
                .entry(slot.context.clone())
                .or_default()
                .rotate(&stack, ticks, pressed && owns_press)
            {
                Rotation::Consumed => return Ok(()),
                Rotation::Forward { index, .. } => index,
            }
        } else {
            selected
        };
        let child = slot
            .children
            .first()
            .and_then(|c| c.get(index))
            .cloned()
            .context("Stack child missing")?;
        let action = self.actions.iter().find(|a| a.uuid == child.uuid).context(
            "Selected stack action is unavailable; install its plugin or choose another action",
        )?;
        if !action.controllers.iter().any(|c| c == "Encoder") {
            bail!("Selected action does not support dials");
        }
        if child.uuid == crate::action_trigger::UUID {
            return self.trigger_input(&child, name, payload);
        }
        if mixer_actions::supported(&child.uuid) || crate::system_actions::supported(&child.uuid) {
            let trigger = if name == "dialRotate" {
                crate::system_actions::Trigger::Rotate(payload["ticks"].as_i64().unwrap_or(0) as i8)
            } else {
                crate::system_actions::Trigger::Tap
            };
            return self.run_system_action(&child, trigger);
        }
        if !self
            .plugins
            .iter()
            .any(|p| p.id == action.plugin && p.sender.is_some())
        {
            bail!("Selected stack plugin is disconnected");
        }
        self.emit(&child, name, payload);
        Ok(())
    }
    pub(super) fn select_stack(
        &mut self,
        context: &str,
        index: usize,
        reset_gesture: bool,
    ) -> Result<()> {
        let mut parent = self
            .visible_slot(context)
            .cloned()
            .context("Stack is no longer visible")?;
        if parent.uuid != dial_stack::UUID {
            bail!("This control is not a Dial Stack");
        }
        if parent
            .children
            .first()
            .is_none_or(|children| index >= children.len())
        {
            bail!("Stack selection is out of range");
        }
        if reset_gesture {
            self.stack_gestures.remove(context);
            self.stack_cooldown.remove(context);
        }
        if parent.active_stack_index() == index && parent.settings["CurrentIdx"] == index {
            return Ok(());
        }
        self.sync_visible();
        parent.settings["CurrentIdx"] = json!(index);
        parent.raw["Settings"] = parent.settings.clone();
        self.library.update(&parent);
        self.refresh_page()
    }
    pub(super) fn save_stack(&mut self, context: &str, raw: Value) -> Result<()> {
        self.editable()?;
        let mut parent = self
            .visible_slot(context)
            .cloned()
            .context("Stack is no longer visible")?;
        if parent.uuid != dial_stack::UUID {
            bail!("This control is not a Dial Stack");
        }
        dial_stack::parse(&raw)?;
        let old = parent.children.clone();
        parent.raw = raw;
        parent.settings = parent
            .raw
            .get("Settings")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or(json!({}));
        parent.hydrate_children(&self.actions)?;
        // Imported unavailable actions stay retained. Newly inserted known actions
        // still need encoder support; an invalid save cannot disturb live children.
        for child in parent.children.iter().flatten() {
            if self
                .actions
                .iter()
                .find(|a| a.uuid == child.uuid)
                .is_some_and(|a| !a.controllers.iter().any(|c| c == "Encoder"))
            {
                bail!("{} does not support dials", child.name);
            }
        }
        self.sync_visible();
        self.stack_gestures.remove(context);
        self.stack_cooldown.remove(context);
        self.library.update(&parent);
        self.refresh_page()?;
        for child in parent.children.iter().flatten() {
            if old
                .iter()
                .flatten()
                .any(|old| old.context == child.context && old.settings != child.settings)
                && self.visible_slot(&child.context).is_some()
            {
                self.emit(child, "didReceiveSettings", json!({}));
                self.notify_inspectors(&child.context, self.settings_event(child, None), None);
            }
        }
        Ok(())
    }
    pub(super) fn cancel_departing_stacks(&mut self, visible: &HashSet<String>) {
        self.stack_gestures.retain(|context, gesture| {
            if !visible.contains(context) {
                gesture.reset();
                false
            } else {
                true
            }
        });
        self.stack_cooldown
            .retain(|context, _| visible.contains(context));
    }
}

/// Independent simple stacked-window mark in the original overlay rectangle.
/// No proprietary SVG is included. Pixel-perfect vendor artwork remains unverified.
pub(super) fn mark(image: &mut DynamicImage) {
    for (x, y) in [(171, 9), (166, 14), (161, 19)] {
        for row in y..y + 14 {
            for col in x..x + 14 {
                image.put_pixel(
                    col,
                    row,
                    if row == y || row == y + 13 || col == x || col == x + 13 {
                        image::Rgba([245, 245, 245, 255])
                    } else {
                        image::Rgba([20, 25, 31, 255])
                    },
                );
            }
        }
    }
}
