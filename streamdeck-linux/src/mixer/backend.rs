#[path = "gain.rs"]
mod gain;
#[path = "journal.rs"]
mod journal;
#[path = "meter.rs"]
mod meter;
use super::*;
use anyhow::{Context as _, bail};
use libpulse_binding as pulse;
use pulse::{
    callbacks::ListResult,
    context::{Context, State as ContextState},
    mainloop::standard::{IterateResult, Mainloop},
    operation::{Operation, State as OperationState},
    volume::{ChannelVolumes, Volume, VolumeLinear},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::Instant,
};

const TIMEOUT: Duration = Duration::from_millis(900);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Module {
    id: u32,
    name: String,
    args: String,
}
#[derive(Clone)]
struct Sink {
    device: Device,
    index: u32,
    module: Option<u32>,
    channels: u8,
    volume: ChannelVolumes,
    muted: bool,
}
#[derive(Clone)]
struct Source {
    index: u32,
    device: Device,
    module: Option<u32>,
    monitor_sink: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Identity {
    index: u32,
    client: Option<u32>,
    process: Option<String>,
    binary: Option<String>,
    app: Option<String>,
    #[serde(default)]
    desktop_app: Option<String>,
    name: String,
}
impl Identity {
    // Desktop resolution is derived metadata. Installing/removing a launcher
    // must not turn a still-running audio stream into a different owner.
    fn same_stream(&self, other: &Self) -> bool {
        self.index == other.index
            && self.client == other.client
            && self.process == other.process
            && self.binary == other.binary
            && self.app == other.app
            && self.name == other.name
    }
}
#[derive(Clone)]
struct Input {
    app: App,
    identity: Identity,
    module: Option<u32>,
    channels: u8,
    volume: ChannelVolumes,
    muted: bool,
}
#[derive(Clone)]
struct Capture {
    client: Option<u32>,
    source: u32,
    module: Option<u32>,
}
#[derive(Clone, Default)]
struct Inventory {
    sinks: Vec<Sink>,
    sources: Vec<Source>,
    inputs: Vec<Input>,
    modules: Vec<Module>,
    captures: Vec<Capture>,
}
#[derive(Clone)]
struct Spec {
    key: String,
    name: String,
    args: String,
    sink: Option<String>,
    source: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Moved {
    identity: Identity,
    original: String,
    target: String,
}

pub(super) struct Engine {
    catalog: discovery::Catalog,
    catalog_at: Instant,
    namespace: String,
    session: String,
    journal: Option<journal::Journal>,
    connection: Option<Connection>,
    owned: Vec<(String, Module)>,
    pending_loads: BTreeMap<String, (String, String)>,
    moved: BTreeMap<u32, Moved>,
    yielded: BTreeSet<Identity>,
    last_members: Vec<(String, Vec<AppSelector>, bool)>,
    last_enabled: bool,
    paused: BTreeSet<String>,
    meters: BTreeMap<String, meter::Meter>,
    meter_keys: BTreeSet<String>,
    meter_client: Option<u32>,
    cache: Option<gain::Cache>,
}
impl Engine {
    pub fn new(namespace: String) -> Self {
        Self {
            catalog: discovery::Catalog::default(),
            catalog_at: Instant::now(),
            namespace,
            session: uuid::Uuid::new_v4().simple().to_string(),
            journal: None,
            connection: None,
            owned: vec![],
            pending_loads: BTreeMap::new(),
            moved: BTreeMap::new(),
            yielded: BTreeSet::new(),
            last_members: vec![],
            last_enabled: false,
            paused: BTreeSet::new(),
            meters: BTreeMap::new(),
            meter_keys: BTreeSet::new(),
            meter_client: None,
            cache: None,
        }
    }
    pub fn update(&mut self, config: &Config, stop: &AtomicBool) -> Snapshot {
        if Instant::now() >= self.catalog_at {
            self.catalog = discovery::Catalog::scan(stop);
            self.catalog_at = Instant::now() + Duration::from_secs(5);
        }
        let mut snapshot = Snapshot {
            applications: self.catalog.applications(&BTreeSet::new()),
            config: config.clone(),
            ..Snapshot::default()
        };
        self.cache = None;
        self.meter_keys = meter_keys(config);
        if !config.enabled {
            self.clear_meters();
        }
        let result = (|| -> Result<()> {
            if self.connection.is_none() {
                self.meters.clear();
                self.meter_client = None;
                self.connection = Some(Connection::new(stop)?);
            }
            let mut connection = self.connection.take().unwrap();
            connection.catalog = self.catalog.clone();
            self.meter_client = connection.context.get_index();
            let result = (|| -> Result<()> {
                if !config.enabled {
                    connection.close_meter_captures(stop)?;
                }
                let mut inv = connection.inventory(stop)?;
                if config.enabled {
                    self.recover(&inv)?;
                }
                self.owned = self
                    .owned
                    .iter()
                    .filter_map(|(key, m)| {
                        inv.modules
                            .iter()
                            .find(|actual| actual.name == m.name && actual.args == m.args)
                            .map(|actual| (key.clone(), actual.clone()))
                    })
                    .collect();
                self.adopt_pending(&inv);
                snapshot.connected = true;
                self.update_memberships(config);
                if config.enabled {
                    let targets = self.meter_targets(config, &inv);
                    self.trim_meters(&targets);
                    self.apply(
                        &mut connection,
                        config,
                        &mut inv,
                        &mut snapshot.warnings,
                        stop,
                    )?;
                } else {
                    self.cleanup(&mut connection, &mut inv, &mut snapshot.warnings, stop)?;
                }
                inv = connection.inventory(stop)?;
                let ids = self.verified_ids(&inv);
                snapshot.sinks = inv
                    .sinks
                    .iter()
                    .map(|s| {
                        let mut d = s.device.clone();
                        d.owned = s.module.is_some_and(|m| ids.contains(&m));
                        d
                    })
                    .collect();
                snapshot.sources = inv
                    .sources
                    .iter()
                    .map(|s| {
                        let mut d = s.device.clone();
                        d.owned = s.module.is_some_and(|m| ids.contains(&m));
                        d
                    })
                    .collect();
                snapshot.recording_sources = self.recording_sources(config, &inv);
                snapshot.apps = inv
                    .inputs
                    .iter()
                    .filter(|i| {
                        i.identity.client.is_some() && !i.module.is_some_and(|m| ids.contains(&m))
                    })
                    .map(|i| i.app.clone())
                    .collect();
                snapshot.applications = self.catalog.applications(
                    &snapshot
                        .apps
                        .iter()
                        .filter_map(|a| a.desktop_application.clone())
                        .collect(),
                );
                snapshot.active = config.enabled && !ids.is_empty();
                if config.enabled {
                    self.reconcile_meters(&mut connection, config, &inv, &mut snapshot.warnings);
                }
                snapshot.levels = self.levels(Instant::now());
                self.cache = Some(gain::Cache {
                    config: config.clone(),
                    snapshot: snapshot.clone(),
                    inventory: inv.clone(),
                });
                Ok(())
            })();
            let result = result.and(self.persist());
            if connection.context.get_state() == ContextState::Ready {
                self.connection = Some(connection);
            } else {
                self.clear_meters();
                self.meter_client = None;
                snapshot.connected = false;
            }
            result
        })();
        if let Err(error) = result {
            self.cache = None;
            snapshot.error = Some(format!("{error:#}"));
        }
        snapshot
    }
    /// Apply only changed levels after fresh targeted identity checks. Routing
    /// changes/uncertain ownership return None so the caller does full inventory.
    pub fn update_controls(&mut self, config: &Config, stop: &AtomicBool) -> Option<Snapshot> {
        let cached = self.cache.as_ref()?;
        if !control::same_routing(&cached.config, config) || stop.load(Ordering::Acquire) {
            return None;
        }
        let mut snapshot = cached.snapshot.clone();
        if let Err(error) = config.validate() {
            snapshot.error = Some(error.to_string());
            return Some(snapshot);
        }
        let changes = gain::changes(self, config, cached)?;
        let connection = self.connection.as_mut()?;
        if connection.ready(stop).is_err() {
            self.cache = None;
            return None;
        }
        if config.enabled {
            // Check every affected identity before starting any writes. Never
            // trust an old sink/input index across deletion or daemon restart.
            for change in &changes {
                if connection
                    .verify_gain(change, &self.namespace, &self.session, stop)
                    .ok()
                    != Some(true)
                {
                    self.cache = None;
                    return None;
                }
            }
            for change in &changes {
                if let Err(error) = connection.apply_gain(change, stop) {
                    snapshot.error = Some(format!("Mixer control update: {error:#}"));
                    snapshot.connected = connection.context.get_state() == ContextState::Ready;
                    self.cache = None;
                    return Some(snapshot);
                }
            }
        }
        snapshot.config = config.clone();
        snapshot.levels = self.levels(Instant::now());
        if let Some(cache) = &mut self.cache {
            cache.config = config.clone();
            cache.snapshot = snapshot.clone();
        }
        Some(snapshot)
    }
    /// Pump already-created meter streams only; no inventory or device lookup.
    pub fn poll_levels(&mut self, stop: &AtomicBool) -> Levels {
        if stop.load(Ordering::Acquire) || !self.last_enabled {
            self.clear_meters();
            return Levels::default();
        }
        if self
            .connection
            .as_mut()
            .is_none_or(|c| c.ready(stop).is_err())
        {
            self.cache = None;
            self.clear_meters();
            self.connection = None;
            self.meter_client = None;
        }
        self.levels(Instant::now())
    }
    fn levels(&mut self, now: Instant) -> Levels {
        let mut levels = Levels::default();
        for key in &self.meter_keys {
            let level = self
                .meters
                .get_mut(key)
                .map(|m| m.level(now))
                .unwrap_or_default();
            if let Some(id) = key.strip_prefix("channel:") {
                levels.channels.insert(id.into(), level);
            } else if let Some(id) = key.strip_prefix("mix:") {
                levels.mixes.insert(id.into(), level);
            }
        }
        levels
    }
    fn clear_meters(&mut self) {
        self.meters.clear(); // Disconnect before cleanup inventories endpoint readers.
    }
    fn trim_meters(&mut self, targets: &BTreeMap<String, meter::Target>) {
        let removed: Vec<_> = self
            .meters
            .iter()
            .filter(|(key, m)| targets.get(*key) != Some(&m.target) || m.failed())
            .map(|(key, _)| key.clone())
            .collect();
        for key in removed {
            self.meters.remove(&key);
        }
    }
    fn meter_targets(&self, config: &Config, inv: &Inventory) -> BTreeMap<String, meter::Target> {
        let mut targets = BTreeMap::new();
        for key in meter_keys(config) {
            let Some((_, module)) = self.owned.iter().find(|(owned, m)| {
                owned == &key
                    && m.name == "module-null-sink"
                    && inv.modules.iter().any(|actual| actual == m)
            }) else {
                continue;
            };
            let (kind, id) = key.split_once(':').unwrap();
            let name = if kind == "channel" {
                channel_sink(&self.namespace, id)
            } else {
                mix_sink(&self.namespace, id)
            };
            let Some(sink) = inv
                .sinks
                .iter()
                .find(|s| s.device.name == name && s.module == Some(module.id))
            else {
                continue;
            };
            let monitor = format!("{}.monitor", sink.device.name);
            let Some(source) = inv.sources.iter().find(|s| {
                s.device.name == monitor
                    && s.module == Some(module.id)
                    && s.device.monitor
                    && s.monitor_sink.as_deref() == Some(name.as_str())
            }) else {
                continue;
            };
            targets.insert(
                key,
                meter::Target {
                    source: source.index,
                    module: module.id,
                    name: monitor,
                },
            );
        }
        targets
    }
    fn reconcile_meters(
        &mut self,
        c: &mut Connection,
        config: &Config,
        inv: &Inventory,
        warnings: &mut Vec<String>,
    ) {
        let targets = self.meter_targets(config, inv);
        self.trim_meters(&targets);
        for (key, target) in targets {
            if self.meters.contains_key(&key) {
                continue;
            }
            match meter::Meter::new(&mut c.context, target, &key) {
                Ok(meter) => {
                    self.meters.insert(key, meter);
                }
                Err(error) => warnings.push(format!("Meter unavailable for {key}: {error}")),
            }
        }
    }
    fn recover(&mut self, inv: &Inventory) -> Result<()> {
        if self.journal.is_some() {
            return Ok(());
        }
        let (journal, saved) = journal::Journal::open(&self.namespace)?;
        if let Some(saved) = saved {
            self.session = saved.session;
            self.owned = saved
                .owned
                .into_iter()
                .filter_map(|(key, m)| {
                    inv.modules
                        .iter()
                        .find(|actual| actual.name == m.name && actual.args == m.args)
                        .map(|actual| (key, actual.clone()))
                })
                .collect();
            self.pending_loads = saved.pending;
            self.adopt_pending(inv);
            let ids = self.verified_ids(inv);
            self.moved = saved
                .moved
                .into_iter()
                .filter(|(_, m)| {
                    inv.inputs
                        .iter()
                        .any(|i| i.identity.same_stream(&m.identity) && i.app.sink == m.target)
                        && inv.sinks.iter().any(|s| {
                            s.device.name == m.target
                                && s.module.is_some_and(|id| ids.contains(&id))
                        })
                })
                .collect();
            self.paused = saved
                .paused
                .into_iter()
                .filter(|name| {
                    inv.sinks.iter().any(|s| {
                        s.device.name == *name && s.module.is_some_and(|id| ids.contains(&id))
                    })
                })
                .collect();
        }
        self.journal = Some(journal);
        self.persist()
    }
    fn persist(&mut self) -> Result<()> {
        if let Some(journal) = &mut self.journal {
            journal.write(&journal::Saved {
                version: 1,
                namespace: self.namespace.clone(),
                session: self.session.clone(),
                owned: self.owned.clone(),
                pending: self.pending_loads.clone(),
                moved: self.moved.clone(),
                paused: self.paused.clone(),
            })?;
        }
        Ok(())
    }
    fn recording_sources(&self, config: &Config, inv: &Inventory) -> BTreeMap<String, String> {
        let verified = self.verified_ids(inv);
        config
            .mixes
            .iter()
            .filter_map(|mix| {
                let name = mix_source(&self.namespace, &mix.id);
                let key = format!("export:{}", mix.id);
                inv.sources
                    .iter()
                    .find(|source| {
                        source.device.name == name
                            && source.module.is_some_and(|id| {
                                verified.contains(&id)
                                    && self
                                        .owned
                                        .iter()
                                        .any(|(owned, m)| owned == &key && m.id == id)
                            })
                    })
                    .map(|_| (mix.id.clone(), name))
            })
            .collect()
    }
    fn verified_ids(&self, inv: &Inventory) -> BTreeSet<u32> {
        self.owned
            .iter()
            .filter(|(_, owned)| inv.modules.iter().any(|m| m == owned))
            .map(|(_, m)| m.id)
            .collect()
    }
    fn adopt_pending(&mut self, inv: &Inventory) {
        for key in self.pending_loads.keys().cloned().collect::<Vec<_>>() {
            let (name, args) = &self.pending_loads[&key];
            if let Some(module) = inv
                .modules
                .iter()
                .find(|m| &m.name == name && &m.args == args)
            {
                if !self.owned.iter().any(|(_, m)| m == module) {
                    self.owned.push((key.clone(), module.clone()));
                }
                self.pending_loads.remove(&key);
            }
        }
    }
    fn plan(&self, config: &Config, inv: &Inventory, warnings: &mut Vec<String>) -> Vec<Spec> {
        let mut plan = vec![];
        let props = |description: &str| {
            quote(&format!(
                "device.description={} deckforge.owner={} deckforge.session={}",
                quote(description),
                quote(&self.namespace),
                quote(&self.session)
            ))
        };
        for (kind, id) in config
            .channels
            .iter()
            .map(|c| ("channel", &c.id))
            .chain(config.mixes.iter().map(|m| ("mix", &m.id)))
        {
            let name = if kind == "channel" {
                channel_sink(&self.namespace, id)
            } else {
                mix_sink(&self.namespace, id)
            };
            plan.push(Spec{key:format!("{kind}:{id}"),name:"module-null-sink".into(),args:format!("sink_name={} rate=48000 channels=2 channel_map=front-left,front-right sink_properties={}",quote(&name),props(&format!("Deckforge {kind} {id}"))),sink:Some(name),source:None});
        }
        for mix in &config.mixes {
            let source = mix_source(&self.namespace, &mix.id);
            let description = if mix.id == "monitor" {
                "Deckforge mix monitor (recording)".into()
            } else {
                format!("Deckforge mix {}", mix.id)
            };
            plan.push(Spec{key:format!("export:{}",mix.id),name:"module-remap-source".into(),args:format!("master={} source_name={} channels=2 channel_map=front-left,front-right source_properties={}",quote(&format!("{}.monitor",mix_sink(&self.namespace,&mix.id))),quote(&source),props(&description)),sink:None,source:Some(source)});
        }
        for r in &config.routes {
            plan.push(self.link(
                format!("route:{}:{}", r.channel, r.mix),
                format!("{}.monitor", channel_sink(&self.namespace, &r.channel)),
                mix_sink(&self.namespace, &r.mix),
            ));
        }
        for c in &config.channels {
            for source in &c.sources {
                if inv.sources.iter().any(|s| s.device.name == *source) {
                    plan.push(self.link(
                        format!("capture:{}:{source}", c.id),
                        source.clone(),
                        channel_sink(&self.namespace, &c.id),
                    ));
                } else {
                    warnings.push(format!(
                        "Source unavailable: {source}; channel {} remains unconnected to it",
                        c.name
                    ));
                }
            }
        }
        for m in &config.mixes {
            for sink in &m.outputs {
                if inv.sinks.iter().any(|s| s.device.name == *sink) {
                    plan.push(self.link(
                        format!("output:{}:{sink}", m.id),
                        format!("{}.monitor", mix_sink(&self.namespace, &m.id)),
                        sink.clone(),
                    ));
                } else {
                    warnings.push(format!(
                        "Output unavailable: {sink}; mix {} is not rerouted",
                        m.name
                    ));
                }
            }
        }
        if config.enabled
            && let (Some(mix), Some(output)) = (&config.monitoring.mix, &config.monitoring.output)
        {
            if inv.sinks.iter().any(|sink| sink.device.name == *output) {
                plan.push(self.link(
                    "listen:main".into(),
                    format!("{}.monitor", mix_sink(&self.namespace, mix)),
                    output.clone(),
                ));
            } else {
                warnings.push(format!(
                    "Listening output unavailable: {output}; no fallback output selected"
                ));
            }
        }
        plan
    }
    fn link(&self, key: String, source: String, sink: String) -> Spec {
        let props = quote(&format!(
            "media.name={} deckforge.owner={} deckforge.session={}",
            quote(&format!("Deckforge mixer {} {}", self.namespace, key)),
            quote(&self.namespace),
            quote(&self.session)
        ));
        Spec {
            key,
            name: "module-loopback".into(),
            args: format!(
                "source={} sink={} rate=48000 channels=2 channel_map=front-left,front-right latency_msec=30 source_dont_move=true sink_dont_move=true sink_input_properties={} source_output_properties={}",
                quote(&source),
                quote(&sink),
                props,
                props
            ),
            sink: None,
            source: None,
        }
    }
    fn apply(
        &mut self,
        c: &mut Connection,
        config: &Config,
        inv: &mut Inventory,
        warnings: &mut Vec<String>,
        stop: &AtomicBool,
    ) -> Result<()> {
        reject_monitor_cycles(config, inv)?;
        let plan = self.plan(config, inv, warnings);
        // Retain only identities still present; a new daemon can reuse indices.
        self.owned
            .retain(|(_, m)| inv.modules.iter().any(|actual| actual == m));
        let changed = graph_requires_pause(&plan, &self.owned, &self.protected_modules(inv));
        if changed {
            for sink in &inv.sinks {
                if sink
                    .module
                    .is_some_and(|m| self.verified_ids(inv).contains(&m))
                {
                    self.paused.insert(sink.device.name.clone());
                    self.persist()?;
                    c.suspend(&sink.device.name, true, stop)?;
                }
            }
        }
        // Restore only apps whose channel is being removed before removing nodes.
        let retained_channels: BTreeSet<String> = config
            .channels
            .iter()
            .map(|ch| channel_sink(&self.namespace, &ch.id))
            .collect();
        self.restore(c, inv, warnings, stop, |m| {
            !retained_channels.contains(&m.target)
                && !config.channels.iter().any(|ch| {
                    ch.apps
                        .iter()
                        .any(|selector| matches_app(selector, &m.identity))
                })
        })?;
        let protected = self.protected_modules(inv);
        for index in (0..self.owned.len()).rev() {
            let (key, m) = &self.owned[index];
            if !plan
                .iter()
                .any(|p| p.key == *key && p.name == m.name && p.args == m.args)
            {
                let is_protected = protected.contains(&m.id);
                if is_protected {
                    if let Some(sink) = inv.sinks.iter().find(|s| s.module == Some(m.id))
                        && !sink.muted
                    {
                        c.sink_mute(&sink.device.name, true, stop)?;
                    }
                    warnings.push("An app or external audio route still uses an owned endpoint; it is retained to prevent default-device fallback".into());
                    continue;
                }
                c.unload(m.id, stop)?;
                self.owned.remove(index);
            }
        }
        for spec in &plan {
            if self
                .owned
                .iter()
                .any(|(key, m)| key == &spec.key && m.name == spec.name && m.args == spec.args)
            {
                continue;
            }
            // Recover a successful load whose callback was lost. Full session-bound
            // arguments must match; names or recycled module IDs alone never suffice.
            let module = if let Some(m) = inv
                .modules
                .iter()
                .find(|m| m.name == spec.name && m.args == spec.args)
            {
                m.clone()
            } else {
                if let Some(name) = &spec.sink {
                    ensure!(
                        !inv.sinks.iter().any(|s| s.device.name == *name),
                        "Mixer sink name is already owned by another graph: {name}"
                    );
                }
                if let Some(name) = &spec.source {
                    ensure!(
                        !inv.sources.iter().any(|s| s.device.name == *name),
                        "Mixer source name is already owned by another graph: {name}"
                    );
                }
                self.pending_loads
                    .insert(spec.key.clone(), (spec.name.clone(), spec.args.clone()));
                self.persist()?;
                let id = c.load(&spec.name, &spec.args, stop)?;
                Module {
                    id,
                    name: spec.name.clone(),
                    args: spec.args.clone(),
                }
            };
            self.owned.push((spec.key.clone(), module));
            self.pending_loads.remove(&spec.key);
            self.persist()?;
            if let Some(sink) = &spec.sink {
                self.paused.insert(sink.clone());
                self.persist()?;
                c.suspend(sink, true, stop)?;
            }
        }
        *inv = c.inventory(stop)?;
        for route in &config.routes {
            let key = format!("route:{}:{}", route.channel, route.mix);
            let module = self
                .owned
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, m)| m.id)
                .context("Route module missing")?;
            let input = inv
                .inputs
                .iter()
                .find(|i| i.module == Some(module))
                .context("Route playback stream missing")?;
            if !gain_matches(&input.volume, route.volume) || input.muted != route.muted {
                c.input_gain(
                    input.app.index,
                    input.channels,
                    route.volume,
                    route.muted,
                    stop,
                )?;
            }
        }
        // Pulse/PipeWire may restore previous gains on newly created loopbacks.
        // Capture legs have unity gain. Output trims affect only the owned
        // loopback sink-input, never the destination sink/master volume.
        for (key, module) in &self.owned {
            if key.starts_with("capture:")
                || key.starts_with("output:")
                || key.starts_with("listen:")
            {
                let input = inv
                    .inputs
                    .iter()
                    .find(|i| i.module == Some(module.id))
                    .context("Owned capture/output stream missing")?;
                let (volume, muted) = output_gain(config, key);
                if !gain_matches(&input.volume, volume) || input.muted != muted {
                    c.input_gain(input.app.index, input.channels, volume, muted, stop)?;
                }
            }
        }
        for channel in &config.channels {
            let name = channel_sink(&self.namespace, &channel.id);
            let sink = inv
                .sinks
                .iter()
                .find(|s| s.device.name == name)
                .context("Owned channel missing")?;
            if sink.muted || !gain_matches(&sink.volume, 100.0) {
                c.sink_gain(&name, 100.0, false, stop)?;
            }
        }
        for mix in &config.mixes {
            let name = mix_sink(&self.namespace, &mix.id);
            let sink = inv
                .sinks
                .iter()
                .find(|s| s.device.name == name)
                .context("Mix sink missing")?;
            ensure!(sink.channels == 2, "Owned mix channel layout changed");
            if !gain_matches(&sink.volume, mix.volume) || sink.muted != mix.muted {
                c.sink_gain(&name, mix.volume, mix.muted, stop)?;
            }
        }
        // Routes are ready before any newly selected source/app can reach a mix.
        self.sync_apps(c, config, inv, warnings, stop)?;
        for sink in self.paused.clone() {
            if inv.sinks.iter().any(|s| {
                s.device.name == sink
                    && s.module
                        .is_some_and(|id| self.verified_ids(inv).contains(&id))
            }) {
                c.suspend(&sink, false, stop)?;
            }
            self.paused.remove(&sink);
        }
        Ok(())
    }
    fn update_memberships(&mut self, config: &Config) {
        let members = config
            .channels
            .iter()
            .map(|c| (c.id.clone(), c.apps.clone(), c.capture_unassigned))
            .collect::<Vec<_>>();
        if config.enabled && !self.last_enabled {
            // Re-enabling routing is an explicit request to take ownership again.
            self.yielded.clear();
        } else if members != self.last_members {
            self.yielded.retain(|identity| {
                let target = |entries: &[(String, Vec<AppSelector>, bool)]| {
                    selected_target(
                        entries
                            .iter()
                            .map(|(id, apps, default)| (id.as_str(), apps.as_slice(), *default)),
                        identity,
                    )
                    .map(|v| v.map(str::to_owned))
                };
                target(&self.last_members) == target(&members)
            });
        }
        self.last_members = members;
        self.last_enabled = config.enabled;
    }
    fn sync_apps(
        &mut self,
        c: &mut Connection,
        config: &Config,
        inv: &mut Inventory,
        warnings: &mut Vec<String>,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.restore(c, inv, warnings, stop, |m| {
            selected_channel(config, &m.identity).is_ok_and(|channel| channel.is_none())
        })?;
        let ids = self.verified_ids(inv);
        for input in &inv.inputs {
            if input.identity.client.is_none() || input.module.is_some_and(|m| ids.contains(&m)) {
                continue;
            }
            if let Some(previous) = self.moved.get(&input.app.index)
                && (!previous.identity.same_stream(&input.identity)
                    || input.app.sink != previous.target)
            {
                self.moved.remove(&input.app.index);
                self.yielded.insert(input.identity.clone());
            }
            if self
                .yielded
                .iter()
                .any(|identity| identity.same_stream(&input.identity))
            {
                continue;
            }
            let channel = match selected_channel(config, &input.identity) {
                Ok(Some(channel)) => channel,
                Ok(None) => continue,
                Err(()) => {
                    warnings.push(format!(
                        "App {} matches more than one channel; no routing changed",
                        input.app.name
                    ));
                    continue;
                }
            };
            let target = channel_sink(&self.namespace, &channel.id);
            if default_must_yield(channel, input, &target, &self.moved, inv, &ids) {
                continue;
            }
            if input.app.sink == target {
                continue;
            }
            ensure!(
                inv.sinks.iter().any(
                    |s| s.device.name == target && s.module.is_some_and(|id| ids.contains(&id))
                ),
                "Owned target sink missing"
            );
            let original = self
                .moved
                .get(&input.app.index)
                .map(|m| m.original.clone())
                .unwrap_or_else(|| input.app.sink.clone());
            if original.is_empty() {
                warnings.push(format!(
                    "Cannot identify original output for {}",
                    input.app.name
                ));
                continue;
            }
            // Record before dispatch so a timed-out acknowledgement is still reversible.
            self.moved.insert(
                input.app.index,
                Moved {
                    identity: input.identity.clone(),
                    original,
                    target: target.clone(),
                },
            );
            self.persist()?;
            c.move_app(input.app.index, &target, stop)?;
        }
        *inv = c.inventory(stop)?;
        Ok(())
    }
    fn restore(
        &mut self,
        c: &mut Connection,
        inv: &mut Inventory,
        warnings: &mut Vec<String>,
        stop: &AtomicBool,
        selected: impl Fn(&Moved) -> bool,
    ) -> Result<()> {
        for id in self.moved.keys().copied().collect::<Vec<_>>() {
            let moved = self.moved.get(&id).unwrap().clone();
            if !selected(&moved) {
                continue;
            }
            let Some(current) = inv
                .inputs
                .iter()
                .find(|i| i.app.index == id && i.identity.same_stream(&moved.identity))
            else {
                self.moved.remove(&id);
                continue;
            };
            if current.app.sink != moved.target {
                self.moved.remove(&id);
                continue;
            }
            let owned = self.verified_ids(inv);
            if !inv.sinks.iter().any(|s| {
                s.device.name == moved.target && s.module.is_some_and(|m| owned.contains(&m))
            }) {
                self.moved.remove(&id);
                continue;
            }
            if !inv.sinks.iter().any(|s| s.device.name == moved.original) {
                warnings.push(format!(
                    "Original output {} is unavailable; {} is kept on its owned channel",
                    moved.original, current.app.name
                ));
                continue;
            }
            c.move_app(id, &moved.original, stop)?;
            self.moved.remove(&id);
        }
        *inv = c.inventory(stop)?;
        Ok(())
    }
    fn protected_modules(&self, inv: &Inventory) -> BTreeSet<u32> {
        let owned = self.verified_ids(inv);
        let mut protected = BTreeSet::new();
        for input in &inv.inputs {
            if !input.module.is_some_and(|id| owned.contains(&id))
                && let Some(id) = inv
                    .sinks
                    .iter()
                    .find(|s| s.device.name == input.app.sink)
                    .and_then(|s| s.module)
                    .filter(|id| owned.contains(id))
            {
                protected.insert(id);
            }
        }
        for capture in &inv.captures {
            // This private context creates only meter recording streams. Its
            // server-assigned client identity also covers streams still Creating.
            if self.meter_client.is_some() && capture.client == self.meter_client {
                continue;
            }
            if capture.module.is_some_and(|id| owned.contains(&id)) {
                continue;
            }
            if let Some(id) = inv
                .sources
                .iter()
                .find(|s| s.index == capture.source)
                .and_then(|s| s.module)
                .filter(|id| owned.contains(id))
            {
                protected.insert(id);
                if let Some(mix) = self
                    .owned
                    .iter()
                    .find(|(_, m)| m.id == id)
                    .and_then(|(key, _)| key.strip_prefix("export:"))
                    && let Some((_, module)) = self
                        .owned
                        .iter()
                        .find(|(key, _)| key == &format!("mix:{mix}"))
                {
                    protected.insert(module.id);
                }
            }
        }
        protected
    }
    fn cleanup(
        &mut self,
        c: &mut Connection,
        inv: &mut Inventory,
        warnings: &mut Vec<String>,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.restore(c, inv, warnings, stop, |_| true)?;
        let protected = self.protected_modules(inv);
        if !protected.is_empty() {
            warnings.push(
                "Owned endpoints remain silent while an app or external audio route is attached"
                    .into(),
            );
        }
        for sink in &inv.sinks {
            if sink.module.is_some_and(|id| protected.contains(&id)) && !sink.muted {
                c.sink_mute(&sink.device.name, true, stop)?;
            }
        }

        for index in (0..self.owned.len()).rev() {
            let (_, m) = &self.owned[index];
            if !inv.modules.iter().any(|actual| actual == m) {
                self.owned.remove(index);
                continue;
            }
            if protected.contains(&m.id) {
                continue;
            }
            c.unload(m.id, stop)?;
            self.owned.remove(index);
        }
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.cache = None;
        self.clear_meters();
        if let Some(mut c) = self.connection.take() {
            c.deadline = Some(Instant::now() + Duration::from_secs(2));
            let stop = AtomicBool::new(false);
            if c.close_meter_captures(&stop).is_ok()
                && let Ok(mut inv) = c.inventory(&stop)
            {
                self.adopt_pending(&inv);
                let _ = self.cleanup(&mut c, &mut inv, &mut vec![], &stop);
            }
        }
        let _ = self.persist();
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
/// A listening-only switch replaces its one loopback without interrupting the
/// independent mix buses, fixed outputs, recorders or active peak streams.
/// Adding/removing a read-only export alias likewise needs no bus suspension.
fn graph_requires_pause(
    plan: &[Spec],
    owned: &[(String, Module)],
    protected: &BTreeSet<u32>,
) -> bool {
    plan.iter()
        .filter(|p| !p.key.starts_with("listen:") && !p.key.starts_with("export:"))
        .any(|p| {
            !owned
                .iter()
                .any(|(key, m)| key == &p.key && m.name == p.name && m.args == p.args)
        })
        || owned
            .iter()
            .filter(|(key, _)| !key.starts_with("listen:") && !key.starts_with("export:"))
            .any(|(key, m)| !plan.iter().any(|p| &p.key == key) && !protected.contains(&m.id))
}
fn meter_keys(config: &Config) -> BTreeSet<String> {
    if !config.enabled {
        return BTreeSet::new();
    }
    config
        .channels
        .iter()
        .map(|c| format!("channel:{}", c.id))
        .chain(config.mixes.iter().map(|m| format!("mix:{}", m.id)))
        .collect()
}
fn output_gain(config: &Config, key: &str) -> (f64, bool) {
    for mix in &config.mixes {
        for output in &mix.outputs {
            if key == format!("output:{}:{output}", mix.id) {
                return mix
                    .output_controls
                    .get(output)
                    .map(|c| (c.volume, c.muted))
                    .unwrap_or((100.0, false));
            }
        }
    }
    (100.0, false)
}
fn gain_matches(volumes: &ChannelVolumes, percent: f64) -> bool {
    let wanted = Volume::from(VolumeLinear(percent / 100.0));
    !volumes.get().is_empty() && volumes.get().iter().all(|v| *v == wanted)
}
fn reject_monitor_cycles(config: &Config, inv: &Inventory) -> Result<()> {
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for route in &config.routes {
        let channel = config
            .channels
            .iter()
            .find(|c| c.id == route.channel)
            .context("Unknown channel")?;
        let mix = config
            .mixes
            .iter()
            .find(|m| m.id == route.mix)
            .context("Unknown mix")?;
        for source in &channel.sources {
            if let Some(sink) = inv
                .sources
                .iter()
                .find(|s| s.device.name == *source)
                .and_then(|s| s.monitor_sink.as_ref())
            {
                edges
                    .entry(sink.clone())
                    .or_default()
                    .extend(mix.outputs.iter().cloned());
                if config.monitoring.mix.as_ref() == Some(&mix.id)
                    && let Some(output) = &config.monitoring.output
                {
                    edges
                        .entry(sink.clone())
                        .or_default()
                        .insert(output.clone());
                }
            }
        }
    }
    for start in edges.keys() {
        let mut pending = edges[start].iter().cloned().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        while let Some(node) = pending.pop() {
            ensure!(
                &node != start,
                "Audio routing would feed a sink monitor back into itself"
            );
            if seen.insert(node.clone())
                && let Some(next) = edges.get(&node)
            {
                pending.extend(next.iter().cloned());
            }
        }
    }
    Ok(())
}
/// Explicit assignments win. Ambiguous explicit selectors never become catch-all.
fn selected_channel<'a>(
    config: &'a Config,
    identity: &Identity,
) -> std::result::Result<Option<&'a Channel>, ()> {
    let target = selected_target(
        config
            .channels
            .iter()
            .map(|ch| (ch.id.as_str(), ch.apps.as_slice(), ch.capture_unassigned)),
        identity,
    )?;
    Ok(target.and_then(|id| config.channels.iter().find(|ch| ch.id == id)))
}
fn selected_target<'a>(
    channels: impl Iterator<Item = (&'a str, &'a [AppSelector], bool)>,
    identity: &Identity,
) -> std::result::Result<Option<&'a str>, ()> {
    let mut selected = None;
    let mut fallback = None;
    for (id, apps, default) in channels {
        if default {
            fallback = Some(id);
        }
        if apps.iter().any(|s| matches_app(s, identity)) {
            if selected.is_some() {
                return Err(());
            }
            selected = Some(id);
        }
    }
    Ok(selected.or(fallback))
}

