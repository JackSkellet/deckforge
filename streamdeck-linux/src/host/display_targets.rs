//! One frame renderer, independently projected onto editor and hardware targets.
//! Nested Trigger/Stack composition follows the same target as its parent frame.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Display<'a> {
    pub hardware_plane: bool,
    pub model: Model,
    pub renderer: &'a Renderer,
    pub actions: &'a [Action],
    pub plugins: &'a [Plugin],
    pub badges: &'a HashMap<String, feedback::Badge>,
    pub now: Duration,
    pub wheels: &'a wheel::Runtime,
    pub wheel_pressed: &'a HashSet<String>,
    pub progress: &'a BTreeMap<String, u8>,
}
impl Display<'_> {
    /// Child models have their own editor images even when they share a physical
    /// parent. Render these separately; do not forward child badges to that key.
    pub fn editor_children(&self, slot: &mut Slot, warnings: &mut Vec<String>) -> Result<()> {
        let active = slot.active_stack_index();
        let stacked = slot.uuid == crate::dial_stack::UUID;
        for (index, child) in slot.children.iter_mut().flatten().enumerate() {
            if stacked && index != active {
                continue;
            }
            self.editor_children(child, warnings)?;
            if child.controller == "Keypad" {
                child.preview = png(&self.picture(child, warnings)?)?;
            }
        }
        Ok(())
    }
    pub fn encoder_icon(&self, slot: &mut Slot) -> Result<String> {
        if !self.actions.iter().any(|action| action.uuid == slot.uuid) {
            return png(&appearance::unavailable_picture(self.renderer, (72, 72)));
        }
        if slot.uuid == crate::dial_stack::UUID {
            let index = slot.active_stack_index();
            if let Some(child) = slot
                .children
                .first_mut()
                .and_then(|children| children.get_mut(index))
            {
                let icon = self.encoder_icon(child)?;
                child.software_icon = icon.clone();
                return Ok(icon);
            }
        }
        let current = slot.appearances.get(&slot.state);
        let override_pixels = current.and_then(|s| s.user_pixels.as_ref()).or_else(|| {
            appearance::custom_image(slot)
                .then_some(slot.background.as_ref())
                .flatten()
        });
        if let Some(image) =
            override_pixels.or_else(|| current.and_then(|s| s.encoder_pixels.as_ref()))
        {
            return png(&image.thumbnail(144, 144));
        }
        let action = self.actions.iter().find(|a| a.uuid == slot.uuid);
        if slot.uuid == crate::action_wheel::UUID {
            return Ok(slot
                .children
                .first()
                .and_then(|g| g.get(wheel::current(slot)))
                .map(|child| child.preview.clone())
                .unwrap_or_default());
        }
        let source = action
            .and_then(|a| a.raw["Encoder"]["Icon"].as_str())
            .filter(|s| !s.is_empty());
        if let Some(source) = source {
            let plugin = action.and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin));
            let signature = format!(
                "{}:{}:{}",
                plugin.map_or(0, |p| p.asset_generation),
                plugin
                    .map(|p| p.root.display().to_string())
                    .unwrap_or_default(),
                source
            );
            let cache_key = "deckforge:encoder-software-icon";
            if slot
                .image_cache
                .get(cache_key)
                .is_none_or(|(old, _)| old != &signature)
            {
                let image = plugin.and_then(|p| {
                    let path = Path::new(source);
                    let high_resolution = if path.extension().is_some_and(|ext| {
                        matches!(
                            ext.to_str(),
                            Some("png" | "jpg" | "jpeg" | "svg" | "gif" | "webp")
                        )
                    }) {
                        path.with_file_name(format!(
                            "{}@2x.{}",
                            path.file_stem()?.to_str()?,
                            path.extension()?.to_str()?
                        ))
                        .to_string_lossy()
                        .into_owned()
                    } else {
                        format!("{source}@2x")
                    };
                    state_image(&p.root, &json!([{"Image":high_resolution}]), 0)
                        .or_else(|| state_image(&p.root, &json!([{"Image":source}]), 0))
                });
                let image = image.unwrap_or_else(|| {
                    slot.display_warnings
                        .push("Encoder icon unavailable; displaying a blank software icon".into());
                    DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1))
                });
                slot.image_cache
                    .insert(cache_key.into(), (signature, image));
            }
            return png(&slot.image_cache[cache_key].1.thumbnail(144, 144));
        }
        slot.background
            .as_ref()
            .map(|image| png(&image.thumbnail(144, 144)))
            .unwrap_or_else(|| Ok(String::new()))
    }
    pub fn picture(&self, slot: &mut Slot, warnings: &mut Vec<String>) -> Result<DynamicImage> {
        let mut picture = if !self.actions.iter().any(|action| action.uuid == slot.uuid) {
            let size = if slot.controller == "Encoder" {
                (
                    self.model.strip_width() / u32::from(self.model.dial_count()).max(1),
                    self.model.strip_height(),
                )
            } else if slot.controller == "Neo" {
                (232, 50)
            } else if self.model.key_width() == self.model.key_height() {
                // Pedal has no physical key pixels; retain its 120x120
                // software preview, as for ordinary square-key rendering.
                (120, 120)
            } else {
                (self.model.key_width(), self.model.key_height())
            };
            appearance::unavailable_picture(self.renderer, size)
        } else if slot.controller == "Neo" {
            let root = self
                .actions
                .iter()
                .find(|a| a.uuid == slot.uuid)
                .and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin))
                .map(|p| p.root.as_path());
            display::dial_image(self.renderer, slot, root, self.actions)?
        } else if slot.controller == "Encoder" {
            let stacked = slot.uuid == crate::dial_stack::UUID;
            let index = slot.active_stack_index();
            let has_child = stacked
                && slot
                    .children
                    .first()
                    .is_some_and(|children| index < children.len());
            let show_mark = has_child && slot.settings["ShowStackIcon"].as_bool().unwrap_or(true);
            let target = if has_child {
                &mut slot.children[0][index]
            } else {
                &mut *slot
            };
            let root = self
                .actions
                .iter()
                .find(|a| a.uuid == target.uuid)
                .and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin))
                .map(|p| p.root.as_path());
            let mut picture = if has_child && !self.actions.iter().any(|a| a.uuid == target.uuid) {
                let mut placeholder = target.clone();
                placeholder.layout = "$X1".into();
                placeholder.layout_cache = None;
                placeholder.feedback = json!({"title":"Unavailable"});
                placeholder.background = None;
                display::dial_image(self.renderer, &mut placeholder, None, self.actions)?
            } else if target.uuid == crate::action_wheel::UUID {
                wheel::picture(self, target)?
            } else if target.uuid == crate::action_trigger::UUID {
                trigger::picture(
                    self.renderer,
                    self.actions,
                    target,
                    self.badges,
                    self.now,
                    self.progress,
                )?
            } else {
                display::dial_image(self.renderer, target, root, self.actions)?
            };
            warnings.append(&mut target.display_warnings);
            if has_child {
                let mut preview = Cursor::new(Vec::new());
                picture.write_to(&mut preview, image::ImageFormat::Png)?;
                target.preview = STANDARD.encode(preview.into_inner());
            }
            if show_mark {
                stack::mark(&mut picture);
            }
            picture
        } else {
            let size = if self.model.key_width() != self.model.key_height() {
                (self.model.key_width(), self.model.key_height())
            } else {
                (120, 120)
            };
            let mut picture = self.renderer.key_styled_size(
                &slot.title,
                slot.background.as_ref(),
                &appearance::style(slot, self.actions),
                size,
            );
            feedback::paint_key(
                self.badges,
                &mut picture,
                slot,
                self.now,
                self.progress.get(&slot.context).copied(),
            );
            picture
        };
        if slot.controller == "Encoder" {
            feedback::paint_dial(self.badges, &mut picture, slot, self.now);
        }
        if slot.controller == "Keypad" && self.model.capabilities().key_images {
            picture = picture.resize_exact(
                self.model.key_width(),
                self.model.key_height(),
                image::imageops::FilterType::Lanczos3,
            );
        }

        warnings.append(&mut slot.display_warnings);
        Ok(picture)
    }
}
pub(super) fn png(image: &DynamicImage) -> Result<String> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(STANDARD.encode(png.into_inner()))
}

