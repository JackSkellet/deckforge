//! Native Open Application short press / 500 ms hold behavior.
use super::*;
use crate::system_app::{Config, Operation, UUID};
pub(super) fn visual<'a>(
    artwork: &'a crate::application_artwork::Artwork,
    slot: &Slot,
) -> Option<&'a crate::application_artwork::Visual> {
    (slot.uuid == UUID && !appearance::custom_image(slot))
        .then(|| artwork.get(&slot.context))
        .flatten()
        .filter(|a| a.settings == slot.settings)
        .map(AsRef::as_ref)
}
impl Host {
    pub(super) fn open_app_input(
        &mut self,
        slot: &Slot,
        trigger: crate::system_actions::Trigger,
    ) -> Result<()> {
        use crate::system_actions::Trigger;
        match trigger {
            Trigger::Down => {
                let config = Config::parse(&slot.settings)?;
                self.open_app_holds.entry(slot.context.clone()).or_insert((
                    self.multi_clock.elapsed() + Duration::from_millis(500),
                    config,
                ));
                self.multi_notify.notify_one();
            }
            Trigger::Up => {
                if let Some((due, config)) = self.open_app_holds.remove(&slot.context) {
                    let operation = if self.multi_clock.elapsed() < due {
                        Some(Operation::Open)
                    } else {
                        config.hold
                    };
                    if let Some(operation) = operation {
                        self.submit_app(&slot.context, config, operation)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn submit_app(&mut self, context: &str, config: Config, operation: Operation) -> Result<()> {
        self.system_actions
            .get_or_insert_with(crate::system_actions::Worker::new)
            .submit(
                context,
                crate::system_actions::Plan::Application(config, operation),
            )
    }
    pub(super) fn next_open_app(&self) -> Option<Duration> {
        self.open_app_holds.values().map(|(due, _)| *due).min()
    }
    pub(super) fn poll_open_apps(&mut self) {
        let now = self.multi_clock.elapsed();
        let due: Vec<_> = self
            .open_app_holds
            .iter()
            .filter(|(_, (due, _))| *due <= now)
            .map(|(context, _)| context.clone())
            .collect();
        for context in due {
            if let Some((_, config)) = self.open_app_holds.remove(&context)
                && self
                    .visible_slot(&context)
                    .is_some_and(|slot| slot.uuid == UUID)
                && let Some(operation) = config.hold
                && let Err(error) = self.submit_app(&context, config, operation)
            {
                self.system_status = format!("Open Application failed: {error:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_press_none_consumes_release_and_departure_cancels_pending_hold() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.actions.extend(
            crate::system_actions::definitions()
                .into_iter()
                .filter(|(id, _, _)| *id == UUID)
                .map(|(id, name, _)| Action {
                    uuid: id.into(),
                    name: name.into(),
                    plugin: "system".into(),
                    controllers: vec!["Keypad".into()],
                    raw: json!({}),
                }),
        );
        host.assign("Keypad", 0, UUID).unwrap();
        let mut slot = host.slots[0].clone();
        slot.settings = json!({"exec":"/usr/bin/true","long_press":"none"});
        host.store_slot(slot.clone());
        host.open_app_input(&slot, crate::system_actions::Trigger::Down)
            .unwrap();
        host.multi_clock = std::time::Instant::now() - Duration::from_secs(1);
        host.poll_open_apps();
        host.open_app_input(&slot, crate::system_actions::Trigger::Up)
            .unwrap();
        assert!(host.open_app_holds.is_empty());
        assert!(host.system_actions.is_none());
        host.open_app_input(&slot, crate::system_actions::Trigger::Down)
            .unwrap();
        host.cancel_departing_inputs(&HashSet::new());
        host.open_app_input(&slot, crate::system_actions::Trigger::Up)
            .unwrap();
        assert!(host.system_actions.is_none());
    }
}

#[cfg(test)]
mod artwork_tests {
    use super::*;
    use image::GenericImageView;
    use std::sync::Arc;
    #[test]
    fn derived_artwork_reaches_hardware_preserves_overrides_and_rejects_stale_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.actions.push(Action {
            uuid: UUID.into(),
            name: "Open Application".into(),
            plugin: "system".into(),
            controllers: vec!["Keypad".into()],
            raw: json!({}),
        });
        host.assign("Keypad", 0, UUID).unwrap();
        let id = host.slots[0].context.clone();
        let settings = json!({"exec":"/usr/bin/true"});
        host.slots[0].settings = settings.clone();
        let raw = host.slots[0].raw.clone();
        host.application_artwork.insert(
            id.clone(),
            Arc::new(crate::application_artwork::Visual {
                settings,
                running: Some(true),
                icon: Some(Arc::new(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                    64,
                    64,
                    Rgb([0, 0, 255]),
                )))),
            }),
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        host.render().unwrap();
        let picture =
            image::load_from_memory(&STANDARD.decode(&host.slots[0].preview).unwrap()).unwrap();
        assert_eq!(picture.get_pixel(60, 50).0, [0, 0, 255, 255]);
        assert_eq!(picture.get_pixel(11, 11).0, [141, 216, 188, 255]);
        assert!(receiver.try_iter().any(
            |draw| matches!(draw, Draw::Key(0, frame) if frame.get_pixel(60,50).0 == [0,0,255,255])
        ));
        assert_eq!(host.slots[0].raw, raw);
        assert!(!appearance::custom_image(&host.slots[0]));
        let red = DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 64, Rgb([255, 0, 0])));
        let state = host.slots[0].appearances.entry(0).or_default();
        state.user_image = Some(root.path().join("custom.png"));
        red.save(state.user_image.as_ref().unwrap()).unwrap();
        state.user_pixels = Some(red.clone());
        host.slots[0].background = Some(red);
        host.render().unwrap();
        let custom =
            image::load_from_memory(&STANDARD.decode(&host.slots[0].preview).unwrap()).unwrap();
        assert_eq!(custom.get_pixel(11, 11).0, [255, 0, 0, 255]);
        host.slots[0].appearances.clear();
        host.slots[0].background = None;
        host.slots[0].settings = json!({"exec":"/usr/bin/false"});
        host.render().unwrap();
        let stale =
            image::load_from_memory(&STANDARD.decode(&host.slots[0].preview).unwrap()).unwrap();
        assert_ne!(stale.get_pixel(60, 50).0, [0, 0, 255, 255]);
    }
}
