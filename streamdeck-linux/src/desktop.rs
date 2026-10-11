//! Native desktop editor. The host remains the owner of devices and plugins.
use crate::host::Controller;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(200);
const ACCENT: Color32 = Color32::from_rgb(108, 218, 187);

pub fn run(controller: Controller) -> Result<()> {
    let capture_path = std::env::var_os("DECKFORGE_CAPTURE").map(PathBuf::from);
    let capture_result = Arc::new(Mutex::new(None));
    let outcome = capture_result.clone();
    let capture_requested = capture_path.is_some();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Deckforge · Stream Deck+")
            .with_app_id("deckforge")
            .with_inner_size([1180.0, 850.0])
            .with_min_inner_size([760.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Deckforge",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx.style_mut_of(egui::Theme::Dark, |s| {
                s.spacing.item_spacing = Vec2::new(10.0, 10.0);
                s.spacing.button_padding = Vec2::new(12.0, 8.0);
                s.visuals.panel_fill = Color32::from_rgb(19, 23, 29);
                s.visuals.selection.bg_fill = Color32::from_rgb(35, 79, 68);
                s.visuals.selection.stroke = egui::Stroke::new(2.0, ACCENT);
                s.text_styles
                    .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
                s.text_styles
                    .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
                s.text_styles
                    .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
                s.text_styles
                    .insert(egui::TextStyle::Heading, egui::FontId::proportional(22.0));
            });
            let mut app = Desktop::new(controller);
            app.capture = capture_path.map(|path| Capture {
                path,
                started: Instant::now(),
                requested: false,
                result: capture_result,
            });
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Native window failed: {e}"))?;
    if capture_requested {
        let result = outcome
            .lock()
            .unwrap()
            .take()
            .context("Window closed before screenshot completed")?;
        result.map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

struct Capture {
    path: PathBuf,
    started: Instant,
    requested: bool,
    result: Arc<Mutex<Option<std::result::Result<(), String>>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Selection {
    dial: bool,
    index: u8,
}
impl Selection {
    fn controller(self) -> &'static str {
        if self.dial { "Encoder" } else { "Keypad" }
    }
    fn label(self) -> String {
        format!(
            "{} {}",
            if self.dial { "Dial" } else { "Key" },
            self.index + 1
        )
    }
}

#[derive(Clone)]
struct Draft {
    context: String,
    action: String,
    title: String,
    settings: Value,
    numbers: HashMap<String, String>,
    raw: String,
    raw_dirty: bool,
    dirty: bool,
}
impl Draft {
    fn from_slot(slot: Option<&Value>) -> Self {
        let settings = slot
            .map(|v| v["settings"].clone())
            .filter(Value::is_object)
            .unwrap_or(json!({}));
        Self {
            context: slot
                .map(|v| string(v, "context").to_owned())
                .unwrap_or_default(),
            action: slot
                .map(|v| string(v, "uuid").to_owned())
                .unwrap_or_default(),
            title: slot
                .map(|v| string(v, "title").to_owned())
                .unwrap_or_default(),
            numbers: numeric_strings(&settings),
            raw: serde_json::to_string_pretty(&settings).unwrap_or_default(),
            settings,
            raw_dirty: false,
            dirty: false,
        }
    }
    fn reconcile(&mut self, slot: Option<&Value>) {
        if !self.dirty {
            *self = Self::from_slot(slot);
        }
    }
    fn settings_to_save(&self) -> Result<Value> {
        if self.raw_dirty {
            let settings: Value =
                serde_json::from_str(&self.raw).context("Settings JSON is invalid")?;
            ensure!(settings.is_object(), "Settings must be a JSON object");
            return Ok(settings);
        }
        let mut value = self.settings.clone();
        for (key, text) in &self.numbers {
            let number: Value = serde_json::from_str(text)
                .with_context(|| format!("{key} needs a valid number"))?;
            ensure!(number.is_number(), "{key} needs a valid number");
            value[key] = number;
        }
        Ok(value)
    }
}
fn numeric_strings(settings: &Value) -> HashMap<String, String> {
    settings
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| v.is_number())
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect()
}
fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

struct Preview {
    encoded: String,
    texture: Option<TextureHandle>,
}
struct Desktop {
    controller: Controller,
    snapshot: Value,
    last_poll: Option<Instant>,
    selected: Selection,
    drafts: HashMap<Selection, Draft>,
    previews: HashMap<String, Preview>,
    error: String,
    notice: String,
    plugin_path: String,
    profile_path: String,
    new_setting: String,
    new_kind: u8,
    confirm_import: bool,
    capture: Option<Capture>,
}
impl Desktop {
    fn new(controller: Controller) -> Self {
        Self {
            controller,
            snapshot: json!({}),
            last_poll: None,
            selected: Selection {
                dial: false,
                index: 0,
            },
            drafts: HashMap::new(),
            previews: HashMap::new(),
            error: String::new(),
            notice: String::new(),
            plugin_path: String::new(),
            profile_path: String::new(),
            new_setting: String::new(),
            new_kind: 0,
            confirm_import: false,
            capture: None,
        }
    }
    fn capture_if_requested(&mut self, ctx: &egui::Context) {
        let Some(capture) = &mut self.capture else {
            return;
        };
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot {
                    viewport_id, image, ..
                } if *viewport_id == egui::ViewportId::ROOT => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot {
            let rgba: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            let result = image::save_buffer(
                &capture.path,
                &rgba,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .map_err(|e| e.to_string());
            *capture.result.lock().unwrap() = Some(result);
            self.capture = None;
            // Normal event-loop exit lets the parent stop the host and its plugins.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if capture.started.elapsed() > Duration::from_secs(10) {
            *capture.result.lock().unwrap() = Some(Err("Native screenshot timed out".into()));
            self.capture = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !capture.requested && capture.started.elapsed() >= Duration::from_millis(500) {
            capture.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }
    fn slot(&self, selection: Selection) -> Option<&Value> {
        self.snapshot["slots"].as_array()?.iter().find(|s| {
            string(s, "controller") == selection.controller()
                && s["index"].as_u64() == Some(u64::from(selection.index))
        })
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.last_poll.is_some_and(|last| last.elapsed() < POLL) {
            return;
        }
        self.snapshot = self.controller.snapshot();
        self.last_poll = Some(Instant::now());
        for dial in [false, true] {
            for index in 0..if dial { 4 } else { 8 } {
                let selection = Selection { dial, index };
                let slot = self.slot(selection).cloned();
                self.drafts
                    .entry(selection)
                    .and_modify(|d| d.reconcile(slot.as_ref()))
                    .or_insert_with(|| Draft::from_slot(slot.as_ref()));
            }
        }
        let slots = self.snapshot["slots"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        self.previews
            .retain(|context, _| slots.iter().any(|s| string(s, "context") == context));
        for slot in slots {
            let context = string(&slot, "context");
            let encoded = string(&slot, "preview");
            if self
                .previews
                .get(context)
                .is_some_and(|p| p.encoded == encoded)
            {
                continue;
            }
            let texture = STANDARD
                .decode(encoded)
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|image| {
                    let rgba = image.to_rgba8();
                    ctx.load_texture(
                        context,
                        egui::ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            rgba.as_raw(),
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                });
            self.previews.insert(
                context.to_owned(),
                Preview {
                    encoded: encoded.to_owned(),
                    texture,
                },
            );
        }
    }
    fn command(&mut self, command: Value) -> bool {
        match self.controller.command(command) {
            Ok(()) => {
                self.error.clear();
                true
            }
            Err(error) => {
                self.error = format!("{error:#}");
                self.notice.clear();
                false
            }
        }
    }
    fn test_press(&mut self, selection: Selection) {
        let prefix = if selection.dial { "dial" } else { "key" };
        if self.command(
            json!({"command":"input","event":format!("{prefix}Down"),"index":selection.index}),
        ) {
            // Always pair a successful down event with its release.
            self.command(
                json!({"command":"input","event":format!("{prefix}Up"),"index":selection.index}),
            );
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Deckforge");
            ui.label(RichText::new("STREAM DECK +").small().color(Color32::GRAY));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let connected = self.snapshot["connected"].as_bool().unwrap_or(false);
                let simulated = self.snapshot["simulate"].as_bool().unwrap_or(false);
                ui.colored_label(
                    if connected {
                        ACCENT
                    } else {
                        Color32::LIGHT_GRAY
                    },
                    if simulated {
                        "Simulation"
                    } else if connected {
                        "USB connected"
                    } else {
                        "USB disconnected"
                    },
                );
            });
        });
        ui.horizontal(|ui| {
            ui.small(format!(
                "Device {}   ·   Firmware {}",
                string(&self.snapshot, "device"),
                string(&self.snapshot, "firmware")
            ));
        });
        ui.separator();
        if !self.error.is_empty() {
            ui.colored_label(Color32::from_rgb(255, 172, 163), &self.error);
        }
        if !self.notice.is_empty() {
            ui.colored_label(ACCENT, &self.notice);
        }
    }
    fn device(&mut self, ui: &mut egui::Ui) {
        ui.heading("Your controls");
        ui.label("Select a key or dial to edit its action.");
        let width = (ui.available_width() - 30.0).max(280.0);
        let key_size = ((width - 3.0 * 12.0) / 4.0).clamp(62.0, 115.0);
        egui::Frame::new().fill(Color32::from_rgb(8, 11, 15)).corner_radius(16).inner_margin(14).show(ui, |ui| {
            egui::Grid::new("device_keys").spacing([12.0, 12.0]).show(ui, |ui| {
                for index in 0..8 {
                    self.control_button(ui, Selection { dial: false, index }, Vec2::splat(key_size));
                    if index % 4 == 3 { ui.end_row(); }
                }
            });
            ui.add_space(12.0);
            egui::Grid::new("device_dials").spacing([12.0, 10.0]).show(ui, |ui| {
                for index in 0..4 {
                    self.control_button(ui, Selection { dial: true, index }, Vec2::new(key_size, key_size / 2.0));
                }
                ui.end_row();
                for index in 0..4 {
                    ui.vertical_centered(|ui| {
                        ui.set_width(key_size);
                        ui.label(RichText::new(format!("Dial {}", index + 1)).small());
                        ui.horizontal(|ui| {
                            for (label, ticks) in [("−", -1), ("+", 1)] {
                                let description = format!("Rotate dial {} {}", index + 1,
                                    if ticks < 0 { "counterclockwise" } else { "clockwise" });
                                let response = ui.button(label);
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &description));
                                if response.on_hover_text(description).clicked() {
                                    self.command(json!({"command":"input","event":"dialRotate","index":index,"ticks":ticks}));
                                }
                            }
                        });
                    });
                }
            });
        });
        ui.add_space(8.0);
        ui.small("The previews show the images sent to the device.");
    }
    fn control_button(&mut self, ui: &mut egui::Ui, selection: Selection, size: Vec2) {
        let slot = self.slot(selection);
        let description = slot
            .map(|s| format!("{} · {}", selection.label(), string(s, "name")))
            .unwrap_or_else(|| format!("{} · Empty", selection.label()));
        let texture = slot
            .and_then(|s| self.previews.get(string(s, "context")))
            .and_then(|p| p.texture.as_ref());
        let button = if let Some(texture) = texture {
            egui::Button::image(egui::Image::new((texture.id(), size - Vec2::splat(8.0))))
        } else {
            egui::Button::new(format!("{}\n+ Assign", selection.label()))
        };
        let response = ui.add_sized(
            size,
            button.corner_radius(8).selected(self.selected == selection),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                self.selected == selection,
                &description,
            )
        });
        if response.on_hover_text(description).clicked() {
            self.selected = selection;
        }
    }
    fn editor(&mut self, ui: &mut egui::Ui) {
        let selection = self.selected;
        let slot = self.slot(selection).cloned();
        let mut draft = self
            .drafts
            .remove(&selection)
            .unwrap_or_else(|| Draft::from_slot(slot.as_ref()));
        ui.heading(selection.label());
        ui.label(
            slot.as_ref()
                .map(|s| string(s, "name"))
                .filter(|s| !s.is_empty())
                .unwrap_or("No action assigned"),
        );
        let stale = draft.dirty
            && draft.context != slot.as_ref().map(|s| string(s, "context")).unwrap_or("");
        if stale {
            ui.colored_label(
                Color32::YELLOW,
                "This control changed. Reload before saving these edits.",
            );
        }
        if draft.dirty {
            ui.horizontal(|ui| {
                ui.colored_label(ACCENT, "Unsaved edits");
                if ui.button("Discard edits").clicked() {
                    draft = Draft::from_slot(slot.as_ref());
                }
            });
        }
        ui.separator();
        ui.label("Plugin action");
        let actions = self.snapshot["actions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let selected_name = actions
            .iter()
            .find(|a| string(a, "uuid") == draft.action)
            .map(|a| string(a, "name"))
            .unwrap_or("Choose an action");
        egui::ComboBox::from_id_salt("action_picker")
            .selected_text(selected_name)
            .width((ui.available_width() - 16.0).max(180.0))
            .show_ui(ui, |ui| {
                for action in actions.iter().filter(|a| {
                    a["controllers"]
                        .as_array()
                        .is_some_and(|cs| cs.iter().any(|c| c == selection.controller()))
                }) {
                    if ui
                        .selectable_value(
                            &mut draft.action,
                            string(action, "uuid").to_owned(),
                            string(action, "name"),
                        )
                        .changed()
                    {
                        draft.dirty = true;
                    }
                }
            });
        ui.horizontal(|ui| {
            if ui.add_enabled(!draft.action.is_empty(), egui::Button::new("Assign action")).clicked()
                && self.command(json!({"command":"assign","controller":selection.controller(),"index":selection.index,"uuid":draft.action})) {
                draft.dirty = false;
                self.notice = format!("Action assigned to {}", selection.label().to_lowercase());
            }
            if ui.add_enabled(slot.is_some(), egui::Button::new("Test press")).clicked() { self.test_press(selection); }
        });
        ui.add_space(8.0);
        ui.add_enabled_ui(slot.is_some() && !stale, |ui| {
            let title_label = ui.label("Title on device");
            if ui.text_edit_singleline(&mut draft.title).labelled_by(title_label.id).changed() { draft.dirty = true; }
            ui.add_space(8.0);
            ui.label(RichText::new("Settings").strong());
            self.settings_editor(ui, &mut draft);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Save changes").fill(Color32::from_rgb(35, 79, 68))).clicked() {
                    match draft.settings_to_save() {
                        Ok(settings) => {
                            if self.command(json!({"command":"settings","context":draft.context,"settings":settings}))
                                && self.command(json!({"command":"appearance","context":draft.context,"title":draft.title})) {
                                draft.dirty = false;
                                self.notice = format!("{} saved", selection.label());
                            }
                        }
                        Err(error) => self.error = error.to_string(),
                    }
                }
                if ui.button("Remove action").clicked()
                    && self.command(json!({"command":"remove","context":draft.context})) {
                    draft = Draft::from_slot(None);
                    self.notice = "Action removed".into();
                }
            });
        });
        self.drafts.insert(selection, draft);
    }
    fn settings_editor(&mut self, ui: &mut egui::Ui, draft: &mut Draft) {
        ui.add_enabled_ui(!draft.raw_dirty, |ui| {
            if let Some(settings) = draft.settings.as_object_mut() {
                if settings.is_empty() {
                    ui.small("This action has no saved settings yet.");
                }
                for (key, value) in settings.iter_mut() {
                    ui.push_id(key, |ui| match value {
                        Value::Bool(enabled) => {
                            if ui.checkbox(enabled, key).changed() {
                                draft.dirty = true;
                            }
                        }
                        Value::String(text) => {
                            let label = ui.label(key);
                            if ui
                                .text_edit_singleline(text)
                                .labelled_by(label.id)
                                .changed()
                            {
                                draft.dirty = true;
                            }
                        }
                        Value::Number(_) => {
                            let label = ui.label(key);
                            if ui
                                .text_edit_singleline(draft.numbers.entry(key.clone()).or_default())
                                .labelled_by(label.id)
                                .changed()
                            {
                                draft.dirty = true;
                            }
                        }
                        _ => {
                            ui.small(format!(
                                "{key}: structured value, available under Advanced JSON"
                            ));
                        }
                    });
                }
            }
            egui::CollapsingHeader::new("Add setting").show(ui, |ui| {
                ui.text_edit_singleline(&mut self.new_setting)
                    .on_hover_text("Setting name expected by the plugin");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("setting_kind")
                        .selected_text(["Text", "Number", "On / off"][self.new_kind as usize])
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.new_kind, 0, "Text");
                            ui.selectable_value(&mut self.new_kind, 1, "Number");
                            ui.selectable_value(&mut self.new_kind, 2, "On / off");
                        });
                    if ui
                        .add_enabled(
                            !self.new_setting.trim().is_empty(),
                            egui::Button::new("Add"),
                        )
                        .clicked()
                    {
                        let key = self.new_setting.trim().to_owned();
                        if draft.settings.get(&key).is_some() {
                            self.error = "A setting with that name already exists".into();
                        } else {
                            draft.settings[&key] = match self.new_kind {
                                1 => json!(0),
                                2 => json!(false),
                                _ => json!(""),
                            };
                            if self.new_kind == 1 {
                                draft.numbers.insert(key, "0".into());
                            }
                            draft.dirty = true;
                            self.new_setting.clear();
                        }
                    }
                });
            });
        });
        egui::CollapsingHeader::new("Advanced JSON").show(ui, |ui| {
            if !draft.raw_dirty
                && let Ok(settings) = draft.settings_to_save()
            {
                draft.raw = serde_json::to_string_pretty(&settings).unwrap_or_default();
            }
            let response = ui.add(
                egui::TextEdit::multiline(&mut draft.raw)
                    .code_editor()
                    .desired_rows(7)
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                draft.raw_dirty = true;
                draft.dirty = true;
            }
            if draft.raw_dirty && ui.button("Apply JSON to fields").clicked() {
                match draft.settings_to_save() {
                    Ok(settings) => {
                        draft.numbers = numeric_strings(&settings);
                        draft.settings = settings;
                        draft.raw_dirty = false;
                    }
                    Err(error) => self.error = error.to_string(),
                }
            }
        });
    }
    fn library(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.heading("Plugins & profiles");
        if let Some(plugins) = self.snapshot["plugins"].as_array() {
            if plugins.is_empty() {
                ui.label("No plugins installed. Choose a plugin package below.");
            }
            for plugin in plugins {
                ui.horizontal_wrapped(|ui| {
                    ui.label(string(plugin, "name"));
                    ui.colored_label(
                        if plugin["connected"] == true {
                            ACCENT
                        } else {
                            Color32::GRAY
                        },
                        if plugin["connected"] == true {
                            "Connected"
                        } else {
                            "Disconnected"
                        },
                    );
                });
            }
        }
        ui.separator();
        ui.label("Plugin package or folder");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.plugin_path)
                    .hint_text("Path to .streamDeckPlugin or plugin folder")
                    .desired_width((ui.available_width() - 130.0).max(120.0)),
            );
            if ui.button("Browse…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose a Stream Deck plugin package")
                    .pick_file()
            {
                self.plugin_path = path.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.plugin_path.trim().is_empty(),
                    egui::Button::new("Install plugin"),
                )
                .clicked()
                && self.command(json!({"command":"install_plugin","path":self.plugin_path.trim()}))
            {
                self.notice = "Plugin installed".into();
                self.plugin_path.clear();
            }
            if ui.button("Choose folder…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose a .sdPlugin folder")
                    .pick_folder()
            {
                self.plugin_path = path.display().to_string();
            }
        });
        ui.add_space(6.0);
        ui.label("Elgato profile");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.profile_path)
                    .hint_text("Path to .streamDeckProfile")
                    .desired_width((ui.available_width() - 130.0).max(120.0)),
            );
            if ui.button("Browse…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose an Elgato profile")
                    .pick_file()
            {
                self.profile_path = path.display().to_string();
            }
        });
        if ui
            .add_enabled(
                !self.profile_path.trim().is_empty(),
                egui::Button::new("Import profile…"),
            )
            .clicked()
        {
            self.confirm_import = true;
        }
        if self.confirm_import {
            ui.label("Import replaces the current assignments. Unfinished edits stay in this window until you discard them.");
            ui.horizontal(|ui| {
                if ui.button("Replace assignments").clicked() {
                    self.confirm_import = false;
                    if self.command(
                        json!({"command":"import_profile","path":self.profile_path.trim()}),
                    ) {
                        self.notice = "Profile imported".into();
                    }
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_import = false;
                }
            });
        }
    }
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.capture_if_requested(ui.ctx());
        self.refresh(ui.ctx());
        ui.ctx().request_repaint_after(POLL);
        egui::Frame::central_panel(ui.style())
            .inner_margin(20)
            .show(ui, |ui| {
                self.toolbar(ui);
                let available = ui.available_width();
                if available > 920.0 {
                    let left_width = (available * 0.52).clamp(440.0, 580.0);
                    let right_width = available - left_width - 32.0;
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(left_width, ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical().id_salt("device_scroll").show(
                                    ui,
                                    |ui| {
                                        self.device(ui);
                                        self.library(ui);
                                    },
                                );
                            },
                        );
                        ui.separator();
                        ui.allocate_ui_with_layout(
                            Vec2::new(right_width, ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(right_width);
                                egui::ScrollArea::vertical()
                                    .id_salt("editor_scroll")
                                    .show(ui, |ui| self.editor(ui));
                            },
                        );
                    });
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.device(ui);
                        ui.separator();
                        self.editor(ui);
                        self.library(ui);
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polling_preserves_unsaved_settings_and_title() {
        let old = json!({"context":"one","uuid":"counter","title":"Before","settings":{"count":1}});
        let changed =
            json!({"context":"two","uuid":"counter","title":"Remote","settings":{"count":9}});
        let mut draft = Draft::from_slot(Some(&old));
        draft.title = "Local".into();
        draft.numbers.insert("count".into(), "2".into());
        draft.dirty = true;
        draft.reconcile(Some(&changed));
        assert_eq!(draft.title, "Local");
        assert_eq!(draft.context, "one");
        assert_eq!(draft.settings_to_save().unwrap()["count"], 2);
        draft.dirty = false;
        draft.reconcile(Some(&changed));
        assert_eq!(draft.context, "two");
        assert_eq!(draft.title, "Remote");
    }
    #[test]
    fn typed_numbers_reject_incomplete_or_non_numeric_edits() {
        let mut draft = Draft::from_slot(Some(&json!({"settings":{"ticks":1}})));
        for invalid in ["-", "NaN", "true", "\"1\""] {
            draft.numbers.insert("ticks".into(), invalid.into());
            assert!(draft.settings_to_save().is_err());
        }
        draft.numbers.insert("ticks".into(), "-3.5".into());
        assert_eq!(draft.settings_to_save().unwrap()["ticks"], -3.5);
    }
    #[test]
    fn advanced_json_keeps_nested_values_and_requires_object() {
        let mut draft = Draft::from_slot(None);
        draft.raw_dirty = true;
        draft.raw = "{\"nested\":{\"enabled\":true}}".into();
        assert_eq!(draft.settings_to_save().unwrap()["nested"]["enabled"], true);
        draft.raw = "[]".into();
        assert!(draft.settings_to_save().is_err());
    }
}