/// Skip all image/tree clones for the normal shared-target case.
pub(super) fn has_hardware(slot: &Slot) -> bool {
    slot.appearances
        .get(&slot.state)
        .is_some_and(|state| state.hardware.is_some())
        || slot.children.iter().flatten().any(has_hardware)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    fn solid(rgb: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(120, 120, Rgb(rgb)))
    }
    fn decode(preview: &str) -> DynamicImage {
        image::load_from_memory(&STANDARD.decode(preview).unwrap()).unwrap()
    }
    fn split(slot: &mut Slot) {
        slot.title.clear();
        slot.background = Some(solid([0, 0, 255]));
        slot.appearances.insert(
            0,
            appearance::StateAppearance {
                plugin_title: Some(String::new()),
                plugin_pixels: Some(solid([0, 0, 255])),
                hardware: Some(appearance::PluginAppearance {
                    title: Some(String::new()),
                    pixels: Some(solid([255, 0, 0])),
                }),
                ..Default::default()
            },
        );
    }
    fn key_frame(receiver: &std::sync::mpsc::Receiver<Draw>) -> DynamicImage {
        receiver
            .try_iter()
            .find_map(|draw| match draw {
                Draw::Key(0, image) => Some(image),
                _ => None,
            })
            .unwrap()
    }
    #[test]
    fn removed_encoder_plugin_uses_placeholder_without_erasing_saved_artwork() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.slots[0].controller = "Encoder".into();
        let artwork = solid([220, 10, 40]);
        host.slots[0].appearances.insert(
            0,
            appearance::StateAppearance {
                encoder_pixels: Some(artwork.clone()),
                ..Default::default()
            },
        );
        host.render().unwrap();
        let previous = host.slots[0].software_icon.clone();
        let uuid = host.slots[0].uuid.clone();
        host.actions.retain(|a| a.uuid != uuid);
        host.render().unwrap();
        assert_ne!(host.slots[0].software_icon, previous);
        assert_eq!(
            host.slots[0].appearances[&0]
                .encoder_pixels
                .as_ref()
                .unwrap()
                .as_bytes(),
            artwork.as_bytes()
        );
    }
    #[test]
    fn key_editor_and_physical_frames_are_independent_and_user_override_wins() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        split(&mut host.slots[0]);
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].preview).get_pixel(60, 60).0,
            [0, 0, 255, 255]
        );
        assert_eq!(key_frame(&receiver).get_pixel(60, 60).0, [255, 0, 0, 255]);
        assert_eq!(
            host.slots[0]
                .background
                .as_ref()
                .unwrap()
                .get_pixel(60, 60)
                .0,
            [0, 0, 255, 255]
        );
        let current = host.slots[0].appearances.get_mut(&0).unwrap();
        current.user_image = Some(root.path().join("user.png"));
        current.user_pixels = Some(solid([0, 255, 0]));
        host.slots[0].background = Some(solid([0, 255, 0]));
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].preview).get_pixel(60, 60).0,
            [0, 255, 0, 255]
        );
        assert_eq!(key_frame(&receiver).get_pixel(60, 60).0, [0, 255, 0, 255]);
        // Returning to another state cannot inherit the inactive state's target split.
        host.slots[0].state = 1;
        host.slots[0].background = Some(solid([255, 255, 0]));
        host.slots[0].appearances.insert(
            1,
            appearance::StateAppearance {
                plugin_pixels: Some(solid([255, 255, 0])),
                plugin_title: Some(String::new()),
                ..Default::default()
            },
        );
        assert!(!has_hardware(&host.slots[0]));
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].preview).to_rgba8(),
            key_frame(&receiver).to_rgba8()
        );
    }
    #[test]
    fn trigger_inside_stack_composes_each_target_and_keeps_child_previews_software_only() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        host.actions.extend([trigger::action(), stack::action()]);
        let mut child = host.slots[0].clone();
        child.context = "nested.key".into();
        split(&mut child);
        let mut trigger = host.slots[0].clone();
        trigger.context = "nested.trigger".into();
        trigger.controller = "Encoder".into();
        trigger.uuid = crate::action_trigger::UUID.into();
        trigger.children = vec![vec![], vec![child], vec![]];
        let parent = &mut host.slots[0];
        parent.controller = "Encoder".into();
        parent.uuid = crate::dial_stack::UUID.into();
        parent.settings = json!({"CurrentIdx":0,"ShowStackIcon":false});
        parent.children = vec![vec![trigger]];
        host.render().unwrap();
        let physical = receiver
            .try_iter()
            .find_map(|draw| match draw {
                Draw::Strip(image) => Some(image),
                _ => None,
            })
            .unwrap();
        assert_eq!(physical.get_pixel(100, 35).0, [255, 0, 0, 255]);
        assert_eq!(
            decode(&host.slots[0].preview).get_pixel(100, 35).0,
            [0, 0, 255, 255]
        );
        let trigger = &host.slots[0].children[0][0];
        assert_eq!(
            decode(&trigger.preview).get_pixel(100, 35).0,
            [0, 0, 255, 255]
        );
        assert_eq!(
            decode(&trigger.children[1][0].preview).get_pixel(60, 60).0,
            [0, 0, 255, 255]
        );
    }
    #[test]
    fn neo_infobar_draw_uses_hardware_projection_without_replacing_preview() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        let (sender, receiver) = std::sync::mpsc::channel();
        host.hardware = Some(sender);
        host.model = Model::Neo;
        let slot = &mut host.slots[0];
        split(slot);
        slot.controller = "Neo".into();
        slot.layout = "$Neo".into();
        slot.layout_cache = Some(
            json!({"id":"fixture","controller":"Neo","items":[{"key":"icon","type":"pixmap","rect":[0,0,232,50]}]}),
        );
        host.render().unwrap();
        let hardware = receiver
            .try_iter()
            .find_map(|draw| match draw {
                Draw::Infobar(image) => Some(image),
                _ => None,
            })
            .unwrap();
        assert_eq!(hardware.get_pixel(100, 25).0, [255, 0, 0, 255]);
        assert_eq!(
            decode(&host.slots[0].preview).get_pixel(100, 25).0,
            [0, 0, 255, 255]
        );
    }
    #[test]
    fn encoder_software_icon_is_separate_from_feedback_and_exists_without_lcd() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        solid([0, 255, 0])
            .save(root.path().join("dial-icon@2x.png"))
            .unwrap();
        host.actions[0].raw["Encoder"] = json!({"Icon":"dial-icon"});
        let slot = &mut host.slots[0];
        slot.controller = "Encoder".into();
        slot.layout = "$X1".into();
        slot.title.clear();
        slot.background = Some(solid([255, 0, 0]));
        slot.feedback =
            json!({"icon":format!("data:image/png;base64,{}",png(&solid([0,0,255])).unwrap())});
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].software_icon).get_pixel(60, 60).0,
            [0, 255, 0, 255]
        );
        assert_eq!(
            decode(&host.slots[0].preview).get_pixel(100, 60).0,
            [0, 0, 255, 255]
        );
        host.slots[0]
            .appearances
            .entry(0)
            .or_default()
            .encoder_pixels = Some(solid([255, 255, 0]));
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].software_icon).get_pixel(60, 60).0,
            [255, 255, 0, 255]
        );
        assert_eq!(
            decode(&host.slots[0].preview).get_pixel(100, 60).0,
            [0, 0, 255, 255]
        );
        host.model = Model::Studio;
        host.render().unwrap();
        assert!(host.slots[0].preview.is_empty());
        assert_eq!(
            decode(&host.slots[0].software_icon).get_pixel(60, 60).0,
            [255, 255, 0, 255]
        );
    }
    #[test]
    fn selected_stack_child_provides_software_icon_and_custom_image_precedence() {
        let root = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(root.path());
        host.actions.push(stack::action());
        let mut child = host.slots[0].clone();
        child.controller = "Encoder".into();
        child.layout = "$X1".into();
        child.appearances.entry(0).or_default().encoder_pixels = Some(solid([0, 0, 255]));
        let mut other = child.clone();
        other.context = "other".into();
        other.appearances.get_mut(&0).unwrap().encoder_pixels = Some(solid([255, 0, 0]));
        let parent = &mut host.slots[0];
        parent.controller = "Encoder".into();
        parent.uuid = crate::dial_stack::UUID.into();
        parent.settings = json!({"CurrentIdx":1});
        parent.children = vec![vec![child, other]];
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].software_icon).get_pixel(60, 60).0,
            [255, 0, 0, 255]
        );
        let child = &mut host.slots[0].children[0][1];
        child.appearances.get_mut(&0).unwrap().user_pixels = Some(solid([0, 255, 0]));
        host.render().unwrap();
        assert_eq!(
            decode(&host.slots[0].software_icon).get_pixel(60, 60).0,
            [0, 255, 0, 255]
        );
    }
}
