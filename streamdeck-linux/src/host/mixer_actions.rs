//! Deckforge's own mixer actions. These settings and identities are original;
//! they do not claim compatibility with an unverified Elgato/Wave Link schema.
use super::*;
use crate::mixer::{Config, Snapshot};
use crate::system_actions::Trigger;
use anyhow::ensure;

pub(super) const CHANNEL: &str = "com.deckforge.mixer.channel";
pub(super) const MIX: &str = "com.deckforge.mixer.mix";

pub(super) fn supported(uuid: &str) -> bool {
    matches!(uuid, CHANNEL | MIX)
}
pub(super) fn defaults(uuid: &str) -> Option<Value> {
    supported(uuid).then(|| {
        let mut settings =
            json!({"mix":"monitor","step":5,"mode":"toggle_mute","volume":50,"delta":5});
        if uuid == CHANNEL {
            settings["channel"] = json!("");
        }
        settings
    })
}
pub(super) fn definitions() -> Vec<Action> {
    [(CHANNEL,"Mixer channel"),(MIX,"Mixer output mix")].into_iter().map(|(uuid,name)|Action {
        uuid:uuid.into(), name:name.into(), plugin:"deckforge.mixer".into(),
        controllers:vec!["Keypad".into(),"Encoder".into()],
        raw:json!({"Name":name,"Category":"Audio mixer","UserTitleEnabled":false,
            "DisableAutomaticStates":true,"SupportedInMultiActions":true,
            "Tooltip":if uuid==CHANNEL {"Control one configured channel in a mix. Turn to change its volume; press or tap to mute. Enable and configure the mixer first."}else{"Control a configured mix's master volume. Turn to change volume; press or tap to mute. Enable and configure the mixer first."},
            "Encoder":{"layout":"$B1"},"States":[{"Title":name}],
            "DeckforgeSettings":settings_description(uuid)}),
    }).collect()
}
/// Editor metadata for these original settings, not an imported vendor schema.
pub(super) fn settings_description(uuid: &str) -> Value {
    let mut properties = json!({
        "mix":{"type":"string","label":"Mix","description":"Choose an existing mix ID from the audio mixer."},
        "step":{"type":"number","label":"Dial step (%)","minimum":0.1,"maximum":100,"default":5},
        "mode":{"type":"string","label":"Key action","enum":["toggle_mute","set_volume","change_volume"],"default":"toggle_mute","description":"Applies to keys. Dial push/touch always toggles mute."},
        "volume":{"type":"number","label":"Set volume (%)","minimum":0,"maximum":100,"default":50,"visibleWhen":{"mode":"set_volume"}},
        "delta":{"type":"number","label":"Change volume (%)","minimum":-100,"maximum":100,"default":5,"visibleWhen":{"mode":"change_volume"}}
    });
    if uuid == CHANNEL {
        properties["channel"] = json!({"type":"string","label":"Channel","description":"Choose an existing channel with a configured route to the selected mix."});
    }
    json!({"type":"object","properties":properties,"description":"Volume percentages are linear amplitude. These actions do not enable the mixer or create routes."})
}

