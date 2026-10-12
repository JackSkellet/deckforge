//! Translate typed patchbay cables into owned Pulse/PipeWire graph primitives.
use super::*;
use crate::mixer::patchbay::{Connection as Cable, Endpoint};

impl Engine {
    pub(super) fn validate_patch_ports(&self, cables: &[Cable], inv: &Inventory) -> Result<()> {
        for c in cables {
            for (endpoint, channel) in [(&c.from, &c.source_channel), (&c.to, &c.target_channel)] {
                let Some(channel) = channel else {
                    continue;
                };
                let channels = match endpoint {
                    Endpoint::Source { name } => inv
                        .sources
                        .iter()
                        .find(|s| &s.device.name == name)
                        .map(|s| s.device.channels.clone()),
                    Endpoint::Output { name } => inv
                        .sinks
                        .iter()
                        .find(|s| &s.device.name == name)
                        .map(|s| s.device.channels.clone()),
                    _ => Some(vec!["front-left".into(), "front-right".into()]),
                };
                if let Some(channels) = channels {
                    ensure!(
                        channels.iter().any(|value| value == channel),
                        "Connection {} refers to unavailable audio channel {}",
                        c.id,
                        channel
                    );
                }
            }
        }
        Ok(())
    }

    fn patch_source(&self, endpoint: &Endpoint, inv: &Inventory) -> Option<String> {
        match endpoint {
            Endpoint::Source { name } => inv
                .sources
                .iter()
                .any(|s| &s.device.name == name)
                .then(|| name.clone()),
            Endpoint::Channel { id } => {
                Some(format!("{}.monitor", channel_sink(&self.namespace, id)))
            }
            Endpoint::Mix { id } | Endpoint::Recording { mix: id } => {
                Some(format!("{}.monitor", mix_sink(&self.namespace, id)))
            }
            Endpoint::Output { name } => inv
                .sources
                .iter()
                .find(|s| s.monitor_sink.as_ref() == Some(name))
                .map(|s| s.device.name.clone()),
            Endpoint::App { .. } | Endpoint::AllAudio => None,
        }
    }
    fn patch_sink(&self, endpoint: &Endpoint, inv: &Inventory) -> Option<String> {
        match endpoint {
            Endpoint::Channel { id } => Some(channel_sink(&self.namespace, id)),
            Endpoint::Mix { id } => Some(mix_sink(&self.namespace, id)),
            Endpoint::Output { name } => inv
                .sinks
                .iter()
                .any(|s| &s.device.name == name)
                .then(|| name.clone()),
            _ => None,
        }
    }
    pub(super) fn patch_plan(
        &self,
        cables: &[Cable],
        inv: &Inventory,
        warnings: &mut Vec<String>,
    ) -> Vec<Spec> {
        let mut plan = vec![];
        for cable in cables {
            let (Some(source), Some(sink)) = (
                self.patch_source(&cable.from, inv),
                self.patch_sink(&cable.to, inv),
            ) else {
                warnings.push(format!(
                    "Connection {} has an unavailable device; no fallback route selected",
                    cable.id
                ));
                continue;
            };
            let source = if let (Some(from), Some(to)) =
                (&cable.source_channel, &cable.target_channel)
            {
                let name = format!(
                    "deckforge.{}.connection.{}.source",
                    self.namespace, cable.id
                );
                let props = quote(&format!(
                    "device.description={} deckforge.owner={} deckforge.session={} node.hidden=true",
                    quote("Deckforge internal channel mapping"),
                    quote(&self.namespace),
                    quote(&self.session)
                ));
                plan.push(Spec{key:format!("connection-source:{}",cable.id),name:"module-remap-source".into(),args:format!("master={} source_name={} master_channel_map={} channel_map={} channels=1 remix=no source_properties={}",quote(&source),quote(&name),quote(from),quote(to),props),sink:None,source:Some(name.clone())});
                name
            } else {
                source
            };
            let mut link = self.link(format!("connection:{}", cable.id), source, sink);
            if let Some(channel) = &cable.target_channel {
                link.args = link.args.replace(
                    "channels=2 channel_map=front-left,front-right",
                    &format!("channels=1 channel_map={} remix=false", quote(channel)),
                );
            }
            plan.push(link);
        }
        plan
    }
}
