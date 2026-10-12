//! Live system-volume dials. Pulse state is authoritative; no synthetic counter.
use super::*;

pub(super) fn is_volume(slot: &Slot) -> bool {
    slot.controller == "Encoder"
        && slot.uuid == crate::system_actions::MULTIMEDIA
        && slot.settings["actionIdx"] == 18
}
fn formatted(percent: f64) -> String {
    let rounded = (percent * 10.0).round() / 10.0;
    if rounded.fract().abs() < 0.001 {
        format!("{rounded:.0}%")
    } else {
        format!("{rounded:.1}%")
    }
}
impl Host {
    fn volume_worker(&mut self) -> &crate::volume::Worker {
        let target = if self.simulate && self.diagnostics {
            std::env::var("DECKFORGE_VOLUME_TEST_SINK").ok()
        } else {
            None
        };
        self.volume.get_or_insert_with(|| match target {
            Some(name) => crate::volume::Worker::for_sink(Some(name)),
            None => crate::volume::Worker::new(),
        })
    }
    pub(super) fn volume_input(
        &mut self,
        slot: &Slot,
        trigger: crate::system_actions::Trigger,
    ) -> Result<()> {
        let step = slot
            .settings
            .get("linuxVolumeStep")
            .map(|v| v.as_f64().context("Volume step must be numeric"))
            .transpose()?
            .unwrap_or(5.0);
        if !step.is_finite() || !(0.1..=100.0).contains(&step) {
            bail!("Volume step must be between 0.1 and 100 percent per tick");
        }
        let worker = self.volume_worker();
        worker.watch(true);
        match trigger {
            crate::system_actions::Trigger::Rotate(ticks) => worker.adjust(ticks, step)?,
            crate::system_actions::Trigger::Up | crate::system_actions::Trigger::Tap => {
                worker.toggle()?
            }
            crate::system_actions::Trigger::Down => {}
        }
        if !matches!(trigger, crate::system_actions::Trigger::Down) {
            self.system_status.clear();
        }
        Ok(())
    }
    pub(super) fn update_volume_feedback(&mut self) {
        let contexts: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| is_volume(s))
            .map(|s| s.context.clone())
            .collect();
        if !contexts.is_empty() {
            self.volume_worker().watch(true);
        } else if let Some(worker) = &self.volume {
            worker.watch(false);
        }
        for context in contexts {
            let Some(slot) = self.slots.iter_mut().find_map(|s| s.find_mut(&context)) else {
                continue;
            };
            if slot.layout != "$B1" {
                slot.layout = "$B1".into();
                slot.layout_cache = None;
                slot.image_cache.clear();
            }
            slot.feedback = if let Some(state) = &self.volume_state {
                json!({"title":if state.muted {"Volume (muted)"}else{"System volume"},"value":formatted(state.percent),
                    "indicator":{"enabled":true,"value":state.percent.clamp(0.0,100.0),"bar_fill_c":if state.muted {"#808080"}else{"#5acaa9"}}})
            } else {
                json!({"title":if self.volume_error.is_some(){"Audio unavailable"}else{"System volume"},"value":"--","indicator":{"enabled":false}})
            };
        }
    }
    pub(super) fn poll_volume(&mut self) {
        let reports = self
            .volume
            .as_ref()
            .map(|worker| worker.drain())
            .unwrap_or_default();
        let mut changed = false;
        for report in reports {
            match report {
                Ok(state) => {
                    if self
                        .volume_state
                        .as_ref()
                        .is_none_or(|previous| previous.sink != state.sink)
                    {
                        self.log(format!(
                            "System volume connected: {} ({})",
                            state.name,
                            formatted(state.percent)
                        ));
                    }
                    if self.volume_state.as_ref() != Some(&state) || self.volume_error.is_some() {
                        changed = true;
                    }
                    if self
                        .volume_error
                        .as_ref()
                        .is_some_and(|error| self.system_status == *error)
                    {
                        self.system_status.clear();
                    }
                    self.volume_state = Some(state);
                    self.volume_error = None;
                }
                Err(error) => {
                    let message = format!("System volume unavailable: {error:#}");
                    if self.volume_error.as_ref() != Some(&message) {
                        self.log(message.clone());
                        self.system_status = message.clone();
                        changed = true;
                    }
                    self.volume_error = Some(message);
                    self.volume_state = None;
                }
            }
        }
        if changed && let Err(error) = self.render() {
            self.log(format!("Volume display update: {error:#}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_percent_retains_useful_precision_without_float_noise() {
        assert_eq!(formatted(49.9999), "50%");
        assert_eq!(formatted(12.5004), "12.5%");
        assert_eq!(formatted(125.0), "125%");
    }
}
