//! User overrides and SDK updates are independent and scoped to an action state.
use super::*;
use crate::appearance::TitleStyle;
use anyhow::ensure;
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct PluginAppearance {
    pub title: Option<String>,
    #[serde(default, with = "image_serde")]
    pub pixels: Option<DynamicImage>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayTarget {
    Both,
    Hardware,
    Software,
}
impl DisplayTarget {
    fn parse(payload: &Value) -> Result<Self> {
        match payload.get("target") {
            None => Ok(Self::Both),
            Some(value) => match value.as_u64() {
                Some(0) => Ok(Self::Both),
                Some(1) => Ok(Self::Hardware),
                Some(2) => Ok(Self::Software),
                _ => anyhow::bail!("Display target must be 0, 1 or 2"),
            },
        }
    }
}
fn pixel_bytes(pixels: Option<&DynamicImage>) -> usize {
    pixels.map_or(0, |image| image.as_bytes().len())
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StateAppearance {
    pub user_title: Option<String>,
    pub imported_title: bool,
    pub imported_image: bool,
    pub title_reset: bool,
    pub image_reset: bool,
    pub user_image: Option<PathBuf>,
    #[serde(default, with = "image_serde")]
    pub user_pixels: Option<DynamicImage>,
    pub plugin_title: Option<String>,
    #[serde(default, with = "image_serde")]
    pub plugin_pixels: Option<DynamicImage>,
    /// None shares the software runtime values. Split before the first targeted write.
    pub hardware: Option<PluginAppearance>,
    /// The encoder's circular software canvas is separate from its LCD layout.
    #[serde(default, with = "image_serde")]
    pub encoder_pixels: Option<DynamicImage>,
}
impl StateAppearance {
    fn split_hardware(&mut self) {
        if self.hardware.is_none() {
            self.hardware = Some(PluginAppearance {
                title: self.plugin_title.clone(),
                pixels: self.plugin_pixels.clone(),
            });
        }
    }
    fn pixel_bytes(&self) -> usize {
        pixel_bytes(self.user_pixels.as_ref())
            + pixel_bytes(self.plugin_pixels.as_ref())
            + self
                .hardware
                .as_ref()
                .map_or(0, |h| pixel_bytes(h.pixels.as_ref()))
            + pixel_bytes(self.encoder_pixels.as_ref())
    }
    fn projected_bytes(&self, target: DisplayTarget, image: Option<usize>, encoder: bool) -> usize {
        if encoder {
            return self.pixel_bytes() - pixel_bytes(self.encoder_pixels.as_ref())
                + image.unwrap_or_else(|| pixel_bytes(self.encoder_pixels.as_ref()));
        }
        let software = pixel_bytes(self.plugin_pixels.as_ref());
        let hardware = self
            .hardware
            .as_ref()
            .map_or(software, |h| pixel_bytes(h.pixels.as_ref()));
        pixel_bytes(self.user_pixels.as_ref())
            + pixel_bytes(self.encoder_pixels.as_ref())
            + if target == DisplayTarget::Hardware {
                software
            } else {
                image.unwrap_or(software)
            }
            + if self.hardware.is_some() || target != DisplayTarget::Both {
                if target == DisplayTarget::Software {
                    hardware
                } else {
                    image.unwrap_or(hardware)
                }
            } else {
                0
            }
    }
}

pub(super) type States = BTreeMap<u64, StateAppearance>;

/// Missing plugin artwork is derived, never written into saved action state.
pub(super) fn unavailable_picture(renderer: &Renderer, size: (u32, u32)) -> DynamicImage {
    renderer.key_styled_size(
        "?",
        None,
        &TitleStyle {
            font_size: 34.,
            title_alignment: "middle".into(),
            font_style: "Bold".into(),
            ..Default::default()
        },
        size,
    )
}

pub(super) fn count(slot: &Slot, actions: &[Action]) -> usize {
    slot.states
        .as_array()
        .map_or(1, Vec::len)
        .max(
            actions
                .iter()
                .find(|a| a.uuid == slot.uuid)
                .and_then(|a| a.raw["States"].as_array())
                .map_or(1, Vec::len),
        )
        .max(1)
}
fn metadata(slot: &Slot, actions: &[Action], state: u64) -> Value {
    let mut value = actions
        .iter()
        .find(|a| a.uuid == slot.uuid)
        .map(|a| a.raw["States"][state as usize].clone())
        .unwrap_or(Value::Null);
    if !value.is_object() {
        value = json!({});
    }
    if let Some(fields) = slot.states[state as usize].as_object() {
        value.as_object_mut().unwrap().extend(fields.clone());
    }
    value
}
pub(super) fn style(slot: &Slot, actions: &[Action]) -> TitleStyle {
    TitleStyle::from_state(&metadata(slot, actions, slot.state)).unwrap_or_default()
}
fn event_style(slot: &Slot, actions: &[Action]) -> TitleStyle {
    let mut value = style(slot, actions);
    if value.font_family.is_empty() {
        value.font_family = TitleStyle::default().font_family;
    }
    value
}
pub(super) fn explicit_style(slot: &Slot, actions: &[Action]) -> bool {
    let value = metadata(slot, actions, slot.state);
    [
        "FontFamily",
        "FontSize",
        "FontStyle",
        "FontUnderline",
        "ShowTitle",
        "TitleAlignment",
        "TitleColor",
    ]
    .iter()
    .any(|key| value.get(*key).is_some())
}
fn builtin_title_owner(uuid: &str) -> bool {
    crate::system_actions::supported(uuid)
        || crate::audio_output::supported(uuid)
        || mixer_actions::supported(uuid)
        || multiaction::container(uuid)
        || matches!(
            uuid,
            crate::brightness::UUID
                | crate::dial_stack::UUID
                | crate::action_trigger::UUID
                | crate::action_wheel::UUID
                | screensaver::UUID
                | library::OPEN_FOLDER
                | library::BACK_FOLDER
                | "com.elgato.streamdeck.page.next"
                | "com.elgato.streamdeck.page.previous"
                | "com.elgato.streamdeck.page.goto"
                | "com.elgato.streamdeck.page.indicator"
        )
}
pub(super) fn migrate(slot: &mut Slot, actions: &[Action]) {
    if (slot.title_changed || slot.image_changed) && !slot.appearances.contains_key(&slot.state) {
        let user = slot.title_changed
            && builtin_title_owner(&slot.uuid)
            && title_enabled(slot, actions)
            && !slot.title.is_empty();
        slot.appearances.insert(
            slot.state,
            StateAppearance {
                user_title: user.then(|| slot.title.clone()),
                plugin_title: (slot.title_changed && !user).then(|| slot.title.clone()),
                plugin_pixels: slot
                    .image_changed
                    .then(|| slot.background.clone())
                    .flatten(),
                ..Default::default()
            },
        );
    }
    if let Some(states) = slot.raw["States"].as_array() {
        let title_enabled = title_enabled(slot, actions);
        for (index, state) in states.iter().take(1024).enumerate() {
            let image = state["Image"].as_str().is_some_and(|s| !s.is_empty());
            let title = state["Title"]
                .as_str()
                .filter(|s| !s.is_empty() && title_enabled);
            if (image || title.is_some()) && !slot.appearances.contains_key(&(index as u64)) {
                slot.appearances.insert(
                    index as u64,
                    StateAppearance {
                        user_title: title.map(str::to_owned),
                        imported_title: title.is_some(),
                        imported_image: image,
                        ..Default::default()
                    },
                );
            }
        }
    }
    if slot.controller == "Encoder" {
        // Earlier Deckforge builds stored app-only dial setImage in the LCD
        // fallback cache. Move those runtime pixels without touching user images.
        for (state, entry) in &mut slot.appearances {
            if entry.encoder_pixels.is_none() && entry.plugin_pixels.is_some() {
                entry.encoder_pixels = entry.plugin_pixels.take();
                if *state == slot.state {
                    slot.background = None;
                    slot.image_changed = false;
                }
            }
        }
    }
    for child in slot.children.iter_mut().flatten() {
        migrate(child, actions);
    }
}
pub(super) fn apply_user_titles(slot: &mut Slot, actions: &[Action]) {
    if slot.appearances.get(&slot.state).is_some_and(|s| {
        s.user_title.is_some()
            || s.title_reset
            || (s.plugin_title.is_some() && !builtin_title_owner(&slot.uuid))
    }) {
        slot.title = title(slot, actions);
    }
    for child in slot.children.iter_mut().flatten() {
        apply_user_titles(child, actions);
    }
}

pub(super) fn title_enabled(slot: &Slot, actions: &[Action]) -> bool {
    actions
        .iter()
        .find(|a| a.uuid == slot.uuid)
        .is_none_or(|a| a.raw["UserTitleEnabled"] != false)
}
pub(super) fn title(slot: &Slot, actions: &[Action]) -> String {
    let current = slot.appearances.get(&slot.state);
    if let Some(title) = current.and_then(|s| s.user_title.as_ref().or(s.plugin_title.as_ref())) {
        return title.clone();
    }
    if current.is_some_and(|s| s.title_reset) {
        return actions
            .iter()
            .find(|a| a.uuid == slot.uuid)
            .and_then(|a| a.raw["States"][slot.state as usize]["Title"].as_str())
            .unwrap_or("")
            .into();
    }
    metadata(slot, actions, slot.state)["Title"]
        .as_str()
        .unwrap_or("")
        .into()
}
pub(super) fn custom_image(slot: &Slot) -> bool {
    slot.appearances
        .get(&slot.state)
        .is_some_and(|s| s.user_image.is_some() || (s.imported_image && !s.image_reset))
}
pub(super) fn snapshot(slot: &Slot, actions: &[Action]) -> Value {
    let current = slot.appearances.get(&slot.state);
    json!({"title_parameters":style(slot,actions),"title_custom":current.is_some_and(|s|s.user_title.is_some()),"image_custom":custom_image(slot)})
}
pub(super) fn pixels(slot: &Slot, state: u64) -> Option<DynamicImage> {
    slot.appearances
        .get(&state)
        .and_then(|s| {
            s.user_pixels.as_ref().or({
                if s.imported_image && !s.image_reset {
                    None
                } else {
                    s.plugin_pixels.as_ref()
                }
            })
        })
        .cloned()
}
fn check(request: &Value, slot: &Slot, actions: &[Action]) -> Result<()> {
    for name in ["reset_title", "reset_image"] {
        if let Some(value) = request.get(name) {
            ensure!(value.is_boolean(), "Reset flags must be boolean");
        }
    }
    ensure!(
        !(request.get("title").is_some() && request["reset_title"] == true),
        "Choose a title or reset it, not both"
    );
    let modern = request.get("title_parameters").is_some()
        || request.get("image_path").is_some()
        || request.get("reset_image").is_some()
        || request.get("reset_title").is_some()
        || request.get("state").is_some();
    if modern {
        ensure!(
            request.get("expected_state").is_some(),
            "Expected state is required for appearance edits"
        );
    }
    if let Some(state) = request.get("expected_state") {
        ensure!(
            state.as_u64() == Some(slot.state),
            "Action state changed; reload appearance before applying"
        );
    }
    if let Some(state) = request.get("state") {
        ensure!(
            state
                .as_u64()
                .is_some_and(|s| s < count(slot, actions) as u64 && s < 1024),
            "Target state out of range"
        );
    }
    ensure!(
        slot.state < 1024 && slot.state < count(slot, actions) as u64,
        "Invalid action state"
    );
    if request.get("title").is_some() || request["reset_title"] == true {
        ensure!(
            title_enabled(slot, actions),
            "This action reserves its title for the plugin"
        );
    }
    ensure!(
        !(request.get("image_path").is_some_and(|v| !v.is_null())
            && request["reset_image"] == true),
        "Choose an icon or reset it, not both"
    );
    Ok(())
}
struct Prepared {
    slot: Slot,
    changed_title: bool,
}
fn prepare(
    request: &Value,
    mut slot: Slot,
    actions: &[Action],
    state_path: &Path,
    save: bool,
) -> Result<Prepared> {
    let state = slot.state;
    let other_pixels: usize = slot
        .appearances
        .iter()
        .map(|(index, a)| {
            a.pixel_bytes()
                - if *index == state {
                    pixel_bytes(a.user_pixels.as_ref())
                } else {
                    0
                }
        })
        .sum();
    let old_title = slot.title.clone();
    if !slot.appearances.contains_key(&state) {
        slot.appearances.insert(
            state,
            StateAppearance {
                plugin_title: Some(old_title),
                plugin_pixels: if slot.image_changed {
                    slot.background.clone()
                } else {
                    None
                },
                ..Default::default()
            },
        );
    }
    let base_style = style(&slot, actions);
    if !slot.states.is_array() {
        slot.states = json!([]);
    }
    let states = slot.states.as_array_mut().unwrap();
    states.resize_with(states.len().max(state as usize + 1), || json!({}));
    if !states[state as usize].is_object() {
        states[state as usize] = json!({});
    }
    let changed_title = request.get("title").is_some()
        || request["reset_title"] == true
        || request.get("title_parameters").is_some();
    if let Some(value) = request.get("title_parameters") {
        let patch = value
            .as_object()
            .context("Title parameters must be an object")?;
        let mut merged = serde_json::to_value(base_style)?;
        for (key, value) in patch {
            ensure!(merged.get(key).is_some(), "Unknown title parameter {key}");
            merged[key] = value.clone();
        }
        let style: TitleStyle =
            serde_json::from_value(merged).context("Invalid title parameters")?;
        style.validate()?;
        style.write_state(&mut states[state as usize])?;
    }
    let current = slot.appearances.get_mut(&state).unwrap();
    if request["reset_title"] == true {
        current.user_title = None;
        current.title_reset = current.imported_title;
    } else if let Some(value) = request.get("title") {
        let value = value.as_str().context("Title must be text")?;
        ensure!(value.len() <= 4096, "Title exceeds 4096 bytes");
        current.user_title = (!value.is_empty()).then(|| value.to_owned());
        if current.user_title.is_none() {
            current.title_reset = current.imported_title;
        }
    }
    if request["reset_image"] == true || request.get("image_path") == Some(&Value::Null) {
        if current.user_image.is_none() && current.imported_image {
            current.image_reset = true;
        }
        current.user_image = None;
        current.user_pixels = None;
    } else if let Some(value) = request.get("image_path") {
        let path = value
            .as_str()
            .filter(|v| !v.is_empty())
            .context("Image path must be nonempty text or null")?;
        let (name, bytes) = crate::resources::read_regular(Path::new(path))?;
        ensure!(bytes.len() <= 8 * 1024 * 1024, "Action icon exceeds 8 MiB");
        let svg = Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        let image = display::decode_image(&bytes, svg)?;
        ensure!(
            u64::from(image.width()) * u64::from(image.height()) <= 16 * 1024 * 1024,
            "Action icon exceeds 16 megapixels"
        );
        ensure!(
            other_pixels.saturating_add(image.as_bytes().len()) <= 64 * 1024 * 1024,
            "Action image states exceed 64 MiB decoded pixels"
        );
        let stored = if save {
            use sha2::{Digest, Sha256};
            let id = format!("{:x}", Sha256::digest(&bytes));
            crate::resources::materialize(
                &state_path
                    .parent()
                    .context("State directory missing")?
                    .join("appearance-assets"),
                "icons",
                &id,
                &name,
                &bytes,
            )?
        } else {
            PathBuf::from(path)
        };
        current.user_image = Some(stored);
        current.user_pixels = Some(image);
    }
    slot.title = title(&slot, actions);
    slot.background = pixels(&slot, state).or(slot.background);
    // The user image has its own byte-preserving export path. Existing SDK
    // flags remain compatible with older persisted state.
    Ok(Prepared {
        slot,
        changed_title,
    })
}
impl Controller {
    fn appearance_prepare(&self, request: &Value, save: bool) -> Result<(Prepared, String)> {
        let (slot, actions, path, device) = {
            let mut host = self.shared.lock().unwrap();
            ensure!(host.running, "Host has stopped");
            let context = request["context"]
                .as_str()
                .context("Missing action context")?;
            let owner = host
                .context_device(context)
                .context("Unknown action device")?;
            ensure!(
                request.get("device").is_none_or(|id| id == &owner),
                "Action belongs to another device"
            );
            host.with_device(&owner, |host| {
                if save {
                    host.editable_context(context)?;
                }
                let mut slot = host
                    .visible_slot(
                        request["context"]
                            .as_str()
                            .context("Missing action context")?,
                    )
                    .context("Action is not visible")?
                    .clone();
                ensure!(
                    slot.controller != "Encoder" || slot.model.capabilities().strip,
                    "This encoder has no display to edit"
                );
                migrate(&mut slot, &host.actions);
                check(request, &slot, &host.actions)?;
                if let Some(target) = request["state"].as_u64().filter(|s| *s != slot.state) {
                    slot.state = target;
                    slot.title = title(&slot, &host.actions);
                    slot.background = host.resolve_state_image(&slot, target);
                }
                if request["reset_image"] == true || request.get("image_path") == Some(&Value::Null)
                {
                    if let Some(current) = slot.appearances.get_mut(&slot.state) {
                        if current.user_image.is_none() && current.imported_image {
                            current.image_reset = true;
                        }
                        current.user_pixels = None;
                    }
                    slot.background = host.resolve_state_image(&slot, slot.state);
                }
                Ok((
                    slot,
                    host.actions.clone(),
                    host.state_path.clone(),
                    host.device.clone(),
                ))
            })?
        };
        let prepared = prepare(request, slot, &actions, &path, save)?;
        if save {
            // Warm the shared font cache before acquiring the engine lock for
            // commit; a newly selected family may require bounded font matching.
            let renderer = Renderer::new()?;
            let _ = renderer.font_info(&style(&prepared.slot, &actions));
        }
        Ok((prepared, device))
    }
    pub fn appearance_preview(&self, request: Value) -> Result<Value> {
        let (prepared, device) = self.appearance_prepare(&request, false)?;
        let (actions, root) = {
            let mut host = self.shared.lock().unwrap();
            host.with_device(&device, |host| {
                ensure!(host.running, "Host stopped");
                let current = host
                    .visible_slot(&prepared.slot.context)
                    .context("Action is no longer visible")?;
                check(&request, current, &host.actions)?;
                let root = host
                    .actions
                    .iter()
                    .find(|a| a.uuid == prepared.slot.uuid)
                    .and_then(|a| host.plugins.iter().find(|p| p.id == a.plugin))
                    .map(|p| p.root.clone());
                Ok((host.actions.clone(), root))
            })?
        };
        let mut slot = prepared.slot;
        let renderer = Renderer::new()?;
        let style = style(&slot, &actions);
        let mut picture = if matches!(slot.controller.as_str(), "Encoder" | "Neo") {
            display::dial_image(&renderer, &mut slot, root.as_deref(), &actions)?
        } else {
            let size = if slot.model.key_width() != slot.model.key_height() {
                (slot.model.key_width(), slot.model.key_height())
            } else {
                (120, 120)
            };
            renderer.key_styled_size(&slot.title, slot.background.as_ref(), &style, size)
        };
        if slot.controller == "Keypad" && slot.model.capabilities().key_images {
            picture = picture.resize_exact(
                slot.model.key_width(),
                slot.model.key_height(),
                image::imageops::FilterType::Lanczos3,
            );
        }
        let mut png = Cursor::new(Vec::new());
        picture.write_to(&mut png, image::ImageFormat::Png)?;
        let flags = snapshot(&slot, &actions);
        Ok(
            json!({"preview":format!("data:image/png;base64,{}",STANDARD.encode(png.into_inner())),"state":slot.state,"title":slot.title,"title_parameters":style,"title_custom":flags["title_custom"],"image_custom":flags["image_custom"],"font_info":renderer.font_info(&style)}),
        )
    }
    pub(super) fn appearance_command(&self, request: Value) -> Result<()> {
        let (prepared, device) = self.appearance_prepare(&request, true)?;
        let mut host = self.shared.lock().unwrap();
        host.with_device(&device,|host| {
        ensure!(
            host.running,
            "Host stopped"
        );
        host.editable_context(&prepared.slot.context)?;
        let current = host
            .visible_slot(&prepared.slot.context)
            .context("Action is no longer visible")?;
        check(&request, current, &host.actions)?;
        // Reapply only appearance fields to the latest action, retaining settings
        // and plugin updates received while the chosen file was being decoded.
        let original = current.clone();
        let mut replacement = original.clone();
        migrate(&mut replacement, &host.actions);
        let state = prepared.slot.state;
        let incoming = prepared.slot.appearances[&state].clone();
        let entry = replacement
            .appearances
            .entry(state)
            .or_insert_with(|| incoming.clone());
        if request.get("title").is_some() || request["reset_title"] == true {
            entry.user_title = incoming.user_title;
            entry.title_reset = incoming.title_reset;
        }
        if request.get("image_path").is_some() || request["reset_image"] == true {
            entry.user_image = incoming.user_image;
            entry.user_pixels = incoming.user_pixels;
            entry.image_reset = incoming.image_reset;
        }
        if request.get("title_parameters").is_some() {
            if !replacement.states.is_array() {
                replacement.states = json!([]);
            }
            let states = replacement.states.as_array_mut().unwrap();
            states.resize_with(states.len().max(state as usize + 1), || json!({}));
            states[state as usize] = prepared.slot.states[state as usize].clone();
            if replacement
                .raw
                .as_object()
                .is_some_and(|raw| !raw.is_empty())
            {
                if !replacement.raw["States"].is_array() {
                    replacement.raw["States"] = json!([]);
                }
                let raw_states = replacement.raw["States"].as_array_mut().unwrap();
                raw_states.resize_with(raw_states.len().max(state as usize + 1), || json!({}));
                if !raw_states[state as usize].is_object() {
                    raw_states[state as usize] = json!({});
                }
                TitleStyle::from_state(&prepared.slot.states[state as usize])?
                    .write_state(&mut raw_states[state as usize])?;
            }
        }
        replacement.title = title(&replacement, &host.actions);
        replacement.background = host.resolve_state_image(&replacement, replacement.state);
        host.store_slot(replacement.clone());
        if let Err(error) = host.persist() {
            host.store_slot(original);
            return Err(error);
        }
        if prepared.changed_title {
            let mut edited = replacement.clone();
            edited.state = state;
            edited.title = title(&edited, &host.actions);
            host.emit(&edited,"titleParametersDidChange",json!({"title":edited.title,"titleParameters":event_style(&edited,&host.actions),"state":state}));
        }
        host.render()
        })
    }
}

// Kept for internal legacy callers; image decoding always uses Controller's
// preparation path outside the host mutex.
pub(super) fn legacy(host: &mut Host, request: &Value) -> Result<()> {
    ensure!(
        request.get("image_path").is_none(),
        "Image edits require controller preparation"
    );
    host.editable()?;
    let slot = host
        .visible_slot(request["context"].as_str().context("Missing context")?)
        .context("Unknown context")?
        .clone();
    check(request, &slot, &host.actions)?;
    let prepared = prepare(request, slot, &host.actions, &host.state_path, false)?;
    host.store_slot(prepared.slot.clone());
    host.persist()?;
    if prepared.changed_title {
        host.emit(&prepared.slot,"titleParametersDidChange",json!({"title":prepared.slot.title,"titleParameters":event_style(&prepared.slot,&host.actions),"state":prepared.slot.state}));
    }
    host.render()
}

impl Host {
    /// A disposable physical view. Saved state and editor previews stay canonical
    /// software values; the same renderer handles both surfaces.
    pub(super) fn hardware_projection(&mut self, mut slot: Slot) -> Slot {
        fn project(host: &mut Host, slot: &mut Slot) {
            if let Some(entry) = slot.appearances.get_mut(&slot.state)
                && let Some(hardware) = entry.hardware.take()
            {
                entry.plugin_title = hardware.title;
                entry.plugin_pixels = hardware.pixels;
                slot.title = title(slot, &host.actions);
                slot.background = host.resolve_state_image(slot, slot.state);
            }
            for child in slot.children.iter_mut().flatten() {
                project(host, child);
            }
        }
        project(self, &mut slot);
        slot
    }

    pub(super) fn apply_plugin_display(
        &mut self,
        slot: &mut Slot,
        root: &Path,
        event: &str,
        payload: &Value,
    ) -> Result<()> {
        let target = DisplayTarget::parse(payload)?;
        let encoder_image = slot.controller == "Encoder" && event == "setImage";
        if slot.controller == "Encoder" {
            ensure!(
                target != DisplayTarget::Hardware,
                "Encoder setImage updates the app icon; use setFeedback for the touch display"
            );
            ensure!(
                event != "setTitle" || target == DisplayTarget::Both,
                "Targeted encoder setTitle is not established; use setFeedback for its title"
            );
        }
        let field = if event == "setImage" {
            "image"
        } else {
            "title"
        };
        let value = match payload.get(field) {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .with_context(|| format!("{field} must be text or null"))?,
            ),
        };
        if event == "setTitle" {
            ensure!(
                value.is_none_or(|v| v.len() <= 4096),
                "Title exceeds 4096 bytes"
            );
        }
        migrate(slot, &self.actions);
        let count = count(slot, &self.actions);
        ensure!(count <= 1024, "Action state count exceeds limit");
        let states: BTreeSet<_> = if let Some(state) = payload.get("state") {
            let state = state.as_u64().context("Invalid state")?;
            ensure!(state < count as u64 && state < 1024, "State out of range");
            BTreeSet::from([state])
        } else {
            (0..count as u64).collect()
        };
        let image = if event == "setImage" {
            value
                .filter(|v| !v.is_empty())
                .map(|value| load_image(root, value))
                .transpose()?
        } else {
            None
        };
        // Calculate the complete result before cloning pixels across states or
        // splitting a shared plane. Title-only writes can also require a split.
        let image_size = (event == "setImage").then(|| pixel_bytes(image.as_ref()));
        let all_states: BTreeSet<_> = slot
            .appearances
            .keys()
            .copied()
            .chain(states.iter().copied())
            .collect();
        let total: usize = all_states
            .iter()
            .map(|state| {
                let empty = StateAppearance::default();
                let entry = slot.appearances.get(state).unwrap_or(&empty);
                if states.contains(state) {
                    entry.projected_bytes(target, image_size, encoder_image)
                } else {
                    entry.pixel_bytes()
                }
            })
            .sum();
        ensure!(
            total <= 64 * 1024 * 1024,
            "Action image states exceed 64 MiB decoded pixels"
        );
        let affects_current = states.contains(&slot.state);
        for state in states {
            let entry = slot.appearances.entry(state).or_default();
            if encoder_image {
                entry.encoder_pixels = image.clone();
                continue;
            }
            if target != DisplayTarget::Both {
                entry.split_hardware();
            }
            if event == "setTitle" {
                let title = value.map(str::to_owned);
                if target != DisplayTarget::Hardware {
                    entry.plugin_title = title.clone();
                }
                if target != DisplayTarget::Software
                    && let Some(hardware) = &mut entry.hardware
                {
                    hardware.title = title;
                }
            } else {
                if target != DisplayTarget::Hardware {
                    entry.plugin_pixels = image.clone();
                }
                if target != DisplayTarget::Software
                    && let Some(hardware) = &mut entry.hardware
                {
                    hardware.pixels = image.clone();
                }
            }
        }
        if affects_current && target != DisplayTarget::Hardware {
            if event == "setTitle" {
                slot.title = title(slot, &self.actions);
                slot.title_changed = false;
            } else {
                slot.background = self.resolve_state_image(slot, slot.state);
                slot.image_changed = false;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Controller) {
        let dir = tempfile::tempdir().unwrap();
        let mut host = super::super::tests::fixture(dir.path());
        host.slots[0].states = json!([{"Title":"Zero","Future":4},{"Title":"One"}]);
        host.slots[0].title = "Zero".into();
        host.sync_visible();
        (
            dir,
            Controller {
                shared: Arc::new(Mutex::new(host)),
            },
        )
    }
    fn request(state: u64) -> Value {
        json!({"command":"appearance","context":"test.context","expected_state":0,"state":state})
    }
    fn svg(dir: &Path, name: &str, color: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path,format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="{color}"/></svg>"#)).unwrap();
        path
    }
    #[test]
    fn targeted_titles_reset_independently_and_blank_is_not_a_reset() {
        let (_dir, controller) = setup();
        let mut host = controller.shared.lock().unwrap();
        for (target, text) in [(0, "Shared"), (1, "Hardware"), (2, "Editor")] {
            host.plugin_message(0, json!({"event":"setTitle","context":"test.context","payload":{"title":text,"target":target}})).unwrap();
        }
        let software = host.slots[0].clone();
        assert_eq!(software.title, "Editor");
        assert_eq!(host.hardware_projection(software).title, "Hardware");
        host.plugin_message(
            0,
            json!({"event":"setTitle","context":"test.context","payload":{"target":2}}),
        )
        .unwrap();
        let software = host.slots[0].clone();
        assert_eq!(software.title, "Zero");
        assert_eq!(host.hardware_projection(software).title, "Hardware");
        host.plugin_message(
            0,
            json!({"event":"setTitle","context":"test.context","payload":{"target":1,"title":""}}),
        )
        .unwrap();
        let software = host.slots[0].clone();
        assert_eq!(host.hardware_projection(software).title, "");
        host.plugin_message(
            0,
            json!({"event":"setTitle","context":"test.context","payload":{"target":1}}),
        )
        .unwrap();
        let software = host.slots[0].clone();
        assert_eq!(host.hardware_projection(software).title, "Zero");
        assert!(host.slots[0].appearances[&1].plugin_title.is_none());
    }
    #[test]
    fn malformed_display_payloads_leave_the_action_unchanged() {
        let (_dir, controller) = setup();
        let mut host = controller.shared.lock().unwrap();
        let before = serde_json::to_value(&host.slots[0]).unwrap();
        for (event, payload) in [
            ("setTitle", json!({"target":"2","title":"bad"})),
            ("setImage", json!({"target":null,"image":""})),
            ("setTitle", json!({"title":42})),
            ("setImage", json!({"image":{}})),
            ("setImage", json!({"state":true,"image":""})),
            ("setTitle", json!({"title":"x".repeat(4097)})),
        ] {
            assert!(
                host.plugin_message(
                    0,
                    json!({"event":event,"context":"test.context","payload":payload})
                )
                .is_err()
            );
            assert_eq!(serde_json::to_value(&host.slots[0]).unwrap(), before);
        }
    }
    #[test]
    fn title_only_target_split_checks_pixel_budget_before_cloning_all_states() {
        let (_dir, controller) = setup();
        let mut host = controller.shared.lock().unwrap();
        host.slots[0].states = json!(vec![json!({}); 11]);
        let pixels = DynamicImage::ImageRgb8(RgbImage::new(1024, 1024));
        for state in 0..11 {
            host.slots[0].appearances.insert(
                state,
                StateAppearance {
                    plugin_pixels: Some(pixels.clone()),
                    ..Default::default()
                },
            );
        }
        let error = host.plugin_message(0,json!({"event":"setTitle","context":"test.context","payload":{"target":1,"title":"Device"}})).unwrap_err();
        assert!(error.to_string().contains("64 MiB"));
        assert!(
            host.slots[0]
                .appearances
                .values()
                .all(|a| a.hardware.is_none() && a.plugin_title.is_none())
        );
    }
    #[test]
    fn legacy_encoder_runtime_image_migrates_without_relabelling_user_artwork() {
        let (_dir, controller) = setup();
        let mut host = controller.shared.lock().unwrap();
        let slot = &mut host.slots[0];
        slot.controller = "Encoder".into();
        slot.image_changed = true;
        let runtime = DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb([10, 20, 30])));
        let user = DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb([40, 50, 60])));
        slot.background = Some(runtime.clone());
        slot.appearances.insert(
            0,
            StateAppearance {
                plugin_pixels: Some(runtime.clone()),
                user_pixels: Some(user.clone()),
                ..Default::default()
            },
        );
        migrate(slot, &[]);
        assert!(slot.appearances[&0].plugin_pixels.is_none());
        assert_eq!(
            slot.appearances[&0]
                .encoder_pixels
                .as_ref()
                .unwrap()
                .as_bytes(),
            runtime.as_bytes()
        );
        assert_eq!(
            slot.appearances[&0]
                .user_pixels
                .as_ref()
                .unwrap()
                .as_bytes(),
            user.as_bytes()
        );
        assert!(!slot.image_changed);
        assert!(slot.background.is_none());
    }
    #[test]
    fn inactive_title_and_style_emit_correct_state_without_changing_live_key() {
        let (_dir, controller) = setup();
        let (tx, mut rx) = mpsc::unbounded_channel();
        controller.shared.lock().unwrap().plugins[0].sender = Some(tx);
        let mut command = request(1);
        command["title"] = json!("Custom one");
        command["title_parameters"] = json!(TitleStyle::default());
        controller.command(command).unwrap();
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.slots[0].state, 0);
        assert_eq!(host.slots[0].title, "Zero");
        let event = rx.try_recv().unwrap();
        assert_eq!(event["event"], "titleParametersDidChange");
        assert_eq!(event["payload"]["state"], 1);
        assert_eq!(event["payload"]["title"], "Custom one");
        assert_eq!(event["payload"]["coordinates"], json!({"column":0,"row":0}));
        assert!(event["payload"]["settings"].is_object());
        assert!(event["payload"]["resources"].is_object());
        assert_eq!(host.slots[0].states[0]["Future"], 4);
    }
    #[test]
    fn preview_is_read_only_and_stale_live_state_is_rejected() {
        let (dir, controller) = setup();
        let mut command = request(1);
        command["image_path"] = json!(svg(dir.path(), "preview.svg", "red"));
        let before = serde_json::to_value(&controller.shared.lock().unwrap().slots).unwrap();
        let preview = controller.appearance_preview(command.clone()).unwrap();
        assert_eq!(preview["state"], 1);
        assert_eq!(preview["title"], "One");
        assert!(
            preview["preview"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(!dir.path().join("appearance-assets").exists());
        assert_eq!(
            serde_json::to_value(&controller.shared.lock().unwrap().slots).unwrap(),
            before
        );
        controller.shared.lock().unwrap().slots[0].state = 1;
        assert!(controller.command(command).is_err());
        assert!(!dir.path().join("appearance-assets").exists());
    }
    #[test]
    fn user_title_has_priority_and_reset_exposes_latest_sdk_title() {
        let (_dir, controller) = setup();
        let mut command = request(0);
        command["title"] = json!("User");
        controller.command(command).unwrap();
        {
            let mut host = controller.shared.lock().unwrap();
            host.plugin_message(
                0,
                json!({"event":"setTitle","context":"test.context","payload":{"title":"Plugin"}}),
            )
            .unwrap();
            assert_eq!(host.slots[0].title, "User");
        }
        let mut reset = request(0);
        reset["reset_title"] = json!(true);
        controller.command(reset).unwrap();
        assert_eq!(controller.snapshot()["slots"][0]["title"], "Plugin");
    }
    #[test]
    fn disabled_user_title_still_allows_style_and_protocol_style_event() {
        let (_dir, controller) = setup();
        controller.shared.lock().unwrap().actions[0].raw["UserTitleEnabled"] = json!(false);
        let mut command = request(0);
        command["title"] = json!("Forbidden");
        assert!(controller.command(command).is_err());
        let mut style = request(0);
        style["title_parameters"] = json!(TitleStyle::default());
        controller.command(style).unwrap();
        assert_eq!(controller.snapshot()["slots"][0]["title"], "Zero");
    }
    #[test]
    fn two_state_original_icons_export_and_reset_independently() {
        let (dir, controller) = setup();
        let zero = svg(dir.path(), "zero.svg", "red");
        let one = dir.path().join("one.gif");
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for rgba in [[0, 0, 255, 255], [0, 255, 0, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        8,
                        8,
                        image::Rgba(rgba),
                    )))
                    .unwrap();
            }
        }
        fs::write(&one, gif).unwrap();
        for (state, path) in [(0, &zero), (1, &one)] {
            let mut command = request(state);
            command["image_path"] = json!(path);
            controller.command(command).unwrap();
        }
        let output = dir.path().join("icons.streamDeckProfile");
        controller
            .shared
            .lock()
            .unwrap()
            .export_profile(&output)
            .unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let page = bundle.pages.iter().find(|p| !p.slots.is_empty()).unwrap();
        let slot = &page.slots[0];
        assert_eq!(
            page.asset(slot.states[0]["Image"].as_str().unwrap())
                .unwrap(),
            fs::read(&zero).unwrap()
        );
        assert_eq!(
            page.asset(slot.states[1]["Image"].as_str().unwrap())
                .unwrap(),
            fs::read(&one).unwrap()
        );
        let mut reset = request(1);
        reset["reset_image"] = json!(true);
        controller.command(reset).unwrap();
        let host = controller.shared.lock().unwrap();
        assert!(host.slots[0].appearances[&0].user_image.is_some());
        assert!(host.slots[0].appearances[&1].user_image.is_none());
        assert_eq!(host.slots[0].state, 0);
    }
    #[test]
    fn legacy_user_title_migrates_without_freezing_plugin_owned_titles() {
        let (_dir, controller) = setup();
        {
            let mut host = controller.shared.lock().unwrap();
            host.slots[0].uuid = crate::system_actions::WEBSITE.into();
            host.actions[0].uuid = crate::system_actions::WEBSITE.into();
            host.slots[0].title = "Legacy user".into();
            host.slots[0].title_changed = true;
            host.sync_visible();
            host.hydrate_library_children();
            host.slots = host.library.visible().unwrap();
        }
        assert_eq!(
            controller.snapshot()["slots"][0]["appearance"]["title_custom"],
            true
        );
        let mut reset = request(0);
        reset["reset_title"] = json!(true);
        controller.command(reset).unwrap();
        assert_eq!(controller.snapshot()["slots"][0]["title"], "Zero");
        let mut host = controller.shared.lock().unwrap();
        host.actions[0].uuid = "test.action".into();
        host.slots[0].uuid = "test.action".into();
        host.actions[0].raw["UserTitleEnabled"] = json!(true);
        host.slots[0].appearances.clear();
        host.slots[0].title_changed = true;
        host.slots[0].title = "Old plugin".into();
        host.sync_visible();
        host.hydrate_library_children();
        host.slots = host.library.visible().unwrap();
        assert!(
            !snapshot(&host.slots[0], &host.actions)["title_custom"]
                .as_bool()
                .unwrap()
        );
        host.plugin_message(
            0,
            json!({"event":"setTitle","context":"test.context","payload":{"title":"New plugin"}}),
        )
        .unwrap();
        assert_eq!(host.slots[0].title, "New plugin");
    }
    #[test]
    fn sparse_style_patch_preserves_other_fields_and_normalizes_event_family_only() {
        let (_dir, controller) = setup();
        let (tx, mut rx) = mpsc::unbounded_channel();
        {
            let mut host = controller.shared.lock().unwrap();
            host.plugins[0].sender = Some(tx);
            host.slots[0].states[0]["FontFamily"] = json!("");
            host.slots[0].states[0]["TitleColor"] = json!("#123456");
            host.slots[0].states[0]["FontStyle"] = json!("Bold");
        }
        let mut command = request(0);
        command["title_parameters"] = json!({"fontSize":23});
        controller.command(command).unwrap();
        let host = controller.shared.lock().unwrap();
        assert_eq!(host.slots[0].states[0]["FontFamily"], "");
        assert_eq!(host.slots[0].states[0]["TitleColor"], "#123456");
        assert_eq!(host.slots[0].states[0]["FontStyle"], "Bold");
        assert_eq!(host.slots[0].states[0]["Future"], 4);
        let event = rx.try_recv().unwrap();
        assert!(event["payload"].get("isInMultiAction").is_none());
        assert_eq!(
            event["payload"]["titleParameters"]["fontFamily"],
            TitleStyle::default().font_family
        );
    }
    #[test]
    fn custom_icon_blocks_sdk_image_and_reset_restores_latest_sdk_value() {
        let (dir, controller) = setup();
        let red = svg(dir.path(), "red.svg", "red");
        svg(dir.path(), "blue.svg", "blue");
        let mut command = request(0);
        command["image_path"] = json!(red);
        controller.command(command).unwrap();
        {
            let mut host = controller.shared.lock().unwrap();
            host.plugin_message(0,json!({"event":"setImage","context":"test.context","payload":{"image":"blue.svg","state":0}})).unwrap();
            assert_eq!(
                host.slots[0]
                    .background
                    .as_ref()
                    .unwrap()
                    .to_rgba8()
                    .get_pixel(3, 3)[0],
                255
            );
        }
        let mut reset = request(0);
        reset["reset_image"] = json!(true);
        controller.command(reset).unwrap();
        let host = controller.shared.lock().unwrap();
        assert_eq!(
            host.slots[0]
                .background
                .as_ref()
                .unwrap()
                .to_rgba8()
                .get_pixel(3, 3)[2],
            255
        );
    }
    #[test]
    fn sdk_cache_exports_baseline_and_reimported_user_icon_remains_custom() {
        let (dir, controller) = setup();
        svg(dir.path(), "temporary.svg", "blue");
        {
            let mut host = controller.shared.lock().unwrap();
            host.plugin_message(0,json!({"event":"setTitle","context":"test.context","payload":{"title":"Transient"}})).unwrap();
            host.plugin_message(0,json!({"event":"setImage","context":"test.context","payload":{"image":"temporary.svg"}})).unwrap();
        }
        let output = dir.path().join("baseline.streamDeckProfile");
        controller
            .shared
            .lock()
            .unwrap()
            .export_profile(&output)
            .unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let page = bundle.pages.iter().find(|p| !p.slots.is_empty()).unwrap();
        assert_eq!(page.slots[0].states[0]["Title"], "Zero");
        assert!(page.slots[0].states[0].get("Image").is_none());
        let red = svg(dir.path(), "user.svg", "red");
        let mut command = request(0);
        command["image_path"] = json!(red);
        controller.command(command).unwrap();
        let output = dir.path().join("custom.streamDeckProfile");
        {
            let mut host = controller.shared.lock().unwrap();
            host.export_profile(&output).unwrap();
            host.import_profile(&output).unwrap();
            let context = host.slots[0].context.clone();
            assert!(
                snapshot(&host.slots[0], &host.actions)["image_custom"]
                    .as_bool()
                    .unwrap()
            );
            host.plugin_message(
                0,
                json!({"event":"setImage","context":context,"payload":{"image":"temporary.svg"}}),
            )
            .unwrap();
            assert_eq!(
                host.slots[0]
                    .background
                    .as_ref()
                    .unwrap()
                    .to_rgba8()
                    .get_pixel(3, 3)[0],
                255
            );
        }
    }
    #[test]
    fn large_original_image_uses_bounded_saved_cache_and_exports_original_bytes() {
        let (dir, controller) = setup();
        let mut random = 0x914abd23u32;
        let image = image::RgbImage::from_fn(2048, 2048, |_, _| {
            let mut channels = [0; 3];
            for channel in &mut channels {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                *channel = random as u8;
            }
            image::Rgb(channels)
        });
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
            .encode_image(&image)
            .unwrap();
        let original = dir.path().join("noise.jpg");
        fs::write(&original, &bytes).unwrap();
        let mut command = request(0);
        command["image_path"] = json!(original);
        controller.command(command).unwrap();
        let mut host = controller.shared.lock().unwrap();
        let image = host.slots[0].background.as_ref().unwrap();
        assert!(image.width() <= 1024 && image.height() <= 1024);
        let saved: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert!(saved.slots[0].background.as_ref().unwrap().width() <= 1024);
        assert_eq!(
            fs::read(host.slots[0].appearances[&0].user_image.as_ref().unwrap()).unwrap(),
            bytes
        );
        let output = dir.path().join("noise.streamDeckProfile");
        host.export_profile(&output).unwrap();
        let bundle = profile::load_bundle(&output).unwrap();
        let page = bundle.pages.iter().find(|p| !p.slots.is_empty()).unwrap();
        assert_eq!(
            page.asset(page.slots[0].states[0]["Image"].as_str().unwrap())
                .unwrap(),
            bytes
        );
    }
    #[test]
    fn nested_style_survives_subsequent_container_record_edit() {
        let (_dir, controller) = setup();
        let parent;
        {
            let mut host = controller.shared.lock().unwrap();
            host.actions.extend(multiaction::definitions());
            host.assign("Keypad", 1, multiaction::ROUTINE).unwrap();
            parent = host
                .slots
                .iter()
                .find(|s| s.index == 1)
                .unwrap()
                .context
                .clone();
            host.save_multi(&parent,json!({"Actions":[{"Actions":[{"UUID":"test.action","ActionID":"child","States":[{"Future":17}],"Settings":{}}]},{"Actions":[]}]})).unwrap();
        }
        let context = controller
            .shared
            .lock()
            .unwrap()
            .visible_slot(&parent)
            .unwrap()
            .children[0][0]
            .context
            .clone();
        controller.command(json!({"command":"appearance","context":context,"expected_state":0,"state":0,"title_parameters":{"fontSize":25,"titleColor":"#009922"}})).unwrap();
        let mut host = controller.shared.lock().unwrap();
        let mut raw = host.visible_slot(&parent).unwrap().raw.clone();
        raw["Actions"][0]["Actions"][0]["Settings"] = json!({"changed":true});
        host.save_multi(&parent, raw).unwrap();
        let child = &host.visible_slot(&parent).unwrap().children[0][0];
        assert_eq!(child.states[0]["FontSize"], 25.0);
        assert_eq!(child.states[0]["TitleColor"], "#009922");
        assert_eq!(child.raw["States"][0]["Future"], 17);
    }
    #[test]
    fn style_only_and_reset_keep_extensionless_manifest_image() {
        let (dir, controller) = setup();
        let path = dir.path().join("imgs/actions/counter/key.png");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = image::RgbaImage::from_pixel(16, 16, image::Rgba([31, 87, 173, 255]));
        original.save(&path).unwrap();
        let context = {
            let mut host = controller.shared.lock().unwrap();
            host.actions[0].raw["States"] = json!([{"Image":"imgs/actions/counter/key"}]);
            host.assign("Keypad", 0, "test.action").unwrap();
            assert_eq!(
                host.slots[0].background.as_ref().unwrap().to_rgba8(),
                original
            );
            host.slots[0].context.clone()
        };
        controller.command(json!({"command":"appearance","context":context,"expected_state":0,"state":0,"title_parameters":{"fontSize":21}})).unwrap();
        assert_eq!(
            controller.shared.lock().unwrap().slots[0]
                .background
                .as_ref()
                .unwrap()
                .to_rgba8(),
            original
        );
        let replacement = svg(dir.path(), "custom.svg", "red");
        controller.command(json!({"command":"appearance","context":context,"expected_state":0,"state":0,"image_path":replacement})).unwrap();
        assert_ne!(
            controller.shared.lock().unwrap().slots[0]
                .background
                .as_ref()
                .unwrap()
                .to_rgba8(),
            original
        );
        controller.command(json!({"command":"appearance","context":context,"expected_state":0,"state":0,"reset_image":true})).unwrap();
        assert_eq!(
            controller.shared.lock().unwrap().slots[0]
                .background
                .as_ref()
                .unwrap()
                .to_rgba8(),
            original
        );
    }
}
