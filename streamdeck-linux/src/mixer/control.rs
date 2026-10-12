//! Pure controls for Deckforge's linear-amplitude mixer. These are explicit
//! Linux policies, not claims about Wave Link's undocumented fade/link curves.
use super::{Config, OutputControl};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Route { channel: String, mix: String },
    Mix { mix: String },
    Output { mix: String, output: String },
}

pub fn all_targets(config: &Config) -> Vec<Target> {
    let mut result: Vec<_> = config
        .routes
        .iter()
        .map(|route| Target::Route {
            channel: route.channel.clone(),
            mix: route.mix.clone(),
        })
        .collect();
    for mix in &config.mixes {
        result.push(Target::Mix {
            mix: mix.id.clone(),
        });
        result.extend(mix.outputs.iter().map(|output| Target::Output {
            mix: mix.id.clone(),
            output: output.clone(),
        }));
    }
    result.sort();
    result
}

pub fn read(config: &Config, target: &Target) -> Result<OutputControl> {
    let value = match target {
        Target::Route { channel, mix } => {
            ensure!(
                config.channels.iter().any(|c| &c.id == channel)
                    && config.mixes.iter().any(|m| &m.id == mix),
                "Mixer route owner no longer exists"
            );
            let route = config
                .routes
                .iter()
                .find(|r| &r.channel == channel && &r.mix == mix)
                .context("Mixer route no longer exists")?;
            OutputControl {
                volume: route.volume,
                muted: route.muted,
            }
        }
        Target::Mix { mix } => {
            let mix = config
                .mixes
                .iter()
                .find(|m| &m.id == mix)
                .context("Mixer mix no longer exists")?;
            OutputControl {
                volume: mix.volume,
                muted: mix.muted,
            }
        }
        Target::Output { mix, output } => {
            let mix = config
                .mixes
                .iter()
                .find(|m| &m.id == mix)
                .context("Mixer mix no longer exists")?;
            ensure!(
                mix.outputs.contains(output),
                "Mixer output is not assigned to this mix"
            );
            mix.output_controls.get(output).cloned().unwrap_or_default()
        }
    };
    super::valid_volume(value.volume)?;
    Ok(value)
}

/// Returns volume targets touched, including an explicitly unchanged volume.
/// Linked routes receive the same percentage-point delta; mutes are independent.
pub fn patch(
    config: &mut Config,
    target: &Target,
    volume: Option<f64>,
    muted: Option<bool>,
) -> Result<Vec<Target>> {
    config.validate()?;
    let previous = read(config, target)?;
    if let Some(volume) = volume {
        super::valid_volume(volume)?;
    }
    let mut candidate = config.clone();
    let mut touched = Vec::new();
    if let Some(volume) = volume {
        if let Target::Route { channel, .. } = target
            && candidate
                .channels
                .iter()
                .any(|c| &c.id == channel && c.linked)
        {
            let delta = volume - previous.volume;
            for route in &mut candidate.routes {
                if &route.channel == channel {
                    route.volume = (route.volume + delta).clamp(0.0, 100.0);
                    touched.push(Target::Route {
                        channel: channel.clone(),
                        mix: route.mix.clone(),
                    });
                }
            }
        } else {
            write(&mut candidate, target, Some(volume), None)?;
            touched.push(target.clone());
        }
    }
    if muted.is_some() {
        write(&mut candidate, target, None, muted)?;
    }
    candidate.validate()?;
    touched.sort();
    *config = candidate;
    Ok(touched)
}

