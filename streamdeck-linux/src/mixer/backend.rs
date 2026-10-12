#[path = "journal.rs"]
mod journal;
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
    name: String,
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
    source: u32,
    module: Option<u32>,
}
#[derive(Default)]
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
    namespace: String,
    session: String,
    journal: Option<journal::Journal>,
    connection: Option<Connection>,
    owned: Vec<(String, Module)>,
    pending_loads: BTreeMap<String, (String, String)>,
    moved: BTreeMap<u32, Moved>,
    yielded: BTreeSet<Identity>,
    last_members: Vec<(String, Vec<AppSelector>)>,
    last_enabled: bool,
    paused: BTreeSet<String>,
}
impl Engine {
    pub fn new(namespace: String) -> Self {
        Self {
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
        }
    }
    pub fn update(&mut self, config: &Config, stop: &AtomicBool) -> Snapshot {
        let mut snapshot = Snapshot {
            config: config.clone(),
            ..Snapshot::default()
        };
        let result = (|| -> Result<()> {
            if self.connection.is_none() {
                self.connection = Some(Connection::new(stop)?);
            }
            let mut connection = self.connection.take().unwrap();
            let result = (|| -> Result<()> {
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
                let members = config
                    .channels
                    .iter()
                    .map(|c| (c.id.clone(), c.apps.clone()))
                    .collect::<Vec<_>>();
                if members != self.last_members || (config.enabled && !self.last_enabled) {
                    self.yielded.clear();
                    self.last_members = members;
                }
                self.last_enabled = config.enabled;
                if config.enabled {
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
                snapshot.apps = inv
                    .inputs
                    .iter()
                    .filter(|i| {
                        i.identity.client.is_some() && !i.module.is_some_and(|m| ids.contains(&m))
                    })
                    .map(|i| i.app.clone())
                    .collect();
                snapshot.active = config.enabled && !ids.is_empty();
                Ok(())
            })();
            let result = result.and(self.persist());
            if connection.context.get_state() == ContextState::Ready {
                self.connection = Some(connection);
            } else {
                snapshot.connected = false;
            }
            result
        })();
        if let Err(error) = result {
            snapshot.error = Some(format!("{error:#}"));
        }
        snapshot
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
                        .any(|i| i.identity == m.identity && i.app.sink == m.target)
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
            plan.push(Spec{key:format!("export:{}",mix.id),name:"module-remap-source".into(),args:format!("master={} source_name={} channels=2 channel_map=front-left,front-right source_properties={}",quote(&format!("{}.monitor",mix_sink(&self.namespace,&mix.id))),quote(&source),props(&format!("Deckforge mix {}",mix.id))),sink:None,source:Some(source)});
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
        let changed = plan.iter().any(|p| {
            !self
                .owned
                .iter()
                .any(|(key, m)| key == &p.key && m.name == p.name && m.args == p.args)
        }) || self.owned.iter().any(|(key, m)| {
            !plan.iter().any(|p| &p.key == key) && !self.protected_modules(inv).contains(&m.id)
        });
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
        // Capture and physical-output legs must have an explicit unity gain too.
        for (key, module) in &self.owned {
            if key.starts_with("capture:") || key.starts_with("output:") {
                let input = inv
                    .inputs
                    .iter()
                    .find(|i| i.module == Some(module.id))
                    .context("Owned capture/output stream missing")?;
                if !gain_matches(&input.volume, 100.0) || input.muted {
                    c.input_gain(input.app.index, input.channels, 100.0, false, stop)?;
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
    fn sync_apps(
        &mut self,
        c: &mut Connection,
        config: &Config,
        inv: &mut Inventory,
        warnings: &mut Vec<String>,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.restore(c, inv, warnings, stop, |m| {
            !config
                .channels
                .iter()
                .any(|ch| ch.apps.iter().any(|s| matches_app(s, &m.identity)))
        })?;
        let ids = self.verified_ids(inv);
        for input in &inv.inputs {
            if input.identity.client.is_none() || input.module.is_some_and(|m| ids.contains(&m)) {
                continue;
            }
            if let Some(previous) = self.moved.get(&input.app.index)
                && (previous.identity != input.identity || input.app.sink != previous.target)
            {
                self.moved.remove(&input.app.index);
                self.yielded.insert(input.identity.clone());
            }
            if self.yielded.contains(&input.identity) {
                continue;
            }
            let matches = config
                .channels
                .iter()
                .filter(|ch| ch.apps.iter().any(|s| matches_app(s, &input.identity)))
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                warnings.push(format!(
                    "App {} matches more than one channel; no routing changed",
                    input.app.name
                ));
                continue;
            }
            let Some(channel) = matches.first() else {
                continue;
            };
            let target = channel_sink(&self.namespace, &channel.id);
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
                .find(|i| i.app.index == id && i.identity == moved.identity)
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
        if let Some(mut c) = self.connection.take() {
            c.deadline = Some(Instant::now() + Duration::from_secs(2));
            let stop = AtomicBool::new(false);
            if let Ok(mut inv) = c.inventory(&stop) {
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
fn matches_app(selector: &AppSelector, identity: &Identity) -> bool {
    match selector {
        AppSelector::ApplicationId(v) => identity.app.as_ref() == Some(v),
        AppSelector::ProcessBinary(v) => identity.binary.as_ref() == Some(v),
    }
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
struct Connection {
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
                    to.borrow_mut().push(Input {
                        app: App {
                            index: i.index,
                            name: name.clone(),
                            application_id: app.clone(),
                            process_binary: binary.clone(),
                            sink: lookup.get(&i.sink).cloned().unwrap_or_default(),
                        },
                        identity: Identity {
                            index: i.index,
                            client: i.client,
                            process: i.proplist.get_str("application.process.id"),
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
    fn rejects_feedback_through_foreign_sink_monitors() {
        let c = Config {
            enabled: true,
            channels: vec![Channel {
                id: "system".into(),
                name: "System".into(),
                sources: vec!["speakers.monitor".into()],
                apps: vec![],
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
}
