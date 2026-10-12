//! Targeted, freshly ownership-checked gain updates between full inventories.
use super::*;
#[derive(Clone)]
pub(super) struct Cache {
    pub config: Config,
    pub snapshot: Snapshot,
    pub inventory: Inventory,
}
pub(super) enum Endpoint {
    Input { index: u32, channels: u8, sink: u32 },
    Sink { index: u32, name: String },
}
pub(super) struct Change {
    pub module: Module,
    pub endpoint: Endpoint,
    pub value: OutputControl,
}
pub(super) fn changes(engine: &Engine, config: &Config, cache: &Cache) -> Option<Vec<Change>> {
    let mut changes = vec![];
    for target in control::all_targets(config) {
        let value = control::read(config, &target).ok()?;
        if control::read(&cache.config, &target).ok()? == value {
            continue;
        }
        let key = match &target {
            control::Target::Route { channel, mix } => format!("route:{channel}:{mix}"),
            control::Target::Mix { mix } => format!("mix:{mix}"),
            control::Target::Output { mix, output } => format!("output:{mix}:{output}"),
        };
        let module = engine
            .owned
            .iter()
            .find(|(k, m)| k == &key && cache.inventory.modules.iter().any(|actual| actual == m))?
            .1
            .clone();
        let endpoint = match target {
            control::Target::Mix { mix } => {
                let name = mix_sink(&engine.namespace, &mix);
                let sink = cache
                    .inventory
                    .sinks
                    .iter()
                    .find(|s| s.device.name == name && s.module == Some(module.id))?;
                Endpoint::Sink {
                    index: sink.index,
                    name,
                }
            }
            _ => {
                let input = cache
                    .inventory
                    .inputs
                    .iter()
                    .find(|i| i.module == Some(module.id))?;
                let sink = cache
                    .inventory
                    .sinks
                    .iter()
                    .find(|s| s.device.name == input.app.sink)?
                    .index;
                Endpoint::Input {
                    index: input.app.index,
                    channels: input.channels,
                    sink,
                }
            }
        };
        changes.push(Change {
            module,
            endpoint,
            value,
        });
    }
    Some(changes)
}
impl Connection {
    pub(super) fn verify_gain(
        &mut self,
        change: &Change,
        owner: &str,
        session: &str,
        stop: &AtomicBool,
    ) -> Result<bool> {
        self.ready(stop)?;
        let actual = Rc::new(RefCell::new(None));
        let result = actual.clone();
        let mut operation =
            self.context
                .introspect()
                .get_module_info(change.module.id, move |value| {
                    if let ListResult::Item(m) = value {
                        *result.borrow_mut() = Some(Module {
                            id: m.index,
                            name: m.name.as_deref().unwrap_or("").into(),
                            args: m.argument.as_deref().unwrap_or("").into(),
                        });
                    }
                });
        self.wait(&mut operation, stop)?;
        if actual.borrow().as_ref() != Some(&change.module) {
            return Ok(false);
        }
        let verified = Rc::new(RefCell::new(false));
        let result = verified.clone();
        let owner = owner.to_owned();
        let session = session.to_owned();
        let module = change.module.id;
        match &change.endpoint {
            Endpoint::Input {
                index,
                channels,
                sink,
            } => {
                let index = *index;
                let channels = *channels;
                let sink = *sink;
                let mut operation =
                    self.context
                        .introspect()
                        .get_sink_input_info(index, move |value| {
                            if let ListResult::Item(input) = value {
                                *result.borrow_mut() = input.index == index
                                    && input.owner_module == Some(module)
                                    && input.sink == sink
                                    && input.sample_spec.channels == channels
                                    && input.proplist.get_str("deckforge.owner").as_deref()
                                        == Some(owner.as_str())
                                    && input.proplist.get_str("deckforge.session").as_deref()
                                        == Some(session.as_str());
                            }
                        });
                self.wait(&mut operation, stop)?;
            }
            Endpoint::Sink { index, name } => {
                let index = *index;
                let name = name.clone();
                let mut operation =
                    self.context
                        .introspect()
                        .get_sink_info_by_index(index, move |value| {
                            if let ListResult::Item(sink) = value {
                                *result.borrow_mut() = sink.index == index
                                    && sink.owner_module == Some(module)
                                    && sink.name.as_deref() == Some(name.as_str())
                                    && sink.sample_spec.channels == 2
                                    && sink.proplist.get_str("deckforge.owner").as_deref()
                                        == Some(owner.as_str())
                                    && sink.proplist.get_str("deckforge.session").as_deref()
                                        == Some(session.as_str());
                            }
                        });
                self.wait(&mut operation, stop)?;
            }
        }
        let matched = *verified.borrow();
        Ok(matched)
    }
    pub(super) fn apply_gain(&mut self, change: &Change, stop: &AtomicBool) -> Result<()> {
        match &change.endpoint {
            Endpoint::Input {
                index, channels, ..
            } => self.input_gain(
                *index,
                *channels,
                change.value.volume,
                change.value.muted,
                stop,
            ),
            Endpoint::Sink { index, .. } => {
                self.sink_index_gain(*index, change.value.volume, change.value.muted, stop)
            }
        }
    }
    fn sink_index_gain(
        &mut self,
        index: u32,
        percent: f64,
        muted: bool,
        stop: &AtomicBool,
    ) -> Result<()> {
        self.ready(stop)?;
        if muted {
            self.sink_index_mute(index, true, stop)?;
        }
        let mut volume = ChannelVolumes::default();
        volume.set(2, Volume::from(VolumeLinear(percent / 100.0)));
        let ok = Rc::new(RefCell::new(false));
        let result = ok.clone();
        let mut operation = self.context.introspect().set_sink_volume_by_index(
            index,
            &volume,
            Some(Box::new(move |value| *result.borrow_mut() = value)),
        );
        self.wait(&mut operation, stop)?;
        ensure!(*ok.borrow(), "Could not set owned mix gain");
        if !muted {
            self.sink_index_mute(index, false, stop)?;
        }
        Ok(())
    }
    fn sink_index_mute(&mut self, index: u32, muted: bool, stop: &AtomicBool) -> Result<()> {
        self.ready(stop)?;
        let ok = Rc::new(RefCell::new(false));
        let result = ok.clone();
        let mut operation = self.context.introspect().set_sink_mute_by_index(
            index,
            muted,
            Some(Box::new(move |value| *result.borrow_mut() = value)),
        );
        self.wait(&mut operation, stop)?;
        ensure!(*ok.borrow(), "Could not set owned mix mute");
        Ok(())
    }
}