fn write(
    config: &mut Config,
    target: &Target,
    volume: Option<f64>,
    muted: Option<bool>,
) -> Result<()> {
    let (level, mute) = match target {
        Target::Route { channel, mix } => {
            let route = config
                .routes
                .iter_mut()
                .find(|r| &r.channel == channel && &r.mix == mix)
                .context("Mixer route no longer exists")?;
            (&mut route.volume, &mut route.muted)
        }
        Target::Mix { mix } => {
            let mix = config
                .mixes
                .iter_mut()
                .find(|m| &m.id == mix)
                .context("Mixer mix no longer exists")?;
            (&mut mix.volume, &mut mix.muted)
        }
        Target::Output { mix, output } => {
            let mix = config
                .mixes
                .iter_mut()
                .find(|m| &m.id == mix)
                .context("Mixer mix no longer exists")?;
            ensure!(
                mix.outputs.contains(output),
                "Mixer output is not assigned to this mix"
            );
            let control = mix.output_controls.entry(output.clone()).or_default();
            (&mut control.volume, &mut control.muted)
        }
    };
    if let Some(value) = volume {
        *level = value;
    }
    if let Some(value) = muted {
        *mute = value;
    }
    Ok(())
}

/// Routing equality ignores display order/names and every control parameter.
pub fn same_routing(a: &Config, b: &Config) -> bool {
    fn channels(config: &Config) -> Vec<(&str, bool, Vec<&super::AppSelector>, Vec<&str>)> {
        let mut result: Vec<_> = config
            .channels
            .iter()
            .map(|channel| {
                let mut apps: Vec<_> = channel.apps.iter().collect();
                apps.sort();
                let mut sources: Vec<_> = channel.sources.iter().map(String::as_str).collect();
                sources.sort();
                (
                    channel.id.as_str(),
                    channel.capture_unassigned,
                    apps,
                    sources,
                )
            })
            .collect();
        result.sort();
        result
    }
    fn mixes(config: &Config) -> Vec<(&str, Vec<&str>)> {
        let mut result: Vec<_> = config
            .mixes
            .iter()
            .map(|mix| {
                let mut outputs: Vec<_> = mix.outputs.iter().map(String::as_str).collect();
                outputs.sort();
                (mix.id.as_str(), outputs)
            })
            .collect();
        result.sort();
        result
    }
    fn routes(config: &Config) -> Vec<(&str, &str)> {
        let mut result: Vec<_> = config
            .routes
            .iter()
            .map(|r| (r.channel.as_str(), r.mix.as_str()))
            .collect();
        result.sort();
        result
    }
    a.enabled == b.enabled
        && channels(a) == channels(b)
        && mixes(a) == mixes(b)
        && routes(a) == routes(b)
}

#[derive(Clone)]
struct Ramp {
    from: f64,
    to: f64,
    started: Duration,
    duration: Duration,
}
impl Ramp {
    fn finished(&self, now: Duration) -> bool {
        now.saturating_sub(self.started) >= self.duration
    }
    fn value(&self, now: Duration) -> f64 {
        if self.finished(now) {
            return self.to;
        }
        let fraction = now.saturating_sub(self.started).as_secs_f64() / self.duration.as_secs_f64();
        self.from + (self.to - self.from) * fraction
    }
}

