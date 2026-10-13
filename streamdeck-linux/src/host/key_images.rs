//! Source ownership and transient playback for key images. Original bytes remain exportable.
use super::*;
use crate::key_animation::Clip;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Source {
    File(PathBuf),
    Profile(PathBuf, String, String),
    Plugin(PathBuf, String, u64),
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    fn gif_url() -> String {
        format!(
            "data:image/gif;base64,{}",
            STANDARD.encode(crate::key_animation::tests::gif())
        )
    }
    fn set(host: &mut Host, context: &str, payload: Value) {
        host.plugin_message(
            0,
            json!({"event":"setImage","context":context,"payload":payload}),
        )
        .unwrap();
    }
    fn hardware(receiver: &std::sync::mpsc::Receiver<Draw>) -> DynamicImage {
        receiver
            .try_iter()
            .filter_map(|draw| {
                if let Draw::Key(0, image) = draw {
                    Some(image)
                } else {
                    None
                }
            })
            .last()
            .unwrap()
    }
    fn preview(host: &Host) -> DynamicImage {
        image::load_from_memory(&STANDARD.decode(&host.slots[0].preview).unwrap()).unwrap()
    }
    #[test]
    fn sdk_frames_keep_targets_separate_and_unchanged_resends_restart_only_their_plane() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let id = host.slots[0].context.clone();
        let raw = host.slots[0].raw.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        set(&mut host, &id, json!({"image":gif_url(),"state":0}));
        assert_eq!(hardware(&receiver).get_pixel(0, 0)[0], 255);
        let due = host.key_images.next_due().unwrap();
        host.multi_clock = std::time::Instant::now() - due;
        host.poll_key_images();
        assert_eq!(preview(&host).get_pixel(0, 0)[2], 255);
        assert_eq!(hardware(&receiver).get_pixel(0, 0)[2], 255);
        assert_eq!(
            host.slots[0].raw, raw,
            "Animation frames must not rewrite source metadata"
        );
        assert_eq!(
            host.slots[0].appearances[&0]
                .plugin_pixels
                .as_ref()
                .unwrap()
                .get_pixel(0, 0)[0],
            255
        );
        set(
            &mut host,
            &id,
            json!({"image":gif_url(),"state":0,"target":1}),
        );
        assert_eq!(hardware(&receiver).get_pixel(0, 0)[0], 255);
        assert_eq!(preview(&host).get_pixel(0, 0)[2], 255);
        assert!(
            host.slots[0].appearances[&0]
                .hardware
                .as_ref()
                .unwrap()
                .animation_source
                .is_some()
        );
        let before = host.slots[0].preview.clone();
        assert!(host.plugin_message(0,json!({"event":"setImage","context":id,"payload":{"image":"data:image/gif;base64,R0lGODlh","state":0}})).is_err());
        assert_eq!(host.slots[0].preview, before);
        host.library_command(&json!({"command":"page_create","name":"Away"}))
            .unwrap();
        assert!(host.key_images.next_due().is_none());
        assert!(host.key_images.planes.iter().all(HashMap::is_empty));
    }
    #[test]
    fn user_artwork_keeps_playing_over_sdk_writes_and_states_have_independent_sources() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(temp.path());
        let id = host.slots[0].context.clone();
        let file = temp.path().join("user.gif");
        fs::write(&file, crate::key_animation::tests::gif()).unwrap();
        let entry = host.slots[0].appearances.entry(0).or_default();
        entry.user_image = Some(file.clone());
        entry.user_pixels = Some(load_image(temp.path(), "user.gif").unwrap());
        host.render().unwrap();
        let due = host.key_images.next_due().unwrap();
        host.multi_clock = std::time::Instant::now() - due;
        host.poll_key_images();
        assert_eq!(preview(&host).get_pixel(0, 0)[2], 255);
        set(&mut host, &id, json!({"image":gif_url(),"state":0}));
        assert_eq!(
            preview(&host).get_pixel(0, 0)[2],
            255,
            "Hidden SDK writes must not restart a user animation"
        );
        set(&mut host, &id, json!({"image":gif_url(),"state":1}));
        host.plugin_message(
            0,
            json!({"event":"setState","context":id,"payload":{"state":1}}),
        )
        .unwrap();
        assert_eq!(preview(&host).get_pixel(0, 0)[0], 255);
        host.plugin_message(
            0,
            json!({"event":"setState","context":id,"payload":{"state":0}}),
        )
        .unwrap();
        assert_eq!(
            preview(&host).get_pixel(0, 0)[0],
            255,
            "Reappearing state starts with its first frame"
        );
        assert_eq!(fs::read(file).unwrap(), crate::key_animation::tests::gif());
    }
}
struct Cached {
    clip: Option<Arc<Clip>>,
    used: u64,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<Source, Cached>,
    serial: u64,
    bytes: usize,
}
impl Cache {
    fn get(&mut self, source: &Source) -> Option<Option<Arc<Clip>>> {
        let entry = self.entries.get_mut(source)?;
        self.serial = self.serial.wrapping_add(1);
        entry.used = self.serial;
        Some(entry.clip.clone())
    }
    fn insert(&mut self, source: Source, clip: Option<Arc<Clip>>) -> Result<()> {
        let size = clip.as_ref().map_or(0, |c| c.bytes);
        while self.bytes + size > 128 * 1024 * 1024 || self.entries.len() >= 2048 {
            let candidate = self
                .entries
                .iter()
                .filter(|(_, v)| v.clip.as_ref().is_none_or(|c| Arc::strong_count(c) == 1))
                .min_by_key(|(_, v)| v.used)
                .map(|(k, _)| k.clone())
                .context("Active animated images exceed the 128 MiB cache budget")?;
            let removed = self.entries.remove(&candidate).unwrap();
            self.bytes -= removed.clip.as_ref().map_or(0, |c| c.bytes);
        }
        self.serial = self.serial.wrapping_add(1);
        self.bytes += size;
        self.entries.insert(
            source,
            Cached {
                clip,
                used: self.serial,
            },
        );
        Ok(())
    }
}
struct Active {
    source: Source,
    state: u64,
    clip: Option<Arc<Clip>>,
    elapsed: Duration,
    due: Option<Duration>,
}
#[derive(Default)]
pub(super) struct Runtime {
    planes: [HashMap<String, Active>; 2],
}
impl Runtime {
    pub(super) fn restart(&mut self, context: &str, software: bool, hardware: bool) {
        if software {
            self.planes[0].remove(context);
        }
        if hardware {
            self.planes[1].remove(context);
        }
    }
    pub(super) fn frame(&self, context: &str, hardware: bool) -> Option<&DynamicImage> {
        let entry = self.planes[usize::from(hardware)].get(context)?;
        Some(entry.clip.as_ref()?.frame(entry.elapsed))
    }
    pub(super) fn next_due(&self) -> Option<Duration> {
        self.planes
            .iter()
            .flat_map(|p| p.values())
            .filter_map(|p| p.due)
            .min()
    }
    fn advance(&mut self, now: Duration) -> bool {
        let mut changed = false;
        for entry in self.planes.iter_mut().flat_map(|p| p.values_mut()) {
            if entry.due.is_some_and(|due| due <= now)
                && let Some(clip) = &entry.clip
            {
                // Native image nodes advance one frame, then schedule its own
                // delay from now. They do not skip unseen frames after a stall.
                entry.elapsed += clip.next_in(entry.elapsed).unwrap_or_default();
                entry.due = clip.next_in(entry.elapsed).map(|delay| now + delay);
                changed = true;
            }
        }
        changed
    }
}