fn default_must_yield(
    channel: &Channel,
    input: &Input,
    target: &str,
    moved: &BTreeMap<u32, Moved>,
    inv: &Inventory,
    owned: &BTreeSet<u32>,
) -> bool {
    // A direct route to an owned channel is already an intentional assignment.
    // Our own previously moved stream can still follow a changed default group.
    channel.capture_unassigned
        && !channel.apps.iter().any(|s| matches_app(s, &input.identity))
        && input.app.sink != target
        && !moved.contains_key(&input.app.index)
        && inv.sinks.iter().any(|sink| {
            sink.device.name == input.app.sink && sink.module.is_some_and(|id| owned.contains(&id))
        })
}
fn matches_app(selector: &AppSelector, identity: &Identity) -> bool {
    match selector {
        AppSelector::ApplicationId(v) => identity.app.as_ref() == Some(v),
        AppSelector::ProcessBinary(v) => identity.binary.as_ref() == Some(v),
        AppSelector::DesktopApplication(v) => identity.desktop_app.as_ref() == Some(v),
    }
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
struct Connection {
    catalog: discovery::Catalog,
    context: Context,
    mainloop: Mainloop,
    deadline: Option<Instant>,
}
impl Connection {
    fn new(stop: &AtomicBool) -> Result<Self> {
        let mainloop = Mainloop::new().context("Create mixer event loop")?;
        let mut context =
            Context::new(&mainloop, "Deckforge mixer").context("Create mixer context")?;
        context
            .connect(None, pulse::context::FlagSet::NOAUTOSPAWN, None)
            .context("Connect to audio server")?;
        let mut c = Self {
            catalog: discovery::Catalog::default(),
            context,
            mainloop,
            deadline: None,
        };
        let end = Instant::now() + Duration::from_millis(1500);
        while c.context.get_state() != ContextState::Ready {
            ensure!(
                !stop.load(Ordering::Acquire) && Instant::now() < end,
                "Audio server connection cancelled/timed out"
            );
            c.iterate()?;
            thread::sleep(Duration::from_millis(2));
        }
        Ok(c)
    }
    fn iterate(&mut self) -> Result<()> {
        for _ in 0..16 {
            match self.mainloop.iterate(false) {
                IterateResult::Success(0) => break,
                IterateResult::Success(_) => {}
                _ => bail!("Audio event loop stopped"),
            }
        }
        ensure!(
            !matches!(
                self.context.get_state(),
                ContextState::Failed | ContextState::Terminated
            ),
            "Audio server disconnected"
        );
        Ok(())
    }
    fn ready(&mut self, stop: &AtomicBool) -> Result<()> {
        ensure!(
            !stop.load(Ordering::Acquire) && self.deadline.is_none_or(|d| Instant::now() < d),
            "Mixer operation cancelled"
        );
        self.iterate()?;
        ensure!(
            self.context.get_state() == ContextState::Ready,
            "Audio server not ready"
        );
        Ok(())
    }
    fn wait<T: ?Sized>(&mut self, op: &mut Operation<T>, stop: &AtomicBool) -> Result<()> {
        let end = (Instant::now() + TIMEOUT).min(self.deadline.unwrap_or(Instant::now() + TIMEOUT));
        while op.get_state() == OperationState::Running {
            if stop.load(Ordering::Acquire) || Instant::now() >= end {
                op.cancel();
                bail!("Mixer audio operation cancelled/timed out");
            }
            if let Err(e) = self.iterate() {
                op.cancel();
                return Err(e);
            }
            if op.get_state() == OperationState::Running {
                thread::sleep(Duration::from_millis(2));
            }
        }
        ensure!(
            op.get_state() == OperationState::Done,
            "Mixer audio operation cancelled"
        );
        Ok(())
    }
    /// Disconnect acknowledgements can lag Stream::drop, particularly while a
    /// recording stream is Creating. Confirm that this private client's captures
    /// are gone before unloading their monitor sinks. Never kill another client.
    fn close_meter_captures(&mut self, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let client = self
            .context
            .get_index()
            .context("Missing mixer client identity")?;
        let previous = self.deadline;
        self.deadline = Some(
            (Instant::now() + Duration::from_millis(500))
                .min(previous.unwrap_or(Instant::now() + Duration::from_millis(500))),
        );
        let result = (|| {
            loop {
                self.ready(stop)?;
                let indices = Rc::new(RefCell::new(Vec::new()));
                let to = indices.clone();
                let failed = Rc::new(RefCell::new(false));
                let err = failed.clone();
                let mut op =
                    self.context
                        .introspect()
                        .get_source_output_info_list(move |r| match r {
                            ListResult::Item(i) if i.client == Some(client) => {
                                to.borrow_mut().push(i.index)
                            }
                            ListResult::Error => *err.borrow_mut() = true,
                            _ => {}
                        });
                self.wait(&mut op, stop)?;
                ensure!(!*failed.borrow(), "Could not verify closed mixer meters");
                let indices = indices.take();
                if indices.is_empty() {
                    return Ok(());
                }
                for index in indices {
                    // A stream may have finished its own disconnect since the
                    // query. Re-querying, not the kill acknowledgement, proves closure.
                    let mut op = self.context.introspect().kill_source_output(index, |_| {});
                    self.wait(&mut op, stop)?;
                }
            }
        })();
        self.deadline = previous;
        result
    }
    fn inventory(&mut self, stop: &AtomicBool) -> Result<Inventory> {
        self.ready(stop)?;
        let modules = Rc::new(RefCell::new(Vec::new()));
        let to = modules.clone();
        let error = Rc::new(RefCell::new(false));
        let err = error.clone();
        let mut op = self
            .context
            .introspect()
            .get_module_info_list(move |r| match r {
                ListResult::Item(i) => to.borrow_mut().push(Module {
                    id: i.index,
                    name: i.name.as_deref().unwrap_or("").into(),
                    args: i.argument.as_deref().unwrap_or("").into(),
                }),
                ListResult::Error => *err.borrow_mut() = true,
                _ => {}
            });
        self.wait(&mut op, stop)?;
        let sinks = Rc::new(RefCell::new(Vec::new()));
        let to = sinks.clone();
        let err = error.clone();
        self.ready(stop)?;
        let mut op = self
            .context
            .introspect()
            .get_sink_info_list(move |r| match r {
                ListResult::Item(i) => to.borrow_mut().push(Sink {
                    device: Device {
                        muted: i.mute,
                        name: i.name.as_deref().unwrap_or("").into(),
                        description: i.description.as_deref().unwrap_or("").into(),
                        owned: false,
                        monitor: false,
                    },
                    index: i.index,
                    module: i.owner_module,
                    channels: i.sample_spec.channels,
                    volume: i.volume,
                    muted: i.mute,
                }),
                ListResult::Error => *err.borrow_mut() = true,
                _ => {}
            });
        self.wait(&mut op, stop)?;
        let sources = Rc::new(RefCell::new(Vec::new()));
        let to = sources.clone();
        let err = error.clone();
        self.ready(stop)?;
        let mut op = self
            .context
            .introspect()
            .get_source_info_list(move |r| match r {
                ListResult::Item(i) => to.borrow_mut().push(Source {
                    index: i.index,
                    device: Device {
                        muted: i.mute,
                        name: i.name.as_deref().unwrap_or("").into(),
                        description: i.description.as_deref().unwrap_or("").into(),
                        owned: false,
                        monitor: i.monitor_of_sink.is_some(),
                    },
                    module: i.owner_module,
                    monitor_sink: i.monitor_of_sink_name.as_ref().map(ToString::to_string),
                }),
                ListResult::Error => *err.borrow_mut() = true,
                _ => {}
            });
        self.wait(&mut op, stop)?;
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let to = inputs.clone();
        let err = error.clone();
        let lookup: BTreeMap<u32, String> = sinks
            .borrow()
            .iter()
            .map(|s| (s.index, s.device.name.clone()))
            .collect();
        self.ready(stop)?;
        let catalog = self.catalog.clone();
        let mut op = self
            .context
            .introspect()
            .get_sink_input_info_list(move |r| match r {
                ListResult::Item(i) => {
                    let name = i
                        .proplist
                        .get_str("application.name")
                        .unwrap_or_else(|| i.name.as_deref().unwrap_or("Audio stream").into());
                    let app = i.proplist.get_str("application.id");
                    let binary = i.proplist.get_str("application.process.binary");
                    let process = i.proplist.get_str("application.process.id");
                    let desktop = i.proplist.get_str("application.desktop");
                    let desktop_app = catalog.resolve(
                        app.as_deref(),
                        desktop.as_deref(),
                        binary.as_deref(),
                        process.as_deref(),
                        &name,
                    );
                    to.borrow_mut().push(Input {
                        app: App {
                            index: i.index,
                            name: name.clone(),
                            application_id: app.clone(),
                            process_binary: binary.clone(),
                            desktop_application: desktop_app.clone(),
                            sink: lookup.get(&i.sink).cloned().unwrap_or_default(),
                        },
                        identity: Identity {
                            index: i.index,
                            client: i.client,
                            process,
                            desktop_app,
                            binary,
                            app,
                            name: i.name.as_deref().unwrap_or("").into(),
                        },
                        module: i.owner_module,
                        channels: i.sample_spec.channels,
                        volume: i.volume,
                        muted: i.mute,
                    });
                }
                ListResult::Error => *err.borrow_mut() = true,
                _ => {}
            });
        self.wait(&mut op, stop)?;
        let captures = Rc::new(RefCell::new(Vec::new()));
        let to = captures.clone();
        let err = error.clone();
        self.ready(stop)?;
        let mut op = self
            .context
            .introspect()
            .get_source_output_info_list(move |r| match r {
                ListResult::Item(i) => to.borrow_mut().push(Capture {
                    client: i.client,
                    source: i.source,
                    module: i.owner_module,
                }),
                ListResult::Error => *err.borrow_mut() = true,
                _ => {}
            });
        self.wait(&mut op, stop)?;
        ensure!(!*error.borrow(), "Audio inventory query failed");
        Ok(Inventory {
            sinks: sinks.take(),
            sources: sources.take(),
            inputs: inputs.take(),
            modules: modules.take(),
            captures: captures.take(),
        })
    }
    fn load(&mut self, name: &str, args: &str, stop: &AtomicBool) -> Result<u32> {
        self.ready(stop)?;
        let value = Rc::new(RefCell::new(None));
        let to = value.clone();
        let mut op = self
            .context
            .introspect()
            .load_module(name, args, move |id| *to.borrow_mut() = Some(id));
        self.wait(&mut op, stop)?;
        let id = value.borrow().context("Missing module acknowledgement")?;
        ensure!(id != u32::MAX, "Audio server could not load {name}");
        Ok(id)
    }
    fn unload(&mut self, id: u32, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self
            .context
            .introspect()
            .unload_module(id, move |v| *to.borrow_mut() = v);
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not unload owned audio module");
        Ok(())
    }
    fn suspend(&mut self, name: &str, suspended: bool, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self.context.introspect().suspend_sink_by_name(
            name,
            suspended,
            Some(Box::new(move |v| *to.borrow_mut() = v)),
        );
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not stage owned audio sink");
        Ok(())
    }
    fn move_app(&mut self, id: u32, sink: &str, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self.context.introspect().move_sink_input_by_name(
            id,
            sink,
            Some(Box::new(move |v| *to.borrow_mut() = v)),
        );
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not move selected application stream");
        Ok(())
    }
    fn input_gain(
        &mut self,
        id: u32,
        channels: u8,
        percent: f64,
        muted: bool,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.ready(stop)?;
        if muted {
            self.input_mute(id, true, stop)?;
        }
        let mut volume = ChannelVolumes::default();
        volume.set(channels, Volume::from(VolumeLinear(percent / 100.0)));
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self.context.introspect().set_sink_input_volume(
            id,
            &volume,
            Some(Box::new(move |v| *to.borrow_mut() = v)),
        );
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not set route gain");
        if !muted {
            self.input_mute(id, false, stop)?;
        }
        Ok(())
    }
    fn input_mute(&mut self, id: u32, muted: bool, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self.context.introspect().set_sink_input_mute(
            id,
            muted,
            Some(Box::new(move |v| *to.borrow_mut() = v)),
        );
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not set route mute");
        Ok(())
    }
    fn sink_gain(
        &mut self,
        name: &str,
        percent: f64,
        muted: bool,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.ready(stop)?;
        if muted {
            self.sink_mute(name, true, stop)?;
        }
        let mut volume = ChannelVolumes::default();
        volume.set(2, Volume::from(VolumeLinear(percent / 100.0)));
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self.context.introspect().set_sink_volume_by_name(
            name,
            &volume,
            Some(Box::new(move |v| *to.borrow_mut() = v)),
        );
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not set mix gain");
        if !muted {
            self.sink_mute(name, false, stop)?;
        }
        Ok(())
    }
    fn sink_mute(&mut self, name: &str, muted: bool, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let ok = Rc::new(RefCell::new(false));
        let to = ok.clone();
        let mut op = self.context.introspect().set_sink_mute_by_name(
            name,
            muted,
            Some(Box::new(move |v| *to.borrow_mut() = v)),
        );
        self.wait(&mut op, stop)?;
        ensure!(*ok.borrow(), "Could not set mix mute");
        Ok(())
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.context.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quote_preserves_spaces_and_cannot_create_arguments() {
        assert_eq!(quote("Microphone FX"), "\"Microphone FX\"");
        assert_eq!(quote("x\" sink=other \\"), "\"x\\\" sink=other \\\\\"");
    }
    #[test]
    fn recycled_module_ids_do_not_grant_ownership() {
        let mut e = Engine::new("fixture".into());
        let own = Module {
            id: 9,
            name: "module-null-sink".into(),
            args: "session=ours".into(),
        };
        e.owned.push(("mix:a".into(), own));
        let inv = Inventory {
            modules: vec![Module {
                id: 9,
                name: "module-null-sink".into(),
                args: "session=other".into(),
            }],
            ..Inventory::default()
        };
        assert!(e.verified_ids(&inv).is_empty());
    }
    #[test]
    fn interrupted_load_adopts_only_its_exact_session_arguments() {
        let mut e = Engine::new("fixture".into());
        e.pending_loads.insert(
            "mix:a".into(),
            ("module-null-sink".into(), "session=ours".into()),
        );
        let mut inv = Inventory {
            modules: vec![Module {
                id: 9,
                name: "module-null-sink".into(),
                args: "session=other".into(),
            }],
            ..Inventory::default()
        };
        e.adopt_pending(&inv);
        assert!(e.owned.is_empty());
        inv.modules[0].args = "session=ours".into();
        e.adopt_pending(&inv);
        assert_eq!(e.owned.len(), 1);
        assert!(e.pending_loads.is_empty());
    }
    #[test]
    #[ignore = "Creates empty owned virtual graph only; requires DECKFORGE_MIXER_VIRTUAL_TEST=1"]
    fn cancellation_during_graph_creation_cleans_up() {
        if std::env::var("DECKFORGE_MIXER_VIRTUAL_TEST").as_deref() != Ok("1") {
            return;
        }
        let stop = AtomicBool::new(false);
        let mut observer = Connection::new(&stop).unwrap();
        for _ in 0..3 {
            let namespace = format!("cancel{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
            let mut config = Config {
                enabled: true,
                channels: (0..12)
                    .map(|i| Channel {
                        id: format!("c{i}"),
                        name: format!("Channel {i}"),
                        ..Channel::default()
                    })
                    .collect(),
                mixes: (0..5)
                    .map(|i| Mix {
                        id: format!("m{i}"),
                        name: format!("Mix {i}"),
                        ..Mix::default()
                    })
                    .collect(),
                routes: vec![],
                ..Config::default()
            };
            for ch in &config.channels {
                for mix in &config.mixes {
                    config.routes.push(Route {
                        channel: ch.id.clone(),
                        mix: mix.id.clone(),
                        ..Route::default()
                    });
                }
            }
            let worker = Worker::new(namespace.clone(), config).unwrap();
            let prefix = format!("deckforge.{namespace}.");
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if observer
                    .inventory(&stop)
                    .unwrap()
                    .sinks
                    .iter()
                    .any(|s| s.device.name.starts_with(&prefix))
                {
                    break;
                }
                assert!(Instant::now() < deadline, "Graph did not start");
            }
            assert!(
                !worker.snapshot().active,
                "Fixture must cancel during startup"
            );
            let start = Instant::now();
            drop(worker);
            assert!(start.elapsed() < Duration::from_secs(3));
            let leftover: Vec<Module> = observer
                .inventory(&stop)
                .unwrap()
                .modules
                .into_iter()
                .filter(|m| m.args.contains(&prefix))
                .collect();
            // Best-effort cleanup of positively identified test nodes even on a
            // failed assertion. Never clean unrelated modules by name alone.
            for module in leftover.iter().rev() {
                let _ = observer.unload(module.id, &stop);
            }
            assert!(
                leftover.is_empty(),
                "Cancelled startup leaked owned modules"
            );
        }
    }
    #[test]
    fn disabled_plan_is_never_needed_and_missing_devices_are_not_defaulted() {
        let e = Engine::new("fixture".into());
        let mut c = Config::default();
        c.channels.push(Channel {
            id: "mic".into(),
            name: "Mic".into(),
            sources: vec!["Missing Mic".into()],
            apps: vec![],
            ..Channel::default()
        });
        c.mixes[0].outputs.push("Missing Output".into());
        let mut warnings = vec![];
        let p = e.plan(&c, &Inventory::default(), &mut warnings);
        assert_eq!(warnings.len(), 2);
        assert!(
            !p.iter()
                .any(|s| s.key.starts_with("capture:") || s.key.starts_with("output:"))
        );
        assert!(p.iter().all(|s| !s.args.contains("@DEFAULT")));
    }
    #[test]
    fn selectors_match_exact_fields_without_name_fallback() {
        let id = Identity {
            index: 1,
            client: Some(1),
            process: Some("5".into()),
            binary: Some("discord".into()),
            app: Some("com.discord.App".into()),
            desktop_app: Some("com.discord.App.desktop".into()),
            name: "Discord".into(),
        };
        assert!(matches_app(
            &AppSelector::ApplicationId("com.discord.App".into()),
            &id
        ));
        assert!(!matches_app(
            &AppSelector::ProcessBinary("Discord".into()),
            &id
        ));
    }
    #[test]
    fn default_group_yields_to_explicit_assignments_and_ambiguity() {
        let identity = Identity {
            index: 1,
            client: Some(2),
            process: None,
            binary: Some("player".into()),
            app: None,
            desktop_app: Some("player.desktop".into()),
            name: "Stream".into(),
        };
        let mut config = Config {
            channels: vec![
                Channel {
                    id: "default".into(),
                    name: "Other apps".into(),
                    capture_unassigned: true,
                    ..Default::default()
                },
                Channel {
                    id: "music".into(),
                    name: "Music".into(),
                    apps: vec![AppSelector::DesktopApplication("player.desktop".into())],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        config.validate().unwrap();
        assert_eq!(
            selected_channel(&config, &identity).unwrap().unwrap().id,
            "music"
        );
        let mut unmatched = identity.clone();
        unmatched.desktop_app = None;
        assert_eq!(
            selected_channel(&config, &unmatched).unwrap().unwrap().id,
            "default"
        );
        config.channels.push(Channel {
            id: "overlap".into(),
            name: "Overlap".into(),
            apps: vec![AppSelector::ProcessBinary("player".into())],
            ..Default::default()
        });
        assert!(selected_channel(&config, &identity).is_err());
        config.channels[2].capture_unassigned = true;
        assert!(config.validate().is_err());
        config.channels[0].capture_unassigned = false;
        config.channels[2].capture_unassigned = false;
        config.channels[2].apps.clear();
        assert!(selected_channel(&config, &unmatched).unwrap().is_none());
    }
    #[test]
    fn membership_changes_reset_only_affected_manual_routes() {
        let identity = Identity {
            index: 1,
            client: Some(2),
            process: None,
            binary: Some("player".into()),
            app: None,
            desktop_app: Some("player.desktop".into()),
            name: "Stream".into(),
        };
        let mut config = Config {
            enabled: true,
            channels: vec![
                Channel {
                    id: "default".into(),
                    capture_unassigned: true,
                    ..Default::default()
                },
                Channel {
                    id: "music".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut engine = Engine::new("fixture".into());
        engine.update_memberships(&config);
        engine.yielded.insert(identity.clone());
        config.channels[1]
            .apps
            .push(AppSelector::DesktopApplication("unrelated.desktop".into()));
        engine.update_memberships(&config);
        assert!(
            engine.yielded.contains(&identity),
            "Unrelated assignment must preserve manual route"
        );
        config.channels[1]
            .apps
            .push(AppSelector::DesktopApplication("player.desktop".into()));
        engine.update_memberships(&config);
        assert!(
            engine.yielded.is_empty(),
            "Changed explicit destination resets only affected stream"
        );
        engine.yielded.insert(identity.clone());
        config.channels[0].capture_unassigned = false;
        config.channels.push(Channel {
            id: "new-default".into(),
            capture_unassigned: true,
            ..Default::default()
        });
        engine.update_memberships(&config);
        assert!(
            engine.yielded.contains(&identity),
            "Explicitly assigned stream ignores default changes"
        );
        config.channels[1].apps.clear();
        engine.update_memberships(&config);
        assert!(
            engine.yielded.is_empty(),
            "Moving explicit app to default changes its destination"
        );
        engine.yielded.insert(identity.clone());
        config.channels[2].capture_unassigned = false;
        config.channels[0].capture_unassigned = true;
        engine.update_memberships(&config);
        assert!(
            engine.yielded.is_empty(),
            "Changing default resets affected default stream"
        );
        engine.yielded.insert(identity.clone());
        config.enabled = false;
        engine.update_memberships(&config);
        assert!(engine.yielded.contains(&identity));
        config.enabled = true;
        engine.update_memberships(&config);
        assert!(
            engine.yielded.is_empty(),
            "Explicit re-enable takes ownership again"
        );
    }
    #[test]
    fn default_group_preserves_direct_owned_route_but_can_move_its_previous_assignment() {
        let identity = Identity {
            index: 1,
            client: Some(2),
            process: None,
            binary: None,
            app: None,
            desktop_app: None,
            name: "Stream".into(),
        };
        let mut input = Input {
            app: App {
                index: 1,
                sink: "deckforge.test.channel.explicit".into(),
                ..Default::default()
            },
            identity: identity.clone(),
            module: None,
            channels: 2,
            volume: ChannelVolumes::default(),
            muted: false,
        };
        let channel = Channel {
            capture_unassigned: true,
            ..Default::default()
        };
        let inv = Inventory {
            sinks: vec![Sink {
                device: Device {
                    name: input.app.sink.clone(),
                    ..Default::default()
                },
                index: 3,
                module: Some(7),
                channels: 2,
                volume: ChannelVolumes::default(),
                muted: false,
            }],
            ..Default::default()
        };
        let owned = BTreeSet::from([7]);
        let target = "deckforge.test.channel.default";
        assert!(default_must_yield(
            &channel,
            &input,
            target,
            &BTreeMap::new(),
            &inv,
            &owned
        ));
        let moved = BTreeMap::from([(
            1,
            Moved {
                identity,
                original: "external".into(),
                target: input.app.sink.clone(),
            },
        )]);
        assert!(!default_must_yield(
            &channel, &input, target, &moved, &inv, &owned
        ));
        input.app.sink = "external".into();
        assert!(!default_must_yield(
            &channel,
            &input,
            target,
            &BTreeMap::new(),
            &inv,
            &owned
        ));
    }
    #[test]
    fn derived_desktop_identity_does_not_invalidate_restoration_owner() {
        let saved = Identity {
            index: 1,
            client: Some(2),
            process: Some("42".into()),
            binary: Some("player".into()),
            app: None,
            desktop_app: None,
            name: "Stream".into(),
        };
        let mut resolved = saved.clone();
        resolved.desktop_app = Some("player.desktop".into());
        assert!(saved.same_stream(&resolved));
        assert!(matches_app(
            &AppSelector::DesktopApplication("player.desktop".into()),
            &resolved
        ));
        assert!(!matches_app(
            &AppSelector::ApplicationId("player.desktop".into()),
            &resolved
        ));
        resolved.client = Some(3);
        assert!(!saved.same_stream(&resolved));
    }
    fn listening_inventory() -> Inventory {
        Inventory {
            sinks: ["headphones", "other"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| Sink {
                    device: Device {
                        name: name.into(),
                        ..Default::default()
                    },
                    index: index as u32,
                    module: None,
                    channels: 2,
                    volume: ChannelVolumes::default(),
                    muted: false,
                })
                .collect(),
            ..Default::default()
        }
    }
    #[test]
    fn listening_plan_is_explicit_available_and_single_branch() {
        let engine = Engine::new("fixture".into());
        let inv = listening_inventory();
        let mut config = Config {
            monitoring: Monitoring {
                output: Some("headphones".into()),
                mix: Some("monitor".into()),
            },
            ..Default::default()
        };
        config.validate().unwrap();
        let mut warnings = vec![];
        assert!(
            !engine
                .plan(&config, &inv, &mut warnings)
                .iter()
                .any(|p| p.key.starts_with("listen:"))
        );
        config.enabled = true;
        config.monitoring.mix = None;
        assert!(
            !engine
                .plan(&config, &inv, &mut warnings)
                .iter()
                .any(|p| p.key.starts_with("listen:"))
        );
        config.monitoring.mix = Some("monitor".into());
        config.monitoring.output = None;
        assert!(
            !engine
                .plan(&config, &inv, &mut warnings)
                .iter()
                .any(|p| p.key.starts_with("listen:"))
        );
        config.monitoring.output = Some("missing".into());
        assert!(
            !engine
                .plan(&config, &inv, &mut warnings)
                .iter()
                .any(|p| p.key.starts_with("listen:"))
        );
        assert!(
            warnings
                .iter()
                .any(|s| s.contains("Listening output unavailable: missing"))
        );
        config.monitoring.output = Some("headphones".into());
        let plan = engine.plan(&config, &inv, &mut vec![]);
        let listeners = plan
            .iter()
            .filter(|p| p.key.starts_with("listen:"))
            .collect::<Vec<_>>();
        assert_eq!(listeners.len(), 1);
        let listener = listeners[0];
        assert!(
            listener
                .args
                .contains("source=\"deckforge.fixture.mix.monitor.monitor\"")
        );
        assert!(listener.args.contains("sink=\"headphones\""));
        assert!(
            listener
                .args
                .contains("source_dont_move=true sink_dont_move=true")
        );
        assert_eq!(output_gain(&config, &listener.key), (100.0, false));
    }
    #[test]
    fn listening_switch_does_not_pause_or_replace_other_graph_nodes() {
        let engine = Engine::new("fixture".into());
        let inv = listening_inventory();
        let mut config = Config {
            enabled: true,
            monitoring: Monitoring {
                output: Some("headphones".into()),
                mix: Some("monitor".into()),
            },
            ..Default::default()
        };
        let initial = engine.plan(&config, &inv, &mut vec![]);
        let owned = initial
            .iter()
            .enumerate()
            .map(|(index, p)| {
                (
                    p.key.clone(),
                    Module {
                        id: index as u32,
                        name: p.name.clone(),
                        args: p.args.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        for monitoring in [
            Monitoring {
                output: Some("headphones".into()),
                mix: Some("stream".into()),
            },
            Monitoring {
                output: Some("other".into()),
                mix: Some("stream".into()),
            },
            Monitoring {
                output: Some("headphones".into()),
                mix: None,
            },
        ] {
            config.monitoring = monitoring;
            let plan = engine.plan(&config, &inv, &mut vec![]);
            assert!(!graph_requires_pause(&plan, &owned, &BTreeSet::new()));
            let other = |plans: &[Spec]| {
                plans
                    .iter()
                    .filter(|p| !p.key.starts_with("listen:"))
                    .map(|p| (p.key.clone(), p.name.clone(), p.args.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(other(&initial), other(&plan));
        }
        config.mixes.remove(1);
        assert!(graph_requires_pause(
            &engine.plan(&config, &inv, &mut vec![]),
            &owned,
            &BTreeSet::new()
        ));
    }
    #[test]
    fn legacy_export_with_reader_is_retained_beside_canonical_alias_without_pausing_mix() {
        let mut engine = Engine::new("fixture".into());
        let config = Config {
            enabled: true,
            ..Default::default()
        };
        let plan = engine.plan(&config, &Inventory::default(), &mut vec![]);
        engine.owned = plan
            .iter()
            .enumerate()
            .map(|(index, p)| {
                (
                    p.key.clone(),
                    Module {
                        id: index as u32 + 1,
                        name: p.name.clone(),
                        args: p.args.clone(),
                    },
                )
            })
            .collect();
        let mix = engine
            .owned
            .iter()
            .find(|(key, _)| key == "mix:monitor")
            .unwrap()
            .1
            .clone();
        let canonical = engine
            .owned
            .iter()
            .find(|(key, _)| key == "export:monitor")
            .unwrap()
            .1
            .clone();
        let legacy_name = "deckforge.fixture.source.monitor";
        let mut legacy = canonical.clone();
        legacy.id = 100;
        legacy.args = legacy
            .args
            .replace(&mix_source("fixture", "monitor"), legacy_name)
            .replace("Deckforge mix monitor (recording)", "Deckforge mix monitor");
        engine.owned.retain(|(_, module)| module.id != canonical.id);
        engine.owned.push(("export:monitor".into(), legacy.clone()));
        let mut inv = Inventory {
            modules: engine.owned.iter().map(|(_, m)| m.clone()).collect(),
            sources: vec![Source {
                index: 200,
                device: Device {
                    name: legacy_name.into(),
                    ..Default::default()
                },
                module: Some(legacy.id),
                monitor_sink: None,
            }],
            captures: vec![Capture {
                client: Some(999),
                source: 200,
                module: None,
            }],
            ..Default::default()
        };
        let protected = engine.protected_modules(&inv);
        assert!(protected.contains(&legacy.id));
        assert!(protected.contains(&mix.id));
        assert!(!graph_requires_pause(&plan, &engine.owned, &protected));
        assert!(
            !engine
                .recording_sources(&config, &inv)
                .contains_key("monitor"),
            "Do not advertise unsafe retained alias"
        );
        // Reconciliation can add the new export without replacing the old key;
        // the journal stores owned modules as a Vec, so both identities persist.
        engine
            .owned
            .push(("export:monitor".into(), canonical.clone()));
        inv.modules.push(canonical.clone());
        inv.sources.push(Source {
            index: 201,
            device: Device {
                name: mix_source("fixture", "monitor"),
                ..Default::default()
            },
            module: Some(canonical.id),
            monitor_sink: None,
        });
        assert_eq!(
            engine.recording_sources(&config, &inv)["monitor"],
            mix_source("fixture", "monitor")
        );
        let protected = engine.protected_modules(&inv);
        assert!(protected.contains(&legacy.id));
        assert!(!protected.contains(&canonical.id));
        inv.captures.clear();
        assert!(
            !engine.protected_modules(&inv).contains(&legacy.id),
            "Legacy alias can be removed after its real reader leaves"
        );
        inv.modules.retain(|m| m.id != canonical.id);
        assert!(
            !engine
                .recording_sources(&config, &inv)
                .contains_key("monitor"),
            "Names alone never prove export ownership"
        );
    }
    #[test]
    fn listening_output_cannot_feed_its_own_mix_through_monitor_capture() {
        let mut config = Config {
            enabled: true,
            monitoring: Monitoring {
                output: Some("headphones".into()),
                mix: Some("monitor".into()),
            },
            channels: vec![Channel {
                id: "system".into(),
                name: "System".into(),
                sources: vec!["headphones.monitor".into()],
                ..Default::default()
            }],
            routes: vec![Route {
                channel: "system".into(),
                mix: "monitor".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let inv = Inventory {
            sources: vec![Source {
                index: 2,
                device: Device {
                    name: "headphones.monitor".into(),
                    monitor: true,
                    ..Default::default()
                },
                module: None,
                monitor_sink: Some("headphones".into()),
            }],
            ..Default::default()
        };
        config.validate().unwrap();
        assert!(reject_monitor_cycles(&config, &inv).is_err());
        config.monitoring.mix = Some("stream".into());
        assert!(reject_monitor_cycles(&config, &inv).is_ok());
        config.monitoring.mix = None;
        assert!(reject_monitor_cycles(&config, &inv).is_ok());
    }
    #[test]
    fn rejects_feedback_through_foreign_sink_monitors() {
        let c = Config {
            enabled: true,
            channels: vec![Channel {
                id: "system".into(),
                name: "System".into(),
                sources: vec!["speakers.monitor".into()],
                apps: vec![],
                ..Channel::default()
            }],
            mixes: vec![Mix {
                id: "monitor".into(),
                name: "Monitor".into(),
                outputs: vec!["speakers".into()],
                ..Mix::default()
            }],
            routes: vec![Route {
                channel: "system".into(),
                mix: "monitor".into(),
                ..Route::default()
            }],
            ..Config::default()
        };
        let inv = Inventory {
            sources: vec![Source {
                index: 1,
                device: Device {
                    name: "speakers.monitor".into(),
                    ..Device::default()
                },
                module: None,
                monitor_sink: Some("speakers".into()),
            }],
            ..Inventory::default()
        };
        assert!(reject_monitor_cycles(&c, &inv).is_err());
        let mut safe = c;
        safe.mixes[0].outputs = vec!["headphones".into()];
        reject_monitor_cycles(&safe, &inv).unwrap();
    }
    #[test]
    fn rename_and_output_trim_do_not_replace_endpoint_module_arguments() {
        let engine = Engine::new("fixture".into());
        let mut config:Config=serde_json::from_value(serde_json::json!({"enabled":true,"channels":[{"id":"music","name":"Before"}],"mixes":[{"id":"monitor","name":"Before","outputs":["owned-test-output"]}],"routes":[{"channel":"music","mix":"monitor"}]})).unwrap();
        let inv = Inventory {
            sinks: vec![Sink {
                device: Device {
                    name: "owned-test-output".into(),
                    ..Device::default()
                },
                index: 1,
                module: None,
                channels: 2,
                volume: ChannelVolumes::default(),
                muted: false,
            }],
            ..Inventory::default()
        };
        let plan = |config: &Config| {
            engine
                .plan(config, &inv, &mut vec![])
                .into_iter()
                .map(|s| (s.key, s.name, s.args))
                .collect::<Vec<_>>()
        };
        let before = plan(&config);
        config.channels[0].name = "After channel rename".into();
        config.mixes[0].name = "After mix rename".into();
        config.mixes[0].output_controls.insert(
            "owned-test-output".into(),
            OutputControl {
                volume: 25.0,
                muted: true,
            },
        );
        assert_eq!(plan(&config), before);
        assert_eq!(
            output_gain(&config, "output:monitor:owned-test-output"),
            (25.0, true)
        );
        assert_eq!(output_gain(&config, "capture:music:other"), (100.0, false));
        assert_eq!(
            output_gain(&config, "output:other:owned-test-output"),
            (100.0, false)
        );
    }
    #[test]
    fn meters_require_verified_owned_sink_and_matching_monitor() {
        let mut engine = Engine::new("fixture".into());
        let config:Config=serde_json::from_value(serde_json::json!({"enabled":true,"channels":[{"id":"music","name":"Music"}],"mixes":[],"routes":[]})).unwrap();
        let spec = engine
            .plan(&config, &Inventory::default(), &mut vec![])
            .remove(0);
        let module = Module {
            id: 7,
            name: spec.name,
            args: spec.args,
        };
        engine.owned.push(("channel:music".into(), module.clone()));
        let sink = channel_sink("fixture", "music");
        let mut inv = Inventory {
            modules: vec![module],
            sinks: vec![Sink {
                device: Device {
                    name: sink.clone(),
                    ..Device::default()
                },
                index: 12,
                module: Some(7),
                channels: 2,
                volume: ChannelVolumes::default(),
                muted: false,
            }],
            sources: vec![Source {
                index: 13,
                device: Device {
                    name: format!("{sink}.monitor"),
                    monitor: true,
                    ..Device::default()
                },
                module: Some(7),
                monitor_sink: Some(sink),
            }],
            ..Inventory::default()
        };
        assert_eq!(
            engine.meter_targets(&config, &inv)["channel:music"].source,
            13
        );
        inv.modules[0].args.push_str(" foreign");
        assert!(engine.meter_targets(&config, &inv).is_empty());
        inv.modules[0] = engine.owned[0].1.clone();
        inv.sources[0].module = Some(99);
        assert!(engine.meter_targets(&config, &inv).is_empty());
        inv.sources[0].module = Some(7);
        inv.sources[0].device.monitor = false;
        assert!(engine.meter_targets(&config, &inv).is_empty());
    }
    #[test]
    fn only_private_client_meter_captures_are_excluded_from_external_protection() {
        let mut engine = Engine::new("fixture".into());
        let module = Module {
            id: 7,
            name: "module-null-sink".into(),
            args: "fixture".into(),
        };
        engine.owned.push(("channel:a".into(), module.clone()));
        engine.meter_client = Some(5);
        let mut inv = Inventory {
            modules: vec![module],
            sources: vec![Source {
                index: 13,
                device: Device::default(),
                module: Some(7),
                monitor_sink: None,
            }],
            captures: vec![Capture {
                client: Some(5),
                source: 13,
                module: None,
            }],
            ..Inventory::default()
        };
        assert!(engine.protected_modules(&inv).is_empty());
        inv.captures[0].client = Some(6);
        assert!(engine.protected_modules(&inv).contains(&7));
        inv.captures[0].client = Some(5);
        assert!(engine.protected_modules(&inv).is_empty());
        engine.meter_client = None;
        inv.captures[0].client = None;
        assert!(engine.protected_modules(&inv).contains(&7));
    }
}
