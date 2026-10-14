//! Page-owned encoder backgrounds retain archive provenance, including copied folders.
use super::*;

type Key = (PathBuf, String, String, Model);
#[derive(Default)]
pub(super) struct Runtime {
    cache: HashMap<Key, Option<Background>>,
    active: Option<Animation>,
}
struct Animation {
    key: Key,
    page: String,
    movie: crate::background_movie::Movie,
    frame: Background,
}
impl Runtime {
    pub fn next_due(&self) -> Option<Duration> {
        self.active.as_ref().and_then(|a| a.movie.next_due())
    }
    fn frame(&self, key: &Key) -> Option<&Background> {
        self.active
            .as_ref()
            .filter(|a| a.key == *key)
            .map(|a| &a.frame)
            .or_else(|| self.cache.get(key).and_then(Option::as_ref))
    }
}
struct Background {
    pixels: DynamicImage,
    preview: String,
    tiles: Vec<String>,
    animation: Option<Arc<[u8]>>,
}
impl Background {
    fn from_image(decoded: DynamicImage, model: Model) -> Result<Self> {
        let decoded = decoded.resize_exact(
            model.strip_width(),
            model.strip_height(),
            image::imageops::FilterType::Lanczos3,
        );
        let mut pixels = DynamicImage::ImageRgb8(RgbImage::from_pixel(
            model.strip_width(),
            model.strip_height(),
            Rgb([0, 0, 0]),
        ));
        image::imageops::overlay(&mut pixels, &decoded, 0, 0);
        let preview = display_targets::png(&pixels)?;
        let (width, height) = model.encoder_size();
        let tiles = (0..model.dial_count())
            .map(|index| {
                let (x, y) = model.encoder_origin(index);
                display_targets::png(&pixels.crop_imm(x, y, width, height))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pixels,
            preview,
            tiles,
            animation: None,
        })
    }
}
impl Host {
    pub(super) fn encoder_background_configured(&self) -> bool {
        self.library
            .current()
            .ok()
            .and_then(|p| p.pages.iter().find(|p| p.id == self.library.active_page))
            .and_then(|p| p.raw["Controllers"].as_array())
            .and_then(|c| c.iter().find(|c| c["Type"] == "Encoder"))
            .and_then(|c| c["Background"].as_str())
            .is_some_and(|p| !p.is_empty())
    }
    pub(super) fn set_controller_background(&mut self, request: &Value) -> Result<()> {
        use anyhow::ensure;
        use std::io::Write;
        self.sync_visible();
        let profile_id = request["profile"].as_str().context("Profile missing")?;
        let page_id = request["id"].as_str().context("Page missing")?;
        let profile = self
            .library
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .context("Profile missing")?;
        ensure!(
            profile.device == self.device,
            "Background belongs to another device"
        );
        ensure!(!profile.readonly, "Profile is read-only");
        ensure!(
            profile.model.capabilities().strip && profile.model.dial_count() > 0,
            "This model has no encoder display"
        );
        ensure!(
            profile.pages.iter().any(|p| p.id == page_id),
            "Page missing"
        );
        let page = profile.pages.iter().find(|p| p.id == page_id).unwrap();
        ensure!(
            page.raw.is_object()
                && (page.raw["Controllers"].is_null() || page.raw["Controllers"].is_array()),
            "Invalid page controller metadata"
        );
        let path = request
            .get("path")
            .context("Background path or null is required")?;
        let asset = if path.is_null() {
            None
        } else {
            let path = Path::new(path.as_str().context("Background path must be text")?);
            let metadata = fs::metadata(path).context("Read background metadata")?;
            ensure!(
                metadata.is_file() && metadata.len() <= 16 * 1024 * 1024,
                "Background must be a regular file no larger than 16 MiB"
            );
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            ensure!(
                matches!(
                    extension.as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"
                ),
                "Unsupported background image type"
            );
            let bytes = fs::read(path)?;
            ensure!(bytes.len() <= 16 * 1024 * 1024, "Background exceeds 16 MiB");
            display::decode_image(&bytes, extension == "svg")
                .context("Invalid background image")?;
            let id = library::new_id();
            let image = format!("Images/background-{id}.{extension}");
            let directory = self
                .state_path
                .parent()
                .context("State directory missing")?
                .join("sources");
            fs::create_dir_all(&directory)?;
            let destination = directory.join(format!("background-{id}.streamDeckProfile"));
            let temporary = tempfile::NamedTempFile::new_in(&directory)?;
            {
                let mut zip = zip::ZipWriter::new(temporary.as_file());
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                zip.start_file("manifest.json", options)?;
                zip.write_all(serde_json::to_string(&json!({"Name":"Display background","Controllers":[{"Type":"Encoder","Background":image,"Actions":{}}]}))?.as_bytes())?;
                zip.start_file(&image, options)?;
                zip.write_all(&bytes)?;
                zip.finish()?;
            }
            temporary
                .persist_noclobber(&destination)
                .map_err(|e| e.error)?;
            Some((
                image,
                transfer::AssetSource {
                    source: destination,
                    manifest: "manifest.json".into(),
                    profile: profile_id.into(),
                },
            ))
        };
        let page = self
            .library
            .profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .unwrap()
            .pages
            .iter_mut()
            .find(|p| p.id == page_id)
            .unwrap();
        let old_raw = page.raw.clone();
        let old_source = page.background_source.clone();
        if page.raw["Controllers"].is_null() {
            page.raw["Controllers"] = if page.raw["Actions"].is_object() {
                let actions = page.raw.as_object_mut().unwrap().remove("Actions").unwrap();
                json!([{"Type":"Keypad","Actions":actions}])
            } else {
                json!([])
            };
        }
        let controllers = page.raw["Controllers"].as_array_mut().unwrap();
        let at = controllers
            .iter()
            .position(|c| c["Type"] == "Encoder")
            .unwrap_or_else(|| {
                controllers.push(json!({"Type":"Encoder","Actions":{}}));
                controllers.len() - 1
            });
        if let Some((image, source)) = &asset {
            controllers[at]["Background"] = json!(image);
            page.background_source = Some(source.clone());
        } else {
            controllers[at]
                .as_object_mut()
                .context("Invalid Encoder controller")?
                .remove("Background");
            page.background_source = None;
        }
        if let Err(error) = self.persist() {
            let page = self
                .library
                .profiles
                .iter_mut()
                .find(|p| p.id == profile_id)
                .unwrap()
                .pages
                .iter_mut()
                .find(|p| p.id == page_id)
                .unwrap();
            page.raw = old_raw;
            page.background_source = old_source;
            if let Some((_, source)) = asset {
                let _ = fs::remove_file(source.source);
            }
            return Err(error);
        }
        let retained: HashSet<_> = self
            .retained_asset_sources()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        self.controller_backgrounds
            .cache
            .retain(|(path, _, _, _), _| retained.contains(path));
        self.profile_assets
            .retain(|path, _| retained.contains(path));
        self.render()
    }
    fn encoder_background_key(&self) -> Option<Key> {
        if !self.model.capabilities().strip || self.model.dial_count() == 0 {
            return None;
        }
        let profile = self.library.current().ok()?;
        let page = profile
            .pages
            .iter()
            .find(|p| p.id == self.library.active_page)?;
        let image = page.raw["Controllers"]
            .as_array()?
            .iter()
            .find(|c| c["Type"] == "Encoder")?["Background"]
            .as_str()?
            .to_owned();
        if image.is_empty() {
            return None;
        }
        let (source, manifest) = if let Some(origin) = page
            .background_source
            .as_ref()
            .or(page.asset_source.as_ref())
        {
            (origin.source.clone(), origin.manifest.clone())
        } else {
            (profile.source.clone()?, page.source_manifest.clone()?)
        };
        Some((source, manifest, image, self.model))
    }
    pub(super) fn encoder_background(&mut self) -> Option<DynamicImage> {
        let Some(key) = self.encoder_background_key() else {
            self.controller_backgrounds.active = None;
            return None;
        };
        if !self.controller_backgrounds.cache.contains_key(&key) {
            let result = (|| -> Result<Background> {
                let (path, manifest, image, model) = &key;
                if !self.profile_assets.contains_key(path) {
                    self.profile_assets
                        .insert(path.clone(), profile::load_bundle(path)?);
                }
                let bundle = &self.profile_assets[path];
                let page = bundle
                    .pages
                    .iter()
                    .find(|p| p.manifest_path() == *manifest)
                    .context("Background source page is missing")?;
                let bytes = page.asset(image)?;
                let decoded =
                    display::decode_image(&bytes, image.to_ascii_lowercase().ends_with(".svg"))?;
                let mut background = Background::from_image(decoded, *model)?;
                if *model == Model::Galleon
                    && image.to_ascii_lowercase().ends_with(".webp")
                    && crate::background_movie::is_animated(&bytes)?
                {
                    background.animation = Some(Arc::from(bytes));
                }
                Ok(background)
            })();
            let background = match result {
                Ok(background) => Some(background),
                Err(error) => {
                    self.log(format!("Encoder background unavailable: {error:#}"));
                    None
                }
            };
            self.controller_backgrounds
                .cache
                .insert(key.clone(), background);
        }
        if self
            .controller_backgrounds
            .active
            .as_ref()
            .is_none_or(|a| a.key != key || a.page != self.library.active_page)
        {
            self.controller_backgrounds.active = None;
            let bytes = self.controller_backgrounds.cache[&key]
                .as_ref()
                .and_then(|b| b.animation.clone());
            if let Some(bytes) = bytes {
                let loaded = crate::background_movie::Movie::new(bytes, self.multi_clock.elapsed())
                    .and_then(|(movie, image)| {
                        Ok((movie, Background::from_image(image, self.model)?))
                    });
                match loaded {
                    Ok((movie, frame)) => {
                        self.controller_backgrounds.active = Some(Animation {
                            key: key.clone(),
                            page: self.library.active_page.clone(),
                            movie,
                            frame,
                        });
                        self.multi_notify.notify_one();
                    }
                    Err(error) => {
                        self.log(format!("Background animation unavailable: {error:#}"));
                        if let Some(background) = self
                            .controller_backgrounds
                            .cache
                            .get_mut(&key)
                            .and_then(Option::as_mut)
                        {
                            background.animation = None;
                        }
                    }
                }
            }
        }
        self.controller_backgrounds
            .frame(&key)
            .map(|b| b.pixels.clone())
    }
    pub(super) fn poll_controller_background(&mut self) {
        let now = self.multi_clock.elapsed();
        if self
            .controller_backgrounds
            .next_due()
            .is_none_or(|due| now < due)
        {
            return;
        }
        if self
            .controller_backgrounds
            .active
            .as_ref()
            .is_some_and(|a| {
                a.page != self.library.active_page
                    || Some(&a.key) != self.encoder_background_key().as_ref()
            })
        {
            self.controller_backgrounds.active = None;
            return;
        }
        let result = self
            .controller_backgrounds
            .active
            .as_mut()
            .unwrap()
            .movie
            .advance(now)
            .and_then(|image| {
                image
                    .map(|image| Background::from_image(image, self.model))
                    .transpose()
            });
        match result {
            Ok(Some(frame)) => {
                self.controller_backgrounds.active.as_mut().unwrap().frame = frame;
                if let Err(error) = self.render() {
                    self.log(format!("Background frame: {error:#}"));
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.controller_backgrounds
                    .active
                    .as_mut()
                    .unwrap()
                    .movie
                    .stop();
                self.log(format!("Background animation stopped: {error:#}"));
            }
        }
    }
    pub(super) fn encoder_background_tiles(&self) -> Option<&[String]> {
        let key = self.encoder_background_key()?;
        self.controller_backgrounds
            .frame(&key)
            .map(|b| b.tiles.as_slice())
    }
    pub(super) fn encoder_background_preview(&self) -> Option<&str> {
        let key = self.encoder_background_key()?;
        self.controller_backgrounds
            .frame(&key)
            .map(|b| b.preview.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::io::Write;
    fn archive(path: &Path, uuid: &str, model: Model) -> Vec<u8> {
        let pixels = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(
            model.strip_width(),
            model.strip_height(),
            |x, y| {
                image::Rgba([
                    if x < model.strip_width() / 2 { 210 } else { 40 },
                    if y < model.strip_height() / 2 {
                        50
                    } else {
                        180
                    },
                    90,
                    128,
                ])
            },
        ));
        let mut png = Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let bytes = png.into_inner();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(serde_json::to_string(&json!({"Name":"Background","Device":{"Model":model.profile_model()},"Controllers":[{"Type":"Encoder","Background":"Images/background.png","Actions":{"0,0":{"UUID":uuid,"States":[{}],"Settings":{}}}}]})).unwrap().as_bytes()).unwrap();
        zip.start_file(
            "Images/background.png",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
        bytes
    }
    #[test]
    fn webp_background_updates_both_planes_and_stops_on_page_departure() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.model = Model::Galleon;
        host.library.current_mut().unwrap().model = Model::Galleon;
        let path = root.path().join("animated.webp");
        let bytes = crate::background_movie::tests::fixture_size(1, 720, 384);
        fs::write(&path, &bytes).unwrap();
        let profile = host.library.active_profile.clone();
        let page = host.library.active_page.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        host.hardware = Some(tx);
        host.set_controller_background(&json!({"profile":profile,"id":page,"path":path}))
            .unwrap();
        let latest = |rx: &std::sync::mpsc::Receiver<Draw>| {
            rx.try_iter()
                .filter_map(|d| {
                    if let Draw::Strip(i) = d {
                        Some(i)
                    } else {
                        None
                    }
                })
                .last()
                .unwrap()
        };
        assert_eq!(&latest(&rx).get_pixel(0, 0).0[..3], &[255, 0, 0]);
        for expected in [[127, 0, 128], [0, 0, 0]] {
            let due = host.controller_backgrounds.next_due().unwrap();
            host.multi_clock = std::time::Instant::now() - due;
            host.poll_controller_background();
            let physical = latest(&rx);
            let preview = image::load_from_memory(
                &STANDARD
                    .decode(host.encoder_background_preview().unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(physical.to_rgb8(), preview.to_rgb8());
            for (actual, expected) in physical.get_pixel(0, 0).0[..3].iter().zip(expected) {
                assert!(
                    actual.abs_diff(expected) <= 1,
                    "physical corner {:?}, expected channel {expected}",
                    physical.get_pixel(0, 0).0
                );
            }
        }
        let due = host.controller_backgrounds.next_due().unwrap();
        host.multi_clock = std::time::Instant::now() - due;
        host.poll_controller_background();
        assert!(host.controller_backgrounds.next_due().is_none());
        host.library_command(&json!({"command":"page_create","name":"Away"}))
            .unwrap();
        assert!(host.controller_backgrounds.active.is_none());
        rx.try_iter().for_each(drop);
        host.poll_controller_background();
        assert!(rx.try_recv().is_err());
        host.select_page(&page, false).unwrap();
        assert_eq!(&latest(&rx).get_pixel(0, 0).0[..3], &[255, 0, 0]);
        assert!(host.controller_backgrounds.next_due().is_some());
        let owner_due = host.controller_backgrounds.next_due();
        host.simulate = true;
        host.add_simulated_device("movie-B", Model::Galleon)
            .unwrap();
        host.with_device("movie-B",|other|{
            assert!(other.controller_backgrounds.active.is_none());
            other.set_controller_background(&json!({"profile":other.library.active_profile,"id":other.library.active_page,"path":path}))?;
            let due=other.controller_backgrounds.next_due().unwrap();other.multi_clock=std::time::Instant::now()-due;other.poll_controller_background();
            assert_ne!(other.encoder_background().unwrap().get_pixel(0,0).0,[255,0,0,255]);Ok(())
        }).unwrap();
        assert_eq!(host.controller_backgrounds.next_due(), owner_due);
        assert_eq!(
            host.encoder_background().unwrap().get_pixel(0, 0).0,
            [255, 0, 0, 255]
        );
        let exported = root.path().join("animation.streamDeckProfile");
        host.export_profile(&exported).unwrap();
        assert!(
            profile::load_bundle(&exported)
                .unwrap()
                .files()
                .any(|(_, b)| b == bytes.as_slice())
        );
    }
    #[test]
    fn background_edits_target_the_captured_page_and_embed_assets_without_rewriting_actions() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.model = Model::Galleon;
        let source = root.path().join("original.streamDeckProfile");
        let original = archive(&source, &host.actions[0].uuid, Model::Galleon);
        let profile = host.import_library(&source, None, false).unwrap();
        host.select_profile(&profile, None).unwrap();
        let page = host.library.active_page.clone();
        let actions = host
            .library
            .current()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.id == page)
            .unwrap()
            .raw["Controllers"][0]["Actions"]
            .clone();
        let image_path = root.path().join("chosen.png");
        DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 2, Rgb([230, 20, 10])))
            .save(&image_path)
            .unwrap();
        let chosen = fs::read(&image_path).unwrap();
        host.library_command(&json!({"command":"page_create","name":"Other"}))
            .unwrap();
        let other = host.library.active_page.clone();
        host.set_controller_background(&json!({"profile":profile,"id":page,"path":image_path}))
            .unwrap();
        assert_eq!(host.library.active_page, other);
        assert!(host.encoder_background_preview().is_none());
        host.select_page(&page, false).unwrap();
        let current = host
            .library
            .current()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.id == page)
            .unwrap();
        assert_eq!(current.raw["Controllers"][0]["Actions"], actions);
        let custom = current.raw["Controllers"][0]["Background"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(custom, "Images/background.png");
        assert!(
            host.retained_asset_sources()
                .contains(&current.background_source.as_ref().unwrap().source.as_path())
        );
        fs::remove_file(image_path).unwrap();
        host.controller_backgrounds = Runtime::default();
        host.render().unwrap();
        assert_eq!(
            &host.encoder_background().unwrap().get_pixel(0, 0).0[..3],
            &[230, 20, 10]
        );
        host.library_command(&json!({"command":"page_duplicate","id":page,"name":"Copy"}))
            .unwrap();
        let copy = host.library.active_page.clone();
        assert!(host.encoder_background_configured());
        host.set_controller_background(&json!({"profile":profile,"id":copy,"path":null}))
            .unwrap();
        assert!(!host.encoder_background_configured());
        host.select_page(&page, false).unwrap();
        assert!(host.encoder_background_configured());
        let before = host
            .library
            .current()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.id == page)
            .unwrap()
            .raw
            .clone();
        let bad = root.path().join("bad.png");
        fs::write(&bad, b"not an image").unwrap();
        assert!(
            host.set_controller_background(&json!({"profile":profile,"id":page,"path":bad}))
                .is_err()
        );
        host.library.current_mut().unwrap().readonly = true;
        assert!(
            host.set_controller_background(&json!({"profile":profile,"id":page,"path":null}))
                .is_err()
        );
        host.library.current_mut().unwrap().readonly = false;
        assert_eq!(
            host.library
                .current()
                .unwrap()
                .pages
                .iter()
                .find(|p| p.id == page)
                .unwrap()
                .raw,
            before
        );
        let exported = root.path().join("export.streamDeckProfile");
        host.export_profile(&exported).unwrap();
        let bundle = profile::load_bundle(&exported).unwrap();
        let configured = bundle
            .pages
            .iter()
            .find(|p| {
                p.page_raw["Controllers"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|c| c["Background"] == custom))
            })
            .unwrap();
        assert_eq!(configured.asset(&custom).unwrap(), chosen);
        assert!(bundle.files().any(|(_, b)| b == original.as_slice()));
    }
    #[test]
    fn page_background_reaches_both_planes_and_survives_provenance_and_export() {
        for model in [Model::Galleon, Model::Plus] {
            let root = tempfile::tempdir().unwrap();
            let mut host = super::super::tests::fixture(root.path());
            host.model = model;
            let source = root.path().join("source.streamDeckProfile");
            let bytes = archive(&source, &host.actions[0].uuid, model);
            let id = host.import_library(&source, None, false).unwrap();
            host.select_profile(&id, None).unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            host.hardware = Some(tx);
            host.render().unwrap();
            let strip = rx
                .try_iter()
                .find_map(|d| {
                    if let Draw::Strip(i) = d {
                        Some(i)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(&strip.get_pixel(0, 0).0[..3], &[105, 25, 45]);
            let slot = host
                .slots
                .iter()
                .find(|s| s.controller == "Encoder")
                .unwrap();
            let preview =
                image::load_from_memory(&STANDARD.decode(&slot.preview).unwrap()).unwrap();
            assert_eq!(&preview.get_pixel(0, 0).0[..3], &[105, 25, 45]);
            assert!(host.encoder_background_preview().is_some());
            let oldpage = host
                .library
                .current()
                .unwrap()
                .pages
                .iter()
                .find(|p| p.id == host.library.active_page)
                .unwrap()
                .clone();
            let origin = transfer::AssetSource {
                source: host.library.current().unwrap().source.clone().unwrap(),
                manifest: oldpage.source_manifest.clone().unwrap(),
                profile: id.clone(),
            };
            let mut copied = oldpage;
            copied.asset_source = Some(origin);
            copied.id = library::new_id();
            copied.manifest_path = library::page_path(&copied.id);
            host.library.current_mut().unwrap().source = None;
            let pin = host.library.current().unwrap().pin_page.clone();
            host.library
                .current_mut()
                .unwrap()
                .pages
                .retain(|page| page.id == pin);
            host.library
                .current_mut()
                .unwrap()
                .pages
                .push(copied.clone());
            host.library.active_page = copied.id.clone();
            host.library.current_mut().unwrap().page_order = vec![copied.id.clone()];
            host.controller_backgrounds = Runtime::default();
            host.render().unwrap();
            assert!(host.encoder_background_preview().is_some());
            let output = root.path().join("export.streamDeckProfile");
            host.export_profile(&output).unwrap();
            let bundle = profile::load_bundle(&output).unwrap();
            let page = bundle
                .pages
                .iter()
                .find(|p| p.page_raw["Controllers"][0]["Background"] == "Images/background.png")
                .unwrap();
            assert_eq!(page.asset("Images/background.png").unwrap(), bytes);
            host.library
                .current_mut()
                .unwrap()
                .pages
                .iter_mut()
                .find(|p| p.id == copied.id)
                .unwrap()
                .raw["Controllers"][0]["Background"] = json!("Images/missing.png");
            host.render().unwrap();
            assert!(host.encoder_background_preview().is_none());
            let errors = host.logs.len();
            host.render().unwrap();
            assert_eq!(host.logs.len(), errors);
        }
    }
}