#[derive(Clone, Debug, PartialEq)]
struct Target {
    name: String,
    volume: f64,
    muted: bool,
}
fn target(config: &Config, uuid: &str, settings: &Value) -> Result<Target> {
    ensure!(supported(uuid), "Unknown Deckforge mixer action");
    ensure!(
        settings.is_object(),
        "Mixer action settings must be an object"
    );
    let mix_id = settings["mix"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Choose a mixer output mix")?;
    let mix = config
        .mixes
        .iter()
        .find(|mix| mix.id == mix_id)
        .context("The selected mixer output no longer exists")?;
    if uuid == MIX {
        return Ok(Target {
            name: mix.name.clone(),
            volume: mix.volume,
            muted: mix.muted,
        });
    }
    let channel_id = settings["channel"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Choose a mixer channel")?;
    let channel = config
        .channels
        .iter()
        .find(|channel| channel.id == channel_id)
        .context("The selected mixer channel no longer exists")?;
    let route = config
        .routes
        .iter()
        .find(|route| route.channel == channel_id && route.mix == mix_id)
        .context("Configure this channel's route to the selected mix first")?;
    Ok(Target {
        name: format!("{} · {}", channel.name, mix.name),
        volume: route.volume,
        muted: route.muted,
    })
}
fn number(
    settings: &Value,
    key: &str,
    default: f64,
    range: std::ops::RangeInclusive<f64>,
) -> Result<f64> {
    let number = settings
        .get(key)
        .map(|v| {
            v.as_f64()
                .with_context(|| format!("Mixer {key} must be numeric"))
        })
        .transpose()?
        .unwrap_or(default);
    ensure!(
        number.is_finite() && range.contains(&number),
        "Mixer {key} must be between {} and {}",
        range.start(),
        range.end()
    );
    Ok(number)
}
/// Produces a new intent without audio I/O, modifying only an existing target.
fn change(
    config: &Config,
    uuid: &str,
    controller: &str,
    settings: &Value,
    trigger: Trigger,
) -> Result<Option<Config>> {
    // Releases/zero turns carry no mutation and never start or enable a graph.
    match (controller, &trigger) {
        ("Keypad", Trigger::Up) | ("Encoder", Trigger::Down) | ("Encoder", Trigger::Rotate(0)) => {
            return Ok(None);
        }
        ("Keypad", Trigger::Down)
        | ("Encoder", Trigger::Up | Trigger::Tap | Trigger::Rotate(_)) => {}
        _ => bail!("Unsupported controller or input for mixer action"),
    }
    config.validate()?;
    ensure!(
        config.enabled,
        "Open Audio mixer and enable routing before using this action"
    );
    let current = target(config, uuid, settings)?;
    let (volume, muted) = match (controller, trigger) {
        ("Encoder", Trigger::Rotate(ticks)) => (
            (current.volume + f64::from(ticks) * number(settings, "step", 5.0, 0.1..=100.0)?)
                .clamp(0.0, 100.0),
            current.muted,
        ),
        ("Encoder", Trigger::Up | Trigger::Tap) => (current.volume, !current.muted),
        ("Keypad", Trigger::Down) => match settings
            .get("mode")
            .map(|v| v.as_str().context("Mixer key mode must be text"))
            .transpose()?
            .unwrap_or("toggle_mute")
        {
            "toggle_mute" => (current.volume, !current.muted),
            "set_volume" => (
                number(settings, "volume", 50.0, 0.0..=100.0)?,
                current.muted,
            ),
            "change_volume" => (
                (current.volume + number(settings, "delta", 5.0, -100.0..=100.0)?)
                    .clamp(0.0, 100.0),
                current.muted,
            ),
            _ => bail!("Unknown mixer key mode"),
        },
        _ => unreachable!(),
    };
    if volume == current.volume && muted == current.muted {
        return Ok(None);
    }
    let mut next = config.clone();
    let mix_id = settings["mix"].as_str().unwrap();
    if uuid == MIX {
        let mix = next.mixes.iter_mut().find(|mix| mix.id == mix_id).unwrap();
        mix.volume = volume;
        mix.muted = muted;
    } else {
        let channel_id = settings["channel"].as_str().unwrap();
        let route = next
            .routes
            .iter_mut()
            .find(|route| route.channel == channel_id && route.mix == mix_id)
            .unwrap();
        route.volume = volume;
        route.muted = muted;
    }
    next.validate()?;
    Ok(Some(next))
}
fn percent(value: f64) -> String {
    let value = (value * 10.0).round() / 10.0;
    if value.fract().abs() < 0.001 {
        format!("{value:.0}%")
    } else {
        format!("{value:.1}%")
    }
}
#[derive(Debug, PartialEq)]
struct Display {
    title: String,
    value: String,
    volume: Option<f64>,
    muted: Option<bool>,
    pending: bool,
    error: Option<String>,
}
fn display(desired: &Config, observed: &Snapshot, uuid: &str, settings: &Value) -> Display {
    let target = match target(desired, uuid, settings) {
        Ok(target) => target,
        Err(error) => {
            return Display {
                title: "Configure mixer".into(),
                value: "--".into(),
                volume: None,
                muted: None,
                pending: false,
                error: Some(error.to_string()),
            };
        }
    };
    let error = if !desired.enabled {
        Some("Audio mixer is disabled".into())
    } else if let Some(error) = &observed.error {
        Some(error.clone())
    } else if !observed.connected {
        Some("Audio server is disconnected".into())
    } else if !observed.active {
        Some("Mixer routing is not active".into())
    } else {
        None
    };
    if let Some(error) = error {
        return Display {
            title: target.name,
            value: "Unavailable".into(),
            volume: None,
            muted: None,
            pending: false,
            error: Some(error),
        };
    }
    let applied = match self::target(&observed.config, uuid, settings) {
        Ok(target) => target,
        Err(_) => {
            return Display {
                title: target.name,
                value: "Applying…".into(),
                volume: None,
                muted: None,
                pending: true,
                error: None,
            };
        }
    };
    let pending = !observed.config.enabled
        || applied.volume != target.volume
        || applied.muted != target.muted;
    Display {
        title: target.name,
        value: format!(
            "{}{}{}",
            percent(applied.volume),
            if applied.muted { " · Muted" } else { "" },
            if pending { " · Applying" } else { "" }
        ),
        volume: Some(applied.volume),
        muted: Some(applied.muted),
        pending,
        error: None,
    }
}

impl Host {
    pub(super) fn mixer_input(&mut self, slot: &Slot, trigger: Trigger) -> Result<()> {
        let previous = self.mixer_config.clone();
        let Some(next) = change(
            &previous,
            &slot.uuid,
            &slot.controller,
            &slot.settings,
            trigger,
        )?
        else {
            return Ok(());
        };
        ensure!(
            self.mixer.is_some(),
            "Audio mixer is not running; open Audio mixer and enable routing"
        );
        ensure!(self.mixer_state.connected, "Audio server is disconnected");
        if let Some(error) = &self.mixer_state.error {
            bail!("Audio mixer unavailable: {error}");
        }
        ensure!(self.mixer_state.active, "Mixer routing is not active yet");
        // Accumulate against current desired intent while a previous tick applies.
        // The existing worker/config method remains the only graph/persistence owner.
        self.set_mixer_config(next, Some(&previous))?;
        self.system_status = "Mixer change queued".into();
        Ok(())
    }
    pub(super) fn update_mixer_feedback(&mut self) {
        fn update(slot: &mut Slot, desired: &Config, observed: &Snapshot) {
            if supported(&slot.uuid) {
                let state = display(desired, observed, &slot.uuid, &slot.settings);
                if slot.controller == "Encoder" && slot.layout != "$B1" {
                    slot.layout = "$B1".into();
                    slot.layout_cache = None;
                    slot.image_cache.clear();
                }
                slot.feedback = json!({"title":state.title,"value":state.value,"muted":state.muted,"pending":state.pending,"error":state.error,
                    "indicator":{"enabled":state.volume.is_some(),"value":state.volume.unwrap_or(0.0),"bar_fill_c":if state.muted==Some(true){"#808080"}else{"#5acaa9"}}});
                if slot.controller == "Keypad" {
                    slot.title = format!("{}\n{}", state.title, state.value);
                }
            }
            for child in slot.children.iter_mut().flatten() {
                update(child, desired, observed);
            }
        }
        for slot in &mut self.slots {
            update(slot, &self.mixer_config, &self.mixer_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::{Channel, Mix, Route};
    fn config() -> Config {
        Config {
            enabled: true,
            channels: vec![Channel {
                id: "music".into(),
                name: "Music".into(),
                ..Default::default()
            }],
            mixes: vec![
                Mix {
                    id: "monitor".into(),
                    name: "Monitor".into(),
                    volume: 80.0,
                    ..Default::default()
                },
                Mix {
                    id: "stream".into(),
                    name: "Stream".into(),
                    ..Default::default()
                },
            ],
            routes: vec![Route {
                channel: "music".into(),
                mix: "monitor".into(),
                volume: 40.0,
                ..Default::default()
            }],
        }
    }
    #[test]
    fn turns_accumulate_desired_volume_without_changing_graph_or_mute() {
        let before = config();
        let settings = json!({"channel":"music","mix":"monitor","step":2.5});
        let first = change(&before, CHANNEL, "Encoder", &settings, Trigger::Rotate(2))
            .unwrap()
            .unwrap();
        let next = change(&first, CHANNEL, "Encoder", &settings, Trigger::Rotate(3))
            .unwrap()
            .unwrap();
        assert_eq!(next.routes[0].volume, 52.5);
        assert_eq!(next.channels, before.channels);
        assert_eq!(next.mixes, before.mixes);
        assert_eq!(next.routes.len(), before.routes.len());
        assert!(!next.routes[0].muted);
        assert_eq!(
            change(&next, CHANNEL, "Encoder", &settings, Trigger::Rotate(127))
                .unwrap()
                .unwrap()
                .routes[0]
                .volume,
            100.0
        );
    }
    #[test]
    fn key_modes_and_dial_push_have_distinct_single_edges() {
        let before = config();
        let settings = json!({"mix":"monitor","mode":"set_volume","volume":12.5});
        assert_eq!(
            change(&before, MIX, "Keypad", &settings, Trigger::Down)
                .unwrap()
                .unwrap()
                .mixes[0]
                .volume,
            12.5
        );
        assert!(
            change(&before, MIX, "Keypad", &settings, Trigger::Up)
                .unwrap()
                .is_none()
        );
        assert!(
            change(&before, MIX, "Encoder", &settings, Trigger::Down)
                .unwrap()
                .is_none()
        );
        let next = change(&before, MIX, "Encoder", &settings, Trigger::Up)
            .unwrap()
            .unwrap();
        assert_eq!(next.mixes[0].volume, 80.0);
        assert!(next.mixes[0].muted);
        let next = change(&before, MIX, "Encoder", &settings, Trigger::Tap)
            .unwrap()
            .unwrap();
        assert!(next.mixes[0].muted);
        let next = change(
            &before,
            MIX,
            "Keypad",
            &json!({"mix":"monitor","mode":"change_volume","delta":-90}),
            Trigger::Down,
        )
        .unwrap()
        .unwrap();
        assert_eq!(next.mixes[0].volume, 0.0);
    }
    #[test]
    fn disabled_missing_and_invalid_targets_never_create_routes() {
        let mut before = config();
        before.enabled = false;
        let original = before.clone();
        assert!(
            change(
                &before,
                CHANNEL,
                "Encoder",
                &json!({"channel":"music","mix":"monitor"}),
                Trigger::Rotate(1)
            )
            .is_err()
        );
        assert_eq!(before, original);
        before.enabled = true;
        let original = before.clone();
        assert!(
            change(
                &before,
                CHANNEL,
                "Keypad",
                &json!({"channel":"music","mix":"stream"}),
                Trigger::Down
            )
            .is_err()
        );
        assert_eq!(before, original);
        assert!(
            change(
                &before,
                MIX,
                "Encoder",
                &json!({"mix":"monitor","step":0}),
                Trigger::Rotate(1)
            )
            .is_err()
        );
        assert!(
            change(
                &before,
                MIX,
                "Keypad",
                &json!({"mix":"monitor","mode":"mystery"}),
                Trigger::Down
            )
            .is_err()
        );
    }
    #[test]
    fn feedback_reports_applied_state_and_pending_or_error_honestly() {
        let mut desired = config();
        let settings = json!({"channel":"music","mix":"monitor"});
        let mut observed = Snapshot {
            config: desired.clone(),
            connected: true,
            active: true,
            ..Default::default()
        };
        desired.routes[0].volume = 60.0;
        desired.routes[0].muted = true;
        let state = display(&desired, &observed, CHANNEL, &settings);
        assert_eq!(state.volume, Some(40.0));
        assert_eq!(state.muted, Some(false));
        assert!(state.pending);
        assert!(state.value.contains("Applying"));
        observed.config = desired.clone();
        let state = display(&desired, &observed, CHANNEL, &settings);
        assert!(!state.pending);
        assert_eq!(state.value, "60% · Muted");
        observed.error = Some("Fixture backend failure".into());
        let state = display(&desired, &observed, CHANNEL, &settings);
        assert_eq!(state.volume, None);
        assert_eq!(state.error.as_deref(), Some("Fixture backend failure"));
        desired.enabled = false;
        assert_eq!(
            display(&desired, &observed, CHANNEL, &settings)
                .error
                .as_deref(),
            Some("Audio mixer is disabled")
        );
    }
    #[test]
    fn device_action_and_feedback_cannot_start_worker_or_enable_routing() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.mixer_config = config();
        let mut slot = host.slots[0].clone();
        slot.uuid = MIX.into();
        slot.settings = defaults(MIX).unwrap();
        let before = host.mixer_config.clone();
        assert!(host.mixer_input(&slot, Trigger::Down).is_err());
        assert!(host.mixer.is_none());
        assert_eq!(host.mixer_config, before);
        host.slots[0] = slot;
        host.update_mixer_feedback();
        assert!(host.mixer.is_none());
        assert_eq!(host.slots[0].feedback["value"], "Unavailable");
        host.mixer_config.enabled = false;
        let slot = host.slots[0].clone();
        assert!(host.mixer_input(&slot, Trigger::Down).is_err());
        assert!(!host.mixer_config.enabled);
        assert!(host.mixer.is_none());
    }
    #[test]
    fn definitions_and_defaults_use_only_original_identities() {
        for action in definitions() {
            assert!(action.uuid.starts_with("com.deckforge.mixer."));
            assert_eq!(action.controllers, vec!["Keypad", "Encoder"]);
            assert_eq!(defaults(&action.uuid).unwrap()["mode"], "toggle_mute");
        }
        assert!(defaults("com.elgato.wavelink.channel").is_none());
    }
}