pub struct Fades {
    target: Config,
    ramps: BTreeMap<Target, Ramp>,
}
impl Fades {
    pub fn new(config: Config) -> Self {
        Self {
            target: config,
            ramps: BTreeMap::new(),
        }
    }
    pub fn target(&self) -> &Config {
        &self.target
    }
    /// Completed ramps are retired by sample() or submit().
    pub fn active(&self) -> bool {
        !self.ramps.is_empty()
    }
    pub fn submit(
        &mut self,
        config: Config,
        changes: BTreeMap<Target, Duration>,
        immediate: bool,
        now: Duration,
    ) -> Result<()> {
        config.validate()?;
        for target in changes.keys() {
            read(&config, target)?;
        }
        if immediate || !same_routing(&self.target, &config) {
            self.target = config;
            self.ramps.clear();
            return Ok(());
        }
        let current = self.sampled(now);
        let mut ramps = self.ramps.clone();
        ramps.retain(|_, ramp| !ramp.finished(now));
        for target in all_targets(&config) {
            let to = read(&config, &target)?.volume;
            if let Some(duration) = changes.get(&target) {
                let from = read(&current, &target)?.volume;
                if duration.is_zero() || from == to {
                    ramps.remove(&target);
                } else {
                    ramps.insert(
                        target,
                        Ramp {
                            from,
                            to,
                            started: now,
                            duration: *duration,
                        },
                    );
                }
            } else if read(&self.target, &target)?.volume != to {
                // No fade was requested for this changed value. Never let an
                // older ramp overwrite the new immediate setting.
                ramps.remove(&target);
            }
        }
        self.target = config;
        self.ramps = ramps;
        Ok(())
    }
    fn sampled(&self, now: Duration) -> Config {
        let mut config = self.target.clone();
        for (target, ramp) in &self.ramps {
            // submit validated these targets and cancels on routing changes.
            write(&mut config, target, Some(ramp.value(now)), None)
                .expect("Fade target belongs to its validated configuration");
        }
        config
    }
    pub fn sample(&mut self, now: Duration) -> Config {
        self.ramps.retain(|_, ramp| !ramp.finished(now));
        self.sampled(now)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AppSelector, Channel, Route};
    use super::*;
    fn config() -> Config {
        Config {
            enabled: true,
            channels: vec![Channel {
                id: "music".into(),
                name: "Music".into(),
                ..Default::default()
            }],
            routes: vec![
                Route {
                    channel: "music".into(),
                    mix: "monitor".into(),
                    volume: 40.,
                    ..Default::default()
                },
                Route {
                    channel: "music".into(),
                    mix: "stream".into(),
                    volume: 70.,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
    fn route(mix: &str) -> Target {
        Target::Route {
            channel: "music".into(),
            mix: mix.into(),
        }
    }
    fn mix() -> Target {
        Target::Mix {
            mix: "monitor".into(),
        }
    }
    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }
    fn change(target: Target, duration: u64) -> BTreeMap<Target, Duration> {
        [(target, seconds(duration))].into()
    }
    fn level(config: &Config, target: &Target) -> f64 {
        read(config, target).unwrap().volume
    }
    #[test]
    fn linked_delta_clamps_existing_routes_and_mute_stays_local() {
        let mut value = config();
        value.channels[0].linked = true;
        let touched = patch(&mut value, &route("monitor"), Some(80.), Some(true)).unwrap();
        assert_eq!(touched, vec![route("monitor"), route("stream")]);
        assert_eq!(level(&value, &route("monitor")), 80.);
        assert_eq!(level(&value, &route("stream")), 100.);
        assert!(read(&value, &route("monitor")).unwrap().muted);
        assert!(!read(&value, &route("stream")).unwrap().muted);
        assert!(
            patch(&mut value, &route("stream"), None, Some(true))
                .unwrap()
                .is_empty()
        );
        patch(&mut value, &route("monitor"), Some(60.), None).unwrap();
        assert_eq!(level(&value, &route("stream")), 80.); // independent endpoint clamp, not a hidden ratio.
        value.routes.pop();
        patch(&mut value, &route("monitor"), Some(50.), None).unwrap();
        assert_eq!(value.routes.len(), 1); // Linking never creates a route.
    }
    #[test]
    fn patches_validate_atomically_and_outputs_must_be_assigned() {
        let mut value = config();
        let before = value.clone();
        for invalid in [f64::NAN, f64::INFINITY, -1., 101.] {
            assert!(patch(&mut value, &route("monitor"), Some(invalid), Some(true)).is_err());
            assert_eq!(value, before);
        }
        let output = Target::Output {
            mix: "monitor".into(),
            output: "headphones".into(),
        };
        assert!(patch(&mut value, &output, Some(30.), Some(true)).is_err());
        assert_eq!(value, before);
        value.mixes[0].outputs.push("headphones".into());
        assert_eq!(read(&value, &output).unwrap(), OutputControl::default());
        patch(&mut value, &output, Some(30.), Some(true)).unwrap();
        assert_eq!(
            read(&value, &output).unwrap(),
            OutputControl {
                volume: 30.,
                muted: true
            }
        );
        assert_eq!(value.mixes[0].volume, 100.);
    }
    #[test]
    fn linear_endpoints_and_retarget_are_continuous() {
        let initial = config();
        let mut fade = Fades::new(initial.clone());
        let mut target = initial;
        patch(&mut target, &route("monitor"), Some(100.), None).unwrap();
        fade.submit(target, change(route("monitor"), 10), false, seconds(0))
            .unwrap();
        assert_eq!(level(&fade.sample(seconds(0)), &route("monitor")), 40.);
        assert_eq!(level(&fade.sample(seconds(5)), &route("monitor")), 70.);
        let mut target = fade.target().clone();
        patch(&mut target, &route("monitor"), Some(10.), None).unwrap();
        fade.submit(target, change(route("monitor"), 6), false, seconds(5))
            .unwrap();
        assert_eq!(level(&fade.sample(seconds(5)), &route("monitor")), 70.);
        assert_eq!(level(&fade.sample(seconds(8)), &route("monitor")), 40.);
        assert_eq!(level(&fade.sample(seconds(11)), &route("monitor")), 10.);
        assert!(!fade.active());
    }
    #[test]
    fn independent_fades_survive_metadata_and_immediate_mute_updates() {
        let mut fade = Fades::new(config());
        let mut target = fade.target().clone();
        patch(&mut target, &route("monitor"), Some(80.), None).unwrap();
        patch(&mut target, &mix(), Some(0.), None).unwrap();
        fade.submit(
            target,
            [(route("monitor"), seconds(10)), (mix(), seconds(20))].into(),
            false,
            seconds(0),
        )
        .unwrap();
        let mut metadata = fade.target().clone();
        metadata.channels[0].name = "Renamed".into();
        metadata.channels[0].linked = true;
        metadata.fade_ms = 500;
        metadata.routes[0].muted = true;
        fade.submit(metadata, BTreeMap::new(), false, seconds(5))
            .unwrap();
        let midway = fade.sample(seconds(10));
        assert_eq!(level(&midway, &route("monitor")), 80.);
        assert_eq!(level(&midway, &mix()), 50.);
        assert!(midway.routes[0].muted && fade.active());
        assert_eq!(level(&fade.sample(seconds(20)), &mix()), 0.);
        assert!(!fade.active());
    }
    #[test]
    fn zero_duration_unlisted_change_and_structure_cancel_correctly() {
        let mut fade = Fades::new(config());
        let mut target = fade.target().clone();
        patch(&mut target, &route("monitor"), Some(100.), None).unwrap();
        fade.submit(
            target.clone(),
            change(route("monitor"), 10),
            false,
            seconds(0),
        )
        .unwrap();
        patch(&mut target, &route("monitor"), Some(20.), None).unwrap();
        fade.submit(
            target.clone(),
            change(route("monitor"), 0),
            false,
            seconds(2),
        )
        .unwrap();
        assert_eq!(level(&fade.sample(seconds(2)), &route("monitor")), 20.);
        assert!(!fade.active());
        patch(&mut target, &route("monitor"), Some(100.), None).unwrap();
        fade.submit(
            target.clone(),
            change(route("monitor"), 10),
            false,
            seconds(3),
        )
        .unwrap();
        patch(&mut target, &route("monitor"), Some(30.), None).unwrap();
        fade.submit(target.clone(), BTreeMap::new(), false, seconds(4))
            .unwrap();
        assert_eq!(level(&fade.sample(seconds(4)), &route("monitor")), 30.);
        assert!(!fade.active());
        patch(&mut target, &route("monitor"), Some(100.), None).unwrap();
        fade.submit(
            target.clone(),
            change(route("monitor"), 10),
            false,
            seconds(5),
        )
        .unwrap();
        target.channels[0].sources.push("microphone".into());
        fade.submit(target, BTreeMap::new(), false, seconds(6))
            .unwrap();
        assert!(!fade.active());
        assert_eq!(level(&fade.sample(seconds(6)), &route("monitor")), 100.);
    }
    #[test]
    fn invalid_submission_keeps_target_and_running_curve() {
        let mut fade = Fades::new(config());
        let mut target = fade.target().clone();
        patch(&mut target, &route("monitor"), Some(100.), None).unwrap();
        fade.submit(
            target.clone(),
            change(route("monitor"), 10),
            false,
            seconds(0),
        )
        .unwrap();
        assert!(
            fade.submit(
                target.clone(),
                change(route("missing"), 2),
                true,
                seconds(4)
            )
            .is_err()
        );
        let mut invalid = target.clone();
        invalid.routes[0].volume = f64::NAN;
        assert!(
            fade.submit(invalid, BTreeMap::new(), true, seconds(4))
                .is_err()
        );
        assert_eq!(fade.target(), &target);
        assert!(fade.active());
        assert_eq!(level(&fade.sample(seconds(5)), &route("monitor")), 70.);
        fade.submit(target, BTreeMap::new(), true, seconds(5))
            .unwrap();
        assert!(!fade.active());
    }
    #[test]
    fn topology_ignores_order_and_labels_but_not_connections() {
        let a = config();
        let mut b = a.clone();
        b.channels[0].linked = true;
        b.channels[0].name = "Alias".into();
        b.mixes.reverse();
        b.routes.reverse();
        b.routes[0].volume = 20.;
        b.routes[0].muted = true;
        b.fade_ms = 100;
        assert!(same_routing(&a, &b));
        b.channels[0]
            .apps
            .push(AppSelector::ProcessBinary("player".into()));
        assert!(!same_routing(&a, &b));
        let mut b = a.clone();
        b.mixes[0].outputs.push("headphones".into());
        assert!(!same_routing(&a, &b));
        let mut b = a.clone();
        b.enabled = false;
        assert!(!same_routing(&a, &b));
        assert_eq!(all_targets(&a).len(), 4);
        assert_eq!(
            serde_json::to_value(route("monitor")).unwrap(),
            serde_json::json!({"kind":"route","channel":"music","mix":"monitor"})
        );
    }
    #[test]
    fn unclamped_linked_changes_are_reversible() {
        let mut value = config();
        value.channels[0].linked = true;
        let before = value.clone();
        patch(&mut value, &route("monitor"), Some(50.), None).unwrap();
        assert_eq!(level(&value, &route("stream")), 80.);
        patch(&mut value, &route("monitor"), Some(40.), None).unwrap();
        assert_eq!(value, before);
    }

    #[test]
    fn output_fade_does_not_modify_mix_master_or_other_output() {
        let mut initial = config();
        initial.mixes[0].outputs = vec!["headphones".into(), "speakers".into()];
        let headphones = Target::Output {
            mix: "monitor".into(),
            output: "headphones".into(),
        };
        let speakers = Target::Output {
            mix: "monitor".into(),
            output: "speakers".into(),
        };
        let mut fades = Fades::new(initial.clone());
        patch(&mut initial, &headphones, Some(20.), None).unwrap();
        fades
            .submit(initial, change(headphones.clone(), 8), false, seconds(0))
            .unwrap();
        let mut muted = fades.target().clone();
        patch(&mut muted, &headphones, None, Some(true)).unwrap();
        fades
            .submit(muted, BTreeMap::new(), false, seconds(2))
            .unwrap();
        let sampled = fades.sample(seconds(4));
        assert_eq!(
            read(&sampled, &headphones).unwrap(),
            OutputControl {
                volume: 60.,
                muted: true
            }
        );
        assert_eq!(read(&sampled, &speakers).unwrap(), OutputControl::default());
        assert_eq!(level(&sampled, &mix()), 100.);
        assert_eq!(level(&fades.sample(seconds(8)), &headphones), 20.);
        assert!(!fades.active());
    }
}
