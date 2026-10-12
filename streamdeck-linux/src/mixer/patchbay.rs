//! Arbitrary audio connections. App streams are captured once before fan-out.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Endpoint {
    Source { name: String },
    App { selector: AppSelector },
    AllAudio,
    Channel { id: String },
    Mix { id: String },
    Output { name: String },
    Recording { mix: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Connection {
    pub id: String,
    pub from: Endpoint,
    pub to: Endpoint,
    pub source_channel: Option<String>,
    pub target_channel: Option<String>,
    pub volume: f64,
    pub muted: bool,
}
impl Default for Connection {
    fn default() -> Self {
        Self {
            id: String::new(),
            from: Endpoint::AllAudio,
            to: Endpoint::AllAudio,
            source_channel: None,
            target_channel: None,
            volume: 100.,
            muted: false,
        }
    }
}
fn node(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Recording { mix } => node(&Endpoint::Mix { id: mix.clone() }),
        _ => serde_json::to_string(endpoint).unwrap(),
    }
}
fn channel_name(value: &str) -> bool {
    matches!(
        value,
        "mono"
            | "front-left"
            | "front-right"
            | "front-center"
            | "rear-left"
            | "rear-right"
            | "rear-center"
            | "side-left"
            | "side-right"
            | "lfe"
            | "top-center"
            | "top-front-left"
            | "top-front-right"
            | "top-front-center"
            | "top-rear-left"
            | "top-rear-right"
            | "top-rear-center"
    ) || value
        .strip_prefix("aux")
        .is_some_and(|v| v.parse::<u8>().is_ok_and(|n| n < 32))
}
pub fn validate(config: &Config) -> Result<()> {
    ensure!(
        config.connections.len() <= 4096,
        "Too many patchbay connections"
    );
    let mut ids = BTreeSet::new();
    let mut edges = BTreeSet::new();
    for c in &config.connections {
        valid_id(&c.id)?;
        valid_volume(c.volume)?;
        ensure!(ids.insert(&c.id), "Duplicate patchbay connection ID");
        ensure!(
            matches!(
                c.to,
                Endpoint::Channel { .. } | Endpoint::Mix { .. } | Endpoint::Output { .. }
            ),
            "This audio node has no input port"
        );
        for endpoint in [&c.from, &c.to] {
            match endpoint {
                Endpoint::Source { name } | Endpoint::Output { name } => {
                    valid_external(name)?;
                    ensure!(
                        !name.starts_with('@') && !name.starts_with("deckforge."),
                        "Use a named external device or a typed mixer endpoint"
                    );
                }
                Endpoint::App { selector } => {
                    let value = match selector {
                        AppSelector::ApplicationId(v)
                        | AppSelector::ProcessBinary(v)
                        | AppSelector::DesktopApplication(v) => v,
                    };
                    valid_external(value)?;
                }
                Endpoint::Channel { id } => ensure!(
                    config.channels.iter().any(|c| &c.id == id),
                    "Patchbay refers to an unknown channel"
                ),
                Endpoint::Mix { id } | Endpoint::Recording { mix: id } => ensure!(
                    config.mixes.iter().any(|m| &m.id == id),
                    "Patchbay refers to an unknown mix"
                ),
                Endpoint::AllAudio => {}
            }
        }
        ensure!(
            c.source_channel.is_some() == c.target_channel.is_some(),
            "Split both connection endpoints to route individual audio channels"
        );
        for channel in [&c.source_channel, &c.target_channel].into_iter().flatten() {
            ensure!(channel_name(channel), "Unknown audio channel position");
        }
        ensure!(
            edges.insert((
                node(&c.from),
                node(&c.to),
                &c.source_channel,
                &c.target_channel
            )),
            "Duplicate patchbay connection"
        );
    }
    reject_cycles(config, &[])
}
pub fn reject_cycles(config: &Config, monitor_aliases: &[(String, String)]) -> Result<()> {
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // PipeWire schedules nodes, not isolated channel paths. Even L→R on the
    // same bus can stall a node cycle, so reject cycles at the node boundary.
    let mut edge = |from: Endpoint, to: Endpoint, _source: Option<&str>, _target: Option<&str>| {
        edges.entry(node(&from)).or_default().insert(node(&to));
    };
    for channel in &config.channels {
        for source in &channel.sources {
            edge(
                Endpoint::Source {
                    name: source.clone(),
                },
                Endpoint::Channel {
                    id: channel.id.clone(),
                },
                None,
                None,
            );
        }
    }
    for route in &config.routes {
        edge(
            Endpoint::Channel {
                id: route.channel.clone(),
            },
            Endpoint::Mix {
                id: route.mix.clone(),
            },
            None,
            None,
        );
    }
    for mix in &config.mixes {
        for output in &mix.outputs {
            edge(
                Endpoint::Mix { id: mix.id.clone() },
                Endpoint::Output {
                    name: output.clone(),
                },
                None,
                None,
            );
        }
    }
    if let (Some(mix), Some(output)) = (&config.monitoring.mix, &config.monitoring.output) {
        edge(
            Endpoint::Mix { id: mix.clone() },
            Endpoint::Output {
                name: output.clone(),
            },
            None,
            None,
        );
    }
    for c in &config.connections {
        edge(
            c.from.clone(),
            c.to.clone(),
            c.source_channel.as_deref(),
            c.target_channel.as_deref(),
        );
    }
    for (source, output) in monitor_aliases {
        edge(
            Endpoint::Output {
                name: output.clone(),
            },
            Endpoint::Source {
                name: source.clone(),
            },
            None,
            None,
        );
    }
    for start in edges.keys() {
        let mut pending = edges[start].iter().cloned().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        while let Some(next) = pending.pop() {
            ensure!(
                &next != start,
                "Audio connection would create a feedback loop"
            );
            if seen.insert(next.clone())
                && let Some(more) = edges.get(&next)
            {
                pending.extend(more.iter().cloned());
            }
        }
    }
    Ok(())
}
fn bus_id(endpoint: &Endpoint) -> String {
    let digest = Sha256::digest(serde_json::to_vec(endpoint).unwrap());
    format!("patch_{:x}", digest)[..30].into()
}
/// Keep legacy single-group app routing stable; introduce a shared bus only
/// when an app/default source fans out or connects directly to another node.
pub fn materialize(config: &Config) -> Result<Config> {
    let mut next = config.clone();
    let mut users: BTreeMap<Endpoint, Vec<String>> = BTreeMap::new();
    for channel in &config.channels {
        for selector in &channel.apps {
            users
                .entry(Endpoint::App {
                    selector: selector.clone(),
                })
                .or_default()
                .push(channel.id.clone());
        }
        if channel.capture_unassigned {
            users
                .entry(Endpoint::AllAudio)
                .or_default()
                .push(channel.id.clone());
        }
    }
    for c in &config.connections {
        if matches!(c.from, Endpoint::App { .. } | Endpoint::AllAudio) {
            users.entry(c.from.clone()).or_default();
        }
    }
    for (source, targets) in users {
        if targets.len() < 2 && !config.connections.iter().any(|c| c.from == source) {
            continue;
        }
        let id = bus_id(&source);
        ensure!(
            !next.channels.iter().any(|c| c.id == id),
            "Internal app capture bus conflicts with a channel ID"
        );
        for channel in &mut next.channels {
            match &source {
                Endpoint::App { selector } => channel.apps.retain(|s| s != selector),
                Endpoint::AllAudio => channel.capture_unassigned = false,
                _ => {}
            }
        }
        let selector = match &source {
            Endpoint::App { selector } => vec![selector.clone()],
            _ => vec![],
        };
        next.channels.push(Channel {
            id: id.clone(),
            name: "Application capture".into(),
            apps: selector,
            capture_unassigned: source == Endpoint::AllAudio,
            ..Default::default()
        });
        for c in &mut next.connections {
            if c.from == source {
                c.from = Endpoint::Channel { id: id.clone() };
            }
        }
        for (index, target) in targets.into_iter().enumerate() {
            let connection_id = format!("{id}_{index}");
            ensure!(
                !next.connections.iter().any(|c| c.id == connection_id),
                "Internal app route conflicts with a connection ID"
            );
            next.connections.push(Connection {
                id: connection_id,
                from: Endpoint::Channel { id: id.clone() },
                to: Endpoint::Channel { id: target },
                ..Default::default()
            });
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            channels: vec![
                Channel {
                    id: "a".into(),
                    name: "A".into(),
                    ..Default::default()
                },
                Channel {
                    id: "b".into(),
                    name: "B".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
    #[test]
    fn preserves_one_capture_but_fans_out_multiple_assignments() {
        let mut c = config();
        let app = AppSelector::ApplicationId("music".into());
        c.channels[0].apps.push(app.clone());
        assert_eq!(materialize(&c).unwrap(), c);
        c.channels[1].apps.push(app.clone());
        let built = materialize(&c).unwrap();
        assert!(built.channels[..2].iter().all(|c| c.apps.is_empty()));
        assert_eq!(built.channels[2].apps, vec![app]);
        assert_eq!(built.connections.len(), 2);
        assert_eq!(built.connections[0].from, built.connections[1].from);
    }
    #[test]
    fn validates_channel_connections_and_detects_actual_feedback() {
        let mut c = config();
        c.connections.push(Connection {
            id: "left_to_right".into(),
            from: Endpoint::Mix {
                id: "monitor".into(),
            },
            to: Endpoint::Mix {
                id: "monitor".into(),
            },
            source_channel: Some("front-left".into()),
            target_channel: Some("front-right".into()),
            ..Default::default()
        });
        assert!(
            validate(&c)
                .unwrap_err()
                .to_string()
                .contains("feedback loop")
        );
        c.connections[0].to = Endpoint::Mix {
            id: "stream".into(),
        };
        validate(&c).unwrap();
        let mut inverse = c.connections[0].clone();
        inverse.id = "right_to_left".into();
        std::mem::swap(&mut inverse.from, &mut inverse.to);
        std::mem::swap(&mut inverse.source_channel, &mut inverse.target_channel);
        c.connections.push(inverse);
        assert!(
            validate(&c)
                .unwrap_err()
                .to_string()
                .contains("feedback loop")
        );
    }
    #[test]
    fn output_monitor_cycle_is_rejected_before_routing() {
        let mut c = config();
        c.connections.push(Connection {
            id: "capture".into(),
            from: Endpoint::Source {
                name: "headphones.monitor".into(),
            },
            to: Endpoint::Output {
                name: "headphones".into(),
            },
            ..Default::default()
        });
        validate(&c).unwrap();
        assert!(reject_cycles(&c, &[("headphones.monitor".into(), "headphones".into())]).is_err());
    }
}
