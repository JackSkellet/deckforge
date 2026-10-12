//! Runtime gesture hints and temporary SDK feedback. Never exported as action settings.
use super::*;
use image::{GenericImageView, Pixel};

impl Host {
    pub(super) fn set_trigger_description(&mut self, slot: &Slot, payload: Value) -> Result<()> {
        if slot.controller != "Encoder" {
            bail!("Trigger descriptions apply to dial actions");
        }
        let object = payload
            .as_object()
            .context("Trigger descriptions must be an object")?;
        let mut description = serde_json::Map::new();
        for key in ["rotate", "push", "touch", "longTouch"] {
            if let Some(value) = object.get(key) {
                let text = value
                    .as_str()
                    .context("Trigger descriptions must be strings")?;
                if text.len() > 4096 {
                    bail!("Trigger description is too long");
                }
                description.insert(key.into(), json!(text));
            }
        }
        if description.is_empty() {
            self.trigger_descriptions.remove(&slot.context);
        } else {
            self.trigger_descriptions
                .insert(slot.context.clone(), Value::Object(description));
        }
        if let Some(redraw) = &self.redraw {
            redraw();
        }
        Ok(())
    }
    pub(super) fn trigger_description(&self, slot: &Slot) -> Value {
        if slot.controller != "Encoder" {
            return json!({});
        }
        if slot.uuid == crate::dial_stack::UUID {
            let mut hints = slot
                .children
                .first()
                .and_then(|g| g.get(slot.active_stack_index()))
                .map(|child| self.trigger_description(child))
                .unwrap_or(json!({}));
            hints["push"] = json!("Next action in stack");
            return hints;
        }
        if let Some(description) = self.trigger_descriptions.get(&slot.context) {
            return description.clone();
        }
        let mut result = serde_json::Map::new();
        if let Some(action) = self.actions.iter().find(|a| a.uuid == slot.uuid) {
            for (key, field) in [
                ("rotate", "Rotate"),
                ("push", "Push"),
                ("touch", "Touch"),
                ("longTouch", "LongTouch"),
            ] {
                if let Some(text) = action.raw["Encoder"]["TriggerDescription"][field].as_str() {
                    result.insert(key.into(), json!(text));
                }
            }
        }
        Value::Object(result)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Badge {
    kind: crate::feedback::Kind,
    started: Duration,
    expires: Duration,
    dial: bool,
    next_frame: Duration,
}
impl Host {
    pub(super) fn show_feedback(&mut self, slot: &Slot, kind: crate::feedback::Kind) -> Result<()> {
        let dial = slot.controller == "Encoder";
        if dial && kind == crate::feedback::Kind::Ok {
            bail!("Success feedback applies to key actions");
        }
        if self.visible_slot(&slot.context).is_none() {
            return Ok(());
        }
        let now = self.multi_clock.elapsed();
        let context = if dial {
            self.slots
                .iter()
                .find(|root| {
                    root.controller == "Encoder"
                        && root
                            .visible_flatten()
                            .iter()
                            .any(|child| child.context == slot.context)
                })
                .map(|root| root.context.clone())
                .unwrap_or_else(|| slot.context.clone())
        } else {
            slot.context.clone()
        };
        self.badges.insert(
            context,
            Badge {
                kind,
                started: now,
                expires: now
                    + if dial {
                        Duration::from_millis(330)
                    } else {
                        Duration::from_secs(3)
                    },
                dial,
                next_frame: now + Duration::from_millis(20),
            },
        );
        self.multi_notify.notify_one();
        if self.visible_slot(&slot.context).is_some() {
            self.render()?;
        }
        Ok(())
    }
    pub(super) fn poll_feedback(&mut self) {
        self.poll_feedback_at(self.multi_clock.elapsed());
    }
    fn poll_feedback_at(&mut self, now: Duration) {
        let redraw = self
            .badges
            .values()
            .any(|badge| badge.expires <= now || (badge.dial && badge.next_frame <= now));
        self.badges.retain(|_, badge| badge.expires > now);
        for badge in self
            .badges
            .values_mut()
            .filter(|b| b.dial && b.next_frame <= now)
        {
            badge.next_frame = (now + Duration::from_millis(20)).min(badge.expires);
        }
        if redraw && let Err(error) = self.render() {
            self.log(format!("Feedback redraw: {error:#}"));
        }
    }
    pub(super) fn next_feedback(&self) -> Option<Duration> {
        self.badges
            .values()
            .map(|b| {
                if b.dial {
                    b.next_frame.min(b.expires)
                } else {
                    b.expires
                }
            })
            .min()
    }
}
pub(super) fn paint_key(
    badges: &HashMap<String, Badge>,
    image: &mut DynamicImage,
    slot: &Slot,
    now: Duration,
) {
    if let Some(badge) = badges
        .get(&slot.context)
        .filter(|badge| badge.started <= now && now < badge.expires)
    {
        crate::feedback::paint(image, badge.kind);
    }
}

pub(super) fn paint_dial(
    badges: &HashMap<String, Badge>,
    image: &mut DynamicImage,
    slot: &Slot,
    now: Duration,
) {
    if let Some(badge) = badges
        .get(&slot.context)
        .filter(|b| b.dial && now < b.expires)
    {
        let alpha = (dial_opacity(now.saturating_sub(badge.started)) * 255.0).round() as u8;
        for y in 0..image.height() {
            for x in 0..image.width() {
                let mut pixel = image.get_pixel(x, y);
                pixel.blend(&image::Rgba([255, 0, 0, alpha]));
                image.put_pixel(x, y, pixel);
            }
        }
    }
}

fn dial_opacity(elapsed: Duration) -> f64 {
    let t = elapsed.as_secs_f64() / 0.33;
    0.75 * (1.0 - (2.0 * (t - 0.5)).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::Kind;
    #[test]
    fn descriptors_replace_sparse_map_reset_and_keep_manifest_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.actions[0].controllers.push("Encoder".into());
        host.actions[0].raw["Encoder"] = json!({"layout":"$B1","TriggerDescription":{"Rotate":"Level","Push":"Mute","Touch":"Mute","LongTouch":"Reset"}});
        host.assign("Encoder", 1, "test.action").unwrap();
        let slot = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .clone();
        assert_eq!(host.trigger_description(&slot)["rotate"], "Level");
        host.set_trigger_description(&slot, json!({"rotate":"Gain","touch":""}))
            .unwrap();
        assert_eq!(
            host.trigger_description(&slot),
            json!({"rotate":"Gain","touch":""})
        );
        assert!(
            host.set_trigger_description(&slot, json!({"push":17}))
                .is_err()
        );
        assert_eq!(host.trigger_description(&slot)["rotate"], "Gain");
        host.set_trigger_description(&slot, json!({})).unwrap();
        assert_eq!(host.trigger_description(&slot)["push"], "Mute");
        assert_eq!(
            host.actions[0].raw["Encoder"]["TriggerDescription"]["Rotate"],
            "Level"
        );
        let key = host
            .slots
            .iter()
            .find(|s| s.controller == "Keypad")
            .unwrap()
            .clone();
        assert!(host.set_trigger_description(&key, json!({})).is_err());
    }
    #[test]
    fn key_badge_retrigger_replaces_kind_and_restores_latest_image_on_expiry() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let slot = host.slots[0].clone();
        host.render().unwrap();
        let before = host.slots[0].preview.clone();
        host.show_feedback(&slot, Kind::Ok).unwrap();
        let initial = host.badges[&slot.context];
        assert_ne!(host.slots[0].preview, before);
        assert_eq!(initial.expires - initial.started, Duration::from_secs(3));
        host.show_feedback(&slot, Kind::Alert).unwrap();
        let updated = host.badges[&slot.context];
        assert_eq!(updated.kind, Kind::Alert);
        assert!(updated.started > initial.started);
        host.slots[0].title = "Updated while alert was visible".into();
        host.render().unwrap();
        let expected = host
            .renderer
            .key(&host.slots[0].title, host.slots[0].background.as_ref());
        host.poll_feedback_at(updated.expires);
        assert!(host.badges.is_empty());
        let image =
            image::load_from_memory(&STANDARD.decode(&host.slots[0].preview).unwrap()).unwrap();
        assert_eq!(image.to_rgb8(), expected.to_rgb8());
        assert!(
            serde_json::to_string(&host.slots[0])
                .unwrap()
                .find("badges")
                .is_none()
        );
    }
    #[test]
    fn dial_warning_curve_and_red_composition_follow_native_330ms_timeline() {
        assert_eq!(dial_opacity(Duration::ZERO), 0.75);
        assert_eq!(dial_opacity(Duration::from_millis(165)), 0.75);
        assert!((dial_opacity(Duration::from_micros(247500)) - 0.375).abs() < 1e-9);
        assert_eq!(dial_opacity(Duration::from_millis(330)), 0.0);
        assert_eq!(dial_opacity(Duration::from_secs(10)), 0.0);
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.actions[0].controllers.push("Encoder".into());
        host.assign("Encoder", 0, "test.action").unwrap();
        let slot = host
            .slots
            .iter()
            .find(|s| s.controller == "Encoder")
            .unwrap()
            .clone();
        host.show_feedback(&slot, Kind::Alert).unwrap();
        let badge = host.badges[&slot.context];
        assert_eq!(badge.expires - badge.started, Duration::from_millis(330));
        assert!(host.show_feedback(&slot, Kind::Ok).is_err());
        let original = DynamicImage::ImageRgb8(RgbImage::from_pixel(200, 100, Rgb([20, 40, 80])));
        let mut peak = original.clone();
        paint_dial(&host.badges, &mut peak, &slot, badge.started);
        for (x, y) in [(0, 0), (100, 50), (199, 99)] {
            let p = peak.get_pixel(x, y);
            assert!(p[0] > 190 && p[1] < 15 && p[2] < 25);
        }
        let mut expired = original.clone();
        paint_dial(&host.badges, &mut expired, &slot, badge.expires);
        assert_eq!(expired, original);
    }
    #[test]
    fn shutdown_queues_clean_frame_and_rejects_late_feedback() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let (tx, rx) = std::sync::mpsc::channel();
        host.hardware = Some(tx);
        let slot = host.slots[0].clone();
        host.show_feedback(&slot, Kind::Alert).unwrap();
        while rx.try_recv().is_ok() {}
        stop_plugins(&mut host);
        let key = rx
            .try_iter()
            .find_map(|draw| match draw {
                Draw::Key(0, image) => Some(image),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            key,
            host.renderer.key(&slot.title, slot.background.as_ref())
        );
        host.plugin_message(0, json!({"event":"showAlert","context":slot.context}))
            .unwrap();
        assert!(host.badges.is_empty());
        assert!(!host.running);
    }
    #[test]
    fn selected_stack_warning_owns_parent_cell_and_inactive_child_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        host.actions[0].controllers.push("Encoder".into());
        host.actions.push(stack::action());
        host.assign("Encoder", 0, crate::dial_stack::UUID).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.uuid == crate::dial_stack::UUID)
            .unwrap()
            .context
            .clone();
        host.save_stack(&parent,json!({"Actions":[{"UUID":"test.action","ActionID":"a"},{"UUID":"test.action","ActionID":"b"}]})).unwrap();
        let stack = host.visible_slot(&parent).unwrap().clone();
        let active = stack.children[0][0].clone();
        let inactive = stack.children[0][1].clone();
        host.show_feedback(&inactive, Kind::Alert).unwrap();
        assert!(host.badges.is_empty());
        host.show_feedback(&active, Kind::Alert).unwrap();
        assert!(host.badges.contains_key(&parent));
        assert!(!host.badges.contains_key(&active.context));
        host.select_stack(&parent, 1, false).unwrap();
        assert!(host.badges.contains_key(&parent));
    }
}