impl Host {
    fn key_image_source(&self, slot: &Slot, hardware: bool) -> Option<Source> {
        if slot.controller != "Keypad" && slot.controller != "Encoder" {
            return None;
        }
        if slot.controller == "Encoder" && hardware {
            return None;
        }
        let entry = slot.appearances.get(&slot.state);
        if let Some(path) = entry.and_then(|e| e.user_image.as_ref()) {
            return Some(Source::File(path.clone()));
        }
        if entry.is_some_and(|e| e.user_pixels.is_some() && e.user_image.is_none()) {
            return None;
        }
        if slot.controller == "Encoder" {
            if entry.is_some_and(|e| e.imported_image && !e.image_reset) {
                let name = slot.states[slot.state as usize]["Image"].as_str()?;
                if let Some(origin) = self.asset_origin(slot) {
                    return Some(Source::Profile(origin.source, origin.manifest, name.into()));
                }
                let action = self.actions.iter().find(|a| a.uuid == slot.uuid)?;
                return self
                    .plugins
                    .iter()
                    .find(|p| p.id == action.plugin)
                    .map(|p| Source::Plugin(p.root.clone(), name.into(), p.asset_generation));
            }
            if let Some(entry) = entry
                && entry.encoder_pixels.is_some()
            {
                return entry
                    .encoder_animation_source
                    .as_ref()
                    .map(|p| Source::File(p.clone()));
            }
            let action = self.actions.iter().find(|a| a.uuid == slot.uuid)?;
            let name = action.raw["Encoder"]["Icon"].as_str()?;
            return self
                .plugins
                .iter()
                .find(|p| p.id == action.plugin)
                .map(|p| Source::Plugin(p.root.clone(), name.into(), p.asset_generation));
        }
        let imported = entry.is_some_and(|e| e.imported_image && !e.image_reset);
        if !imported && let Some(entry) = entry {
            let (pixels, source) = if hardware && let Some(h) = &entry.hardware {
                (&h.pixels, &h.animation_source)
            } else {
                (&entry.plugin_pixels, &entry.plugin_animation_source)
            };
            if pixels.is_some() {
                return source.as_ref().map(|p| Source::File(p.clone()));
            }
        }
        let action = self.actions.iter().find(|a| a.uuid == slot.uuid)?;
        let plugin = self.plugins.iter().find(|p| p.id == action.plugin);
        if !entry.is_some_and(|e| e.image_reset)
            && let Some(name) = slot.states[slot.state as usize]["Image"]
                .as_str()
                .filter(|n| !n.is_empty())
        {
            if let Some(origin) = self.asset_origin(slot) {
                return Some(Source::Profile(origin.source, origin.manifest, name.into()));
            }
            return plugin.map(|p| Source::Plugin(p.root.clone(), name.into(), p.asset_generation));
        }
        let name = action.raw["States"][slot.state as usize]["Image"]
            .as_str()
            .filter(|n| !n.is_empty())?;
        plugin.map(|p| Source::Plugin(p.root.clone(), name.into(), p.asset_generation))
    }
    fn key_image_clip(&mut self, source: &Source) -> Result<Option<Arc<Clip>>> {
        if let Some(clip) = self.key_image_cache.get(source) {
            return Ok(clip);
        }
        let bytes = match source {
            Source::File(path) => crate::resources::read_regular(path)?.1,
            Source::Plugin(root, name, _) => display::IMAGE_SUFFIXES
                .iter()
                .find_map(|suffix| image_bytes(root, &format!("{name}{suffix}")).ok())
                .context("Key image is unavailable")?,
            Source::Profile(path, manifest, name) => {
                if !self.profile_assets.contains_key(path) {
                    self.profile_assets
                        .insert(path.clone(), profile::load_bundle(path)?);
                }
                self.profile_assets[path]
                    .pages
                    .iter()
                    .find(|p| p.manifest_path() == *manifest)
                    .context("Image profile page is missing")?
                    .asset(name)?
                    .to_vec()
            }
        };
        let clip = Clip::decode(&bytes)?.map(Arc::new);
        self.key_image_cache.insert(source.clone(), clip.clone())?;
        Ok(clip)
    }
    pub(super) fn sync_key_images(&mut self) {
        let plans: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .flat_map(|slot| {
                [false, true].into_iter().filter_map(|hardware| {
                    self.key_image_source(slot, hardware).map(|source| {
                        (
                            slot.context.clone(),
                            slot.state,
                            usize::from(hardware),
                            source,
                        )
                    })
                })
            })
            .collect();
        let mut runtime = std::mem::take(&mut self.key_images);
        for (plane, entries) in runtime.planes.iter_mut().enumerate() {
            entries.retain(|context, entry| {
                plans.iter().any(|(c, state, p, source)| {
                    c == context && *p == plane && *state == entry.state && source == &entry.source
                })
            });
        }
        let now = self.multi_clock.elapsed();
        let mut started = false;
        for (context, state, plane, source) in plans {
            if runtime.planes[plane].contains_key(&context) {
                continue;
            }
            let clip = match self.key_image_clip(&source) {
                Ok(clip) => clip,
                Err(error) => {
                    self.log(format!(
                        "Animated image for {context}: {error:#}; keeping the still image"
                    ));
                    None
                }
            };
            let due = clip
                .as_ref()
                .and_then(|clip| clip.next_in(Duration::ZERO))
                .map(|delay| now + delay);
            started |= due.is_some();
            runtime.planes[plane].insert(
                context,
                Active {
                    source,
                    state,
                    clip,
                    elapsed: Duration::ZERO,
                    due,
                },
            );
        }
        self.key_images = runtime;
        if started {
            self.multi_notify.notify_one();
        }
    }
    pub(super) fn poll_key_images(&mut self) {
        if self.key_images.advance(self.multi_clock.elapsed())
            && let Err(error) = self.render()
        {
            self.log(format!("Animated key redraw: {error:#}"));
        }
    }
    pub(super) fn store_plugin_animation(&mut self, bytes: &[u8]) -> Result<Option<PathBuf>> {
        let Some(clip) = Clip::decode(bytes)? else {
            return Ok(None);
        };
        use sha2::{Digest, Sha256};
        let id = format!("{:x}", Sha256::digest(bytes));
        let path = crate::resources::materialize(
            &self
                .state_path
                .parent()
                .context("State directory missing")?
                .join("appearance-assets"),
            "animations",
            &id,
            "image.gif",
            bytes,
        )?;
        let source = Source::File(path.clone());
        if self.key_image_cache.get(&source).is_none() {
            self.key_image_cache.insert(source, Some(Arc::new(clip)))?;
        }
        Ok(Some(path))
    }
}
