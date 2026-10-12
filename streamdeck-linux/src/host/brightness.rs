//! Original brightness action behavior. Linux uses the native fallback repeat
//! timing (500 ms then 100 ms); Windows keyboard preferences have no universal Linux API.
use super::*;
use crate::brightness;

impl Host {
    pub(super) fn brightness_value(&self) -> u8 {
        self.configured_controls()
            .brightness
            .unwrap_or(brightness::DEFAULT)
    }
    fn change_brightness(&mut self, value: u8) -> Result<()> {
        let mut controls = self.configured_controls();
        if controls.brightness != Some(value) {
            controls.brightness = Some(value);
            self.set_device_controls(controls)?;
        }
        Ok(())
    }
    pub(super) fn brightness_input(
        &mut self,
        slot: &Slot,
        trigger: crate::system_actions::Trigger,
    ) -> Result<()> {
        use crate::system_actions::Trigger;
        match (slot.controller.as_str(), trigger) {
            ("Keypad", Trigger::Down) => {
                let value = brightness::key_value(&slot.settings, self.brightness_value())?;
                if value == self.brightness_value() && brightness::mode(&slot.settings).is_err() {
                    return Ok(());
                }
                self.change_brightness(value)?;
                if let Ok(mode @ 0..=1) = brightness::mode(&slot.settings) {
                    self.brightness_repeats.insert(
                        slot.context.clone(),
                        (
                            mode,
                            self.multi_clock.elapsed() + Duration::from_millis(500),
                        ),
                    );
                    self.multi_notify.notify_one();
                }
            }
            ("Keypad", Trigger::Up) => {
                self.brightness_repeats.remove(&slot.context);
            }
            ("Encoder", Trigger::Rotate(ticks)) if ticks != 0 => {
                let pressed = self
                    .dial_pressed
                    .get(slot.index as usize)
                    .copied()
                    .unwrap_or(false);
                self.change_brightness(brightness::rotate(
                    self.brightness_value(),
                    ticks,
                    pressed,
                ))?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn next_brightness(&self) -> Option<Duration> {
        self.brightness_repeats.values().map(|(_, due)| *due).min()
    }
    pub(super) fn poll_brightness(&mut self) {
        let now = self.multi_clock.elapsed();
        let due: Vec<_> = self
            .brightness_repeats
            .iter()
            .filter(|(_, (_, due))| *due <= now)
            .map(|(context, (mode, _))| (context.clone(), *mode))
            .collect();
        for (context, mode) in due {
            match brightness::key_value(&json!({"actionIdx":mode}), self.brightness_value())
                .and_then(|value| self.change_brightness(value))
            {
                Ok(()) => {
                    if let Some((_, due)) = self.brightness_repeats.get_mut(&context) {
                        *due = now + Duration::from_millis(100);
                    }
                }
                Err(error) => {
                    self.brightness_repeats.remove(&context);
                    self.system_status = format!("Brightness failed: {error:#}");
                }
            }
        }
    }
    pub(super) fn update_brightness_feedback(&mut self) {
        let value = self.brightness_value();
        for slot in &mut self.slots {
            update(slot, value);
        }
        fn update(slot: &mut Slot, value: u8) {
            if slot.uuid == brightness::UUID && slot.controller == "Encoder" {
                if slot.layout != "$B1" {
                    slot.layout = "$B1".into();
                    slot.layout_cache = None;
                    slot.image_cache.clear();
                }
                slot.feedback =
                    json!({"title":"Brightness","value":format!("{value}%"),"indicator":value});
            }
            for child in slot.children.iter_mut().flatten() {
                update(child, value);
            }
        }
    }
}
