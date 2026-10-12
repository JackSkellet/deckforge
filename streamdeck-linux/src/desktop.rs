//! Native desktop editor. The host remains the owner of devices and plugins.
use crate::host::Controller;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(200);
const ACCENT: Color32 = Color32::from_rgb(108, 218, 187);

pub fn run(controller: Controller, state: &std::path::Path, background: bool) -> Result<()> {
    let window = crate::window::Window::new(!background);
    let endpoint_window = window.clone();
    // The host already owns session.lock. Keep IPC alive until the GUI exits,
    // so relaunch can reopen the editor even on desktops without a tray.
    let _instance = crate::instance::spawn(
        state,
        Arc::new(move |command| Ok(endpoint_window.request(command))),
    )?;
    let tray_window = window.clone();
    let _tray = match crate::tray::spawn(Arc::new(move |event| tray_window.tray(event))) {
        Ok(tray) => Some(tray),
        Err(error) => {
            window.tray(crate::tray::Event::Error(error.to_string()));
            None
        }
    };
    let capture_path = std::env::var_os("DECKFORGE_CAPTURE").map(PathBuf::from);
    let capture_result = Arc::new(Mutex::new(None));
    let outcome = capture_result.clone();
    let capture_requested = capture_path.is_some();
    ensure!(
        !background || !capture_requested,
        "A screenshot requires a visible editor"
    );
    let saved: Arc<Mutex<Option<Desktop>>> = Arc::new(Mutex::new(None));
    eprintln!("Native background host ready");
    loop {
        if !controller.is_running() || window.quitting() {
            break;
        }
        if !window.wait() {
            continue;
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Deckforge · Stream Deck+")
                .with_app_id("deckforge")
                .with_inner_size([1180.0, 850.0])
                .with_min_inner_size([760.0, 620.0]),
            ..Default::default()
        };
        let saved_app = saved.clone();
        let current_window = window.clone();
        let controller = controller.clone();
        let preferences =
            crate::preferences::Preferences::new(state, controller.snapshot()["simulate"] == true);
        let capture_path = capture_path.clone();
        let capture_result = capture_result.clone();
        // eframe reuses its event loop across run_native calls. Closing a real
        // window works on Wayland too; winit's set_visible(false) does not.
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
                let mut app = match saved_app.lock().unwrap().take() {
                    Some(mut app) => {
                        app.attach(cc.egui_ctx.clone());
                        app
                    }
                    None => {
                        let mut app = Desktop::new(controller, cc.egui_ctx.clone(), current_window);
                        app.preferences = Some(preferences);
                        app
                    }
                };
                if capture_path.is_some()
                    && std::env::var("DECKFORGE_CAPTURE_CONTROL").as_deref() == Ok("dial0")
                {
                    app.selected = Selection {
                        dial: true,
                        index: 0,
                    };
                }
                if capture_path.is_some()
                    && std::env::var("DECKFORGE_CAPTURE_PAGE").as_deref() == Ok("preferences")
                    && let Some(preferences) = &mut app.preferences
                {
                    preferences.show();
                }
                app.capture = capture_path.map(|path| Capture {
                    path,
                    started: Instant::now(),
                    requested: false,
                    result: capture_result,
                });
                eprintln!("Native editor ready");
                Ok(Box::new(EditorWindow {
                    app: Some(app),
                    saved: saved_app,
                }))
            }),
        )
        .map_err(|error| anyhow::anyhow!("Native window failed: {error}"))?;
        window.detach();
    }
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
/// Descriptions are an effective host snapshot, not inferred manifest defaults.
fn dial_gesture_descriptions(snapshot: &Value) -> Vec<(&'static str, &str)> {
    if snapshot["controller"] != "Encoder" {
        return Vec::new();
    }
    [
        ("rotate", "Turn"),
        ("push", "Press"),
        ("touch", "Tap"),
        ("longTouch", "Hold"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        snapshot["trigger_description"]
            .get(key)?
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| (label, text))
    })
    .collect()
}

fn show_dial_gesture_descriptions(ui: &mut egui::Ui, snapshot: &Value) {
    let descriptions = dial_gesture_descriptions(snapshot);
    if descriptions.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (label, description) in descriptions {
            ui.add(
                egui::Label::new(RichText::new(format!("{label}: {description}")).small()).wrap(),
            );
        }
    });
}

fn connection_label(snapshot: &Value) -> &'static str {
    if snapshot["simulate"] == true {
        "Simulation"
    } else if snapshot["connected"] == true {
        "USB connected"
    } else {
        "Waiting for USB"
    }
}

fn device_summary(snapshot: &Value) -> String {
    let device = string(snapshot, "device").trim();
    let mut summary = if device.is_empty() || device.eq_ignore_ascii_case("unknown") {
        "Stream Deck +".to_owned()
    } else {
        format!("Device {device}")
    };
    let firmware = string(snapshot, "firmware").trim();
    if snapshot["simulate"] != true
        && !firmware.is_empty()
        && !firmware.eq_ignore_ascii_case("unknown")
        && !firmware.eq_ignore_ascii_case("simulation")
    {
        summary.push_str(&format!(
            "   ·   Firmware {firmware}{}",
            if snapshot["connected"] == true {
                ""
            } else {
                " (last known)"
            }
        ));
    }
    summary
}

fn waiting_for_usb(snapshot: &Value) -> bool {
    snapshot["simulate"] != true && snapshot["connected"] != true
}

fn page_label(page: &Value) -> String {
    let name = string(page, "name");
    let name = if name.trim().is_empty() {
        "Untitled page"
    } else {
        name
    };
    if page["folder"] == true {
        format!("Folder · {name}")
    } else {
        name.to_owned()
    }
}
fn export_file_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim().trim_matches('.');
    format!(
        "{}.streamDeckProfile",
        if name.is_empty() { "Profile" } else { name }
    )
}

struct Preview {
    encoded: String,
    texture: Option<TextureHandle>,
}
#[derive(Default)]
struct NameDraft {
    id: String,
    name: String,
    original: String,
}
impl NameDraft {
    fn reconcile(&mut self, id: &str, name: &str) {
        if self.id != id || self.name == self.original {
            self.id = id.to_owned();
            self.name = name.to_owned();
            self.original = name.to_owned();
        }
    }
}
#[derive(Clone)]
struct PendingDelete {
    profile: bool,
    id: String,
    name: String,
}
#[derive(Clone)]
struct NestedContainer {
    root_context: String,
    child_context: String,
}

/// Visit runtime child snapshots only. Raw profile/settings objects can contain
/// arbitrary lookalike fields and must not become navigation targets.
fn nested_snapshot<'a>(snapshot: &'a Value, context: &str) -> Option<&'a Value> {
    if let Some(children) = snapshot.as_array() {
        return children
            .iter()
            .find_map(|child| nested_snapshot(child, context));
    }
    if !snapshot.is_object() {
        return None;
    }
    if !context.is_empty() && string(snapshot, "context") == context {
        return Some(snapshot);
    }
    ["trigger_children", "stack_children", "multi_children"]
        .into_iter()
        .filter_map(|key| snapshot.get(key))
        .find_map(|children| nested_snapshot(children, context))
}

fn nested_container_target(parent: &Value, context: &str) -> Option<NestedContainer> {
    let child = nested_snapshot(parent, context)?;
    if !matches!(
        string(child, "uuid"),
        crate::multi::ROUTINE
            | crate::multi::SWITCH
            | crate::multi::RANDOM
            | "com.elgato.streamdeck.keys.adaptor"
    ) {
        return None;
    }
    let root_context = string(parent, "context");
    if root_context.is_empty() || context == root_context {
        return None;
    }
    Some(NestedContainer {
        root_context: root_context.to_owned(),
        child_context: context.to_owned(),
    })
}

struct Desktop {
    preferences: Option<crate::preferences::Preferences>,
    closing: bool,
    window: crate::window::Window,
    redraw_pending: Arc<AtomicBool>,
    stack_editor: crate::stack_editor::Editor,
    multi_editor: crate::multi_editor::Editor,
    trigger_editor: crate::trigger_editor::Editor,
    container_navigation: Vec<NestedContainer>,
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
    profile_name: NameDraft,
    page_name: NameDraft,
    pending_delete: Option<PendingDelete>,
    capture: Option<Capture>,
}
impl Desktop {
    fn new(controller: Controller, context: egui::Context, window: crate::window::Window) -> Self {
        window.attach(context);
        let redraw_pending = Arc::new(AtomicBool::new(true));
        let pending = redraw_pending.clone();
        let redraw_window = window.clone();
        controller.on_redraw(move || {
            pending.store(true, Ordering::Release);
            redraw_window.repaint();
        });
        Self {
            preferences: None,
            closing: false,
            window,
            redraw_pending,
            multi_editor: crate::multi_editor::Editor::default(),
            trigger_editor: crate::trigger_editor::Editor::default(),
            container_navigation: Vec::new(),
            stack_editor: crate::stack_editor::Editor::default(),
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
            profile_name: NameDraft::default(),
            page_name: NameDraft::default(),
            pending_delete: None,
            capture: None,
        }
    }
    fn attach(&mut self, context: egui::Context) {
        self.previews.clear();
        self.last_poll = None;
        self.closing = false;
        self.redraw_pending.store(true, Ordering::Release);
        self.window.attach(context);
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
            self.window.request(crate::instance::Command::Quit);
        } else if capture.started.elapsed() > Duration::from_secs(10) {
            *capture.result.lock().unwrap() = Some(Err("Native screenshot timed out".into()));
            self.capture = None;
            self.window.request(crate::instance::Command::Quit);
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
        if !self.redraw_pending.swap(false, Ordering::AcqRel)
            && self.last_poll.is_some_and(|last| last.elapsed() < POLL)
        {
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
                // Container editors acknowledge saves immediately; use the host's
                // committed snapshot before their clean drafts can reconcile.
                self.snapshot = self.controller.snapshot();
                self.last_poll = None;
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
            if ui
                .button("Hide")
                .on_hover_text("Keep the deck running. Reopen from the launcher or tray.")
                .clicked()
            {
                self.window.request(crate::instance::Command::Hide);
            }
            if ui
                .button("Quit")
                .on_hover_text("Stop Deckforge and its plugins.")
                .clicked()
            {
                self.window.request(crate::instance::Command::Quit);
            }
            if ui.button("Preferences").clicked()
                && let Some(preferences) = &mut self.preferences
            {
                preferences.show();
            }
            ui.label(RichText::new("STREAM DECK +").small().color(Color32::GRAY));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(
                    if self.snapshot["connected"] == true {
                        ACCENT
                    } else {
                        Color32::LIGHT_GRAY
                    },
                    connection_label(&self.snapshot),
                );
            });
        });
        ui.small(device_summary(&self.snapshot));
        if waiting_for_usb(&self.snapshot) {
            ui.weak("Deckforge checks for your Stream Deck every second. You can keep editing.");
            if let Some(error) = self.snapshot["connection_error"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
            {
                let summary = error
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or(error)
                    .trim();
                ui.add(egui::Label::new(RichText::new(summary).small()).truncate())
                    .on_hover_text(error);
            }
        }
        let system_status = string(&self.snapshot, "system_status");
        if !system_status.is_empty() {
            ui.label(system_status);
        }
        if self.snapshot["multi_runs"]
            .as_array()
            .is_some_and(|runs| !runs.is_empty())
            && ui.button("Cancel running sequences").clicked()
        {
            self.command(json!({"command":"multi_cancel"}));
        }
        let voices = self.snapshot["audio_active"]
            .as_object()
            .map(|v| v.values().filter_map(Value::as_u64).sum::<u64>())
            .unwrap_or(0);
        if voices > 0 {
            ui.horizontal(|ui| {
                ui.label(format!("Soundboard: {voices} playing"));
                if ui.button("Stop soundboard").clicked() {
                    self.command(json!({"command":"soundboard_stop"}));
                }
            });
        }
        if !self.error.is_empty() {
            ui.colored_label(Color32::from_rgb(255, 172, 163), &self.error);
        }
        if !self.notice.is_empty() {
            ui.colored_label(ACCENT, &self.notice);
        }
    }
    fn profile_navigation(&mut self, ui: &mut egui::Ui) {
        let library = self.snapshot["library"].clone();
        let profiles = library["profiles"].as_array().cloned().unwrap_or_default();
        let active_profile = string(&library, "active_profile");
        let profile = profiles.iter().find(|p| string(p, "id") == active_profile);
        let active_page = string(&library, "active_page");
        let pages = profile
            .and_then(|p| p["pages"].as_array())
            .cloned()
            .unwrap_or_default();
        let pin_page = profile.map(|p| string(p, "pin_page")).unwrap_or("");
        let page = pages.iter().find(|p| string(p, "id") == active_page);
        let order = profile.and_then(|p| p["page_order"].as_array());
        ui.horizontal_wrapped(|ui| {
            ui.label("Profile");
            egui::ComboBox::from_id_salt("profile_picker")
                .selected_text(profile.map(|p| string(p, "name")).unwrap_or("No profile"))
                .width(210.0)
                .show_ui(ui, |ui| {
                    for profile in &profiles {
                        let id = string(profile, "id");
                        if ui
                            .selectable_label(id == active_profile, string(profile, "name"))
                            .clicked()
                            && id != active_profile
                        {
                            self.command(json!({"command":"profile_select","id":id}));
                        }
                    }
                });
            ui.label("Page");
            egui::ComboBox::from_id_salt("page_picker")
                .selected_text(if active_page == pin_page {
                    "Pinned actions".into()
                } else {
                    page.map(page_label).unwrap_or_else(|| "No page".into())
                })
                .width(210.0)
                .show_ui(ui, |ui| {
                    for page in &pages {
                        let id = string(page, "id");
                        if ui
                            .selectable_label(id == active_page, page_label(page))
                            .clicked()
                            && id != active_page
                        {
                            self.command(json!({"command":"page_select","id":id}));
                        }
                    }
                });
            if ui
                .add_enabled(active_page != pin_page, egui::Button::new("Pinned actions"))
                .clicked()
            {
                self.command(json!({"command":"page_select","id":pin_page}));
            }
            for (label, direction) in [("Previous page", -1), ("Next page", 1)] {
                if ui
                    .add_enabled(order.is_some_and(|o| o.len() > 1), egui::Button::new(label))
                    .clicked()
                {
                    self.command(json!({"command":"page_step","direction":direction}));
                }
            }
            if ui
                .add_enabled(
                    library["folder_depth"].as_u64().unwrap_or(0) > 0,
                    egui::Button::new(if active_page == pin_page {
                        "Back to page"
                    } else {
                        "Back from folder"
                    }),
                )
                .clicked()
            {
                self.command(json!({"command":"folder_back"}));
            }
        });
        ui.separator();
    }
    fn profile_management(&mut self, ui: &mut egui::Ui) {
        let library = self.snapshot["library"].clone();
        let active_id = string(&library, "active_profile");
        let profile = library["profiles"]
            .as_array()
            .and_then(|ps| ps.iter().find(|p| string(p, "id") == active_id))
            .cloned()
            .unwrap_or(json!({}));
        let page_id = string(&library, "active_page");
        let page = profile["pages"]
            .as_array()
            .and_then(|ps| ps.iter().find(|p| string(p, "id") == page_id))
            .cloned()
            .unwrap_or(json!({}));
        self.profile_name
            .reconcile(active_id, string(&profile, "name"));
        self.page_name.reconcile(page_id, string(&page, "name"));
        egui::CollapsingHeader::new("Manage profiles & pages").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("New profile").clicked() && self.command(json!({"command":"profile_create","name":"New profile"})) {
                    self.notice = "Profile created".into();
                }
                if ui.add_enabled(!active_id.is_empty(), egui::Button::new("Duplicate profile")).clicked()
                    && self.command(json!({"command":"profile_duplicate","id":active_id,"name":format!("{} copy",string(&profile,"name"))})) {
                    self.notice = "Profile duplicated".into();
                }
                if ui.add_enabled(!active_id.is_empty(), egui::Button::new("Export profile…")).clicked()
                    && let Some(mut path) = rfd::FileDialog::new().set_title("Export Elgato profile")
                        .add_filter("Stream Deck profile", &["streamDeckProfile"])
                        .set_file_name(export_file_name(string(&profile,"name"))).save_file() {
                    if path.extension().is_none() { path.set_extension("streamDeckProfile"); }
                    if self.command(json!({"command":"export_profile","path":path})) {
                        self.notice = format!("Profile exported to {}", path.display());
                    }
                }
            });
            ui.add_enabled_ui(!active_id.is_empty(), |ui| {
                let label = ui.label("Profile name");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.profile_name.name).desired_width((ui.available_width()-115.0).max(140.0))).labelled_by(label.id);
                    if ui.add_enabled(!self.profile_name.name.trim().is_empty(),egui::Button::new("Rename")).clicked()
                        && self.command(json!({"command":"profile_rename","id":active_id,"name":self.profile_name.name.trim()})) {
                        self.profile_name.original = self.profile_name.name.clone(); self.notice = "Profile renamed".into();
                    }
                });
                if ui.button("Delete profile…").clicked() {
                    self.pending_delete = Some(PendingDelete { profile: true, id: active_id.to_owned(), name: string(&profile,"name").to_owned() });
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("New page").clicked() && self.command(json!({"command":"page_create","name":"New page"})) {
                        self.notice = "Page created".into();
                    }
                    if ui.add_enabled(!page_id.is_empty(),egui::Button::new("Duplicate page")).clicked()
                        && self.command(json!({"command":"page_duplicate","id":page_id,"name":format!("{} copy",string(&page,"name"))})) {
                        self.notice = "Page duplicated".into();
                    }
                });
                ui.add_enabled_ui(!page_id.is_empty(), |ui| {
                    let label = ui.label("Page name");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.page_name.name).desired_width((ui.available_width()-115.0).max(140.0))).labelled_by(label.id);
                        if ui.add_enabled(!self.page_name.name.trim().is_empty(),egui::Button::new("Rename")).clicked()
                            && self.command(json!({"command":"page_rename","id":page_id,"name":self.page_name.name.trim()})) {
                            self.page_name.original = self.page_name.name.clone(); self.notice = "Page renamed".into();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let order = profile["page_order"].as_array();
                        let position = order.and_then(|order| order.iter().position(|id| id == page_id));
                        for (label, direction, enabled) in [
                            ("Move earlier", -1, position.is_some_and(|p| p > 0)),
                            ("Move later", 1, position.is_some_and(|p| order.is_some_and(|o| p + 1 < o.len())))
                        ] {
                            if ui.add_enabled(enabled,egui::Button::new(label)).clicked() {
                                self.command(json!({"command":"page_move","id":page_id,"direction":direction}));
                            }
                        }
                        if ui.button("Delete page…").clicked() {
                            self.pending_delete = Some(PendingDelete { profile: false, id: page_id.to_owned(), name: page_label(&page) });
                        }
                    });
                });
            });
        });
    }
    fn deletion_dialog(&mut self, ctx: &egui::Context) {
        let Some(target) = self.pending_delete.clone() else {
            return;
        };
        let kind = if target.profile { "profile" } else { "page" };
        let response = egui::Modal::new(egui::Id::new("delete_library_item")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(format!("Delete {kind}?"));
            ui.label(format!("Delete {kind} \"{}\"{}?", target.name,
                if target.profile { " and all of its pages" } else { " and its assigned actions" }));
            ui.label("This cannot be undone.");
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() { self.pending_delete = None; }
                if ui.add(egui::Button::new(format!("Delete {kind}")).fill(Color32::from_rgb(105,39,43))).clicked() {
                    self.pending_delete = None;
                    if self.command(json!({"command":if target.profile { "profile_delete" } else { "page_delete" },"id":target.id})) {
                        self.notice = format!("{} deleted", if target.profile { "Profile" } else { "Page" });
                    }
                }
            });
        });
        if response.should_close() {
            self.pending_delete = None;
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
        ui.small("Preview your keys and touch strip.");
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
        if self.container_navigation.last().is_some_and(|nested| {
            slot.as_ref()
                .is_none_or(|parent| string(parent, "context") != nested.root_context)
        }) {
            self.container_navigation.clear();
        }
        if let Some(nested) = self.container_navigation.last().cloned() {
            ui.heading(selection.label());
            let previous = self
                .container_navigation
                .iter()
                .rev()
                .nth(1)
                .and_then(|target| {
                    slot.as_ref()
                        .and_then(|root| nested_snapshot(root, &target.child_context))
                })
                .or(slot.as_ref());
            let back_label = match previous.map(|parent| string(parent, "uuid")) {
                Some("com.elgato.streamdeck.keys.adaptor") => "Back to Action Trigger",
                Some("com.elgato.streamdeck.dial.stack") => "Back to Dial Stack",
                _ => "Back",
            };
            if ui.button(back_label).clicked() {
                self.container_navigation.pop();
                ui.ctx().request_repaint();
                return;
            }
            let child = slot
                .as_ref()
                .and_then(|parent| nested_snapshot(parent, &nested.child_context))
                .cloned();
            if let Some(child) = child {
                show_dial_gesture_descriptions(ui, &child);
                if child["uuid"] == "com.elgato.streamdeck.keys.adaptor" {
                    self.trigger_settings_editor(ui, &child);
                } else {
                    self.multi_settings_editor(ui, &child);
                }
            } else {
                ui.weak("This nested action is no longer available. Use Back to return.");
            }
            return;
        }
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
        if let Some(slot) = &slot {
            if slot["available"] == false {
                ui.colored_label(
                    Color32::YELLOW,
                    "This action is unavailable. Its saved settings are retained.",
                );
                ui.label(string(slot, "uuid"));
            }
            let mut pinned = slot["pinned"].as_bool().unwrap_or(false);
            if ui
                .checkbox(&mut pinned, "Pin across pages")
                .on_hover_text("Keep this control on every page in this profile")
                .changed()
            {
                self.command(
                    json!({"command":"slot_pin","context":slot["context"],"pinned":pinned}),
                );
            }
        }
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
        if slot
            .as_ref()
            .is_some_and(|slot| slot["has_inspector"] == true)
            && ui
                .button("Plugin settings…")
                .on_hover_text("Open this plugin's own configuration window")
                .clicked()
        {
            self.command(
                json!({"command":"open_inspector","context":slot.as_ref().unwrap()["context"]}),
            );
        }
        ui.label("Action");
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
                    a["multi_only"] != true
                        && a["controllers"]
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
        if !selection.dial {
            let active_profile = string(&self.snapshot["library"], "active_profile");
            let readonly =
                self.snapshot["library"]["profiles"]
                    .as_array()
                    .is_some_and(|profiles| {
                        profiles.iter().any(|profile| {
                            string(profile, "id") == active_profile && profile["readonly"] == true
                        })
                    });
            if ui
                .add_enabled(!readonly, egui::Button::new("Create folder here"))
                .on_hover_text(
                    "Assign a new folder to this key. Rename it using Manage profiles & pages.",
                )
                .clicked()
                && self.command(
                    json!({"command":"folder_create","name":"New folder","index":selection.index}),
                )
            {
                draft.dirty = false;
                self.notice = "Folder created".into();
            }
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(slot.is_some() && !stale, |ui| {
            let title_label = ui.label("Title on device");
            if ui.text_edit_singleline(&mut draft.title).labelled_by(title_label.id).changed() { draft.dirty = true; }
            ui.add_space(8.0);
            ui.label(RichText::new("Settings").strong());
            if let Some(slot) = &slot { show_dial_gesture_descriptions(ui, slot); }
            if slot.as_ref().is_some_and(|s| s["uuid"]=="com.elgato.streamdeck.system.multimedia"&&s["controller"]=="Encoder"&&s["settings"]["actionIdx"]==18) {
                if let Some(volume)=self.snapshot.get("volume").filter(|v|v.is_object()) {
                    ui.label(format!("Current volume: {}{}",slot.as_ref().and_then(|s|s["feedback"]["value"].as_str()).unwrap_or("--"),if volume["muted"]==true {" (muted)"}else{""}));
                    ui.small(string(volume,"name"));
                }else{ui.weak("Reading current system volume…");}
            }

            self.settings_editor(ui, &mut draft, slot.as_ref().map(|s|string(s,"uuid")).unwrap_or(""));
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
    fn container_request(&mut self, request: Value) -> bool {
        let command = string(&request, "command").to_owned();
        let context = string(&request, "context").to_owned();
        if matches!(
            command.as_str(),
            "edit_nested_multi" | "edit_nested_trigger"
        ) {
            let target = self
                .slot(self.selected)
                .and_then(|root| nested_container_target(root, &context));
            if let Some(target) = target {
                if self
                    .container_navigation
                    .last()
                    .is_none_or(|current| current.child_context != target.child_context)
                {
                    self.container_navigation.push(target);
                }
                return true;
            }
            self.error = "The nested action is no longer available.".into();
            return false;
        }
        let success = self.command(request);
        match command.as_str() {
            "multi_save" => self.multi_editor.saved(&context, success),
            "stack_save" => self.stack_editor.saved(&context, success),
            "trigger_save" => self.trigger_editor.saved(&context, success),
            _ => {}
        }
        success
    }

    fn multi_settings_editor(&mut self, ui: &mut egui::Ui, parent: &Value) {
        let actions = self.snapshot["actions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let outputs: Vec<(String, String)> =
            serde_json::from_value(self.snapshot["audio_outputs"].clone()).unwrap_or_default();
        for request in self.multi_editor.show(ui, parent, &actions, &outputs) {
            if !self.container_request(request) {
                break;
            }
        }
    }

    fn trigger_settings_editor(&mut self, ui: &mut egui::Ui, parent: &Value) {
        let actions = self.snapshot["actions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let outputs: Vec<(String, String)> =
            serde_json::from_value(self.snapshot["audio_outputs"].clone()).unwrap_or_default();
        for request in self.trigger_editor.show(ui, parent, &actions, &outputs) {
            if !self.container_request(request) {
                break;
            }
        }
    }

    fn settings_editor(&mut self, ui: &mut egui::Ui, draft: &mut Draft, assigned_action: &str) {
        if assigned_action == "com.elgato.streamdeck.keys.adaptor" {
            let parent = self.slot(self.selected).cloned().unwrap_or(json!({}));
            self.trigger_settings_editor(ui, &parent);
            return;
        }
        if assigned_action == crate::dial_stack::UUID {
            let parent = self.slot(self.selected).cloned().unwrap_or(json!({}));
            let actions = self.snapshot["actions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let outputs: Vec<(String, String)> =
                serde_json::from_value(self.snapshot["audio_outputs"].clone()).unwrap_or_default();
            for request in self.stack_editor.show(ui, &parent, &actions, &outputs) {
                if !self.container_request(request) {
                    break;
                }
            }
            return;
        }
        if matches!(
            assigned_action,
            "com.elgato.streamdeck.multiactions.routine"
                | "com.elgato.streamdeck.multiactions.routine2"
                | "com.elgato.streamdeck.multiactions.random"
        ) {
            let parent = self.slot(self.selected).cloned().unwrap_or(json!({}));
            self.multi_settings_editor(ui, &parent);
            return;
        }
        ui.add_enabled_ui(!draft.raw_dirty, |ui| {
            let outputs: Vec<(String, String)> =
                serde_json::from_value(self.snapshot["audio_outputs"].clone()).unwrap_or_default();
            if let Some(result) =
                crate::soundboard_editor::edit(ui, assigned_action, &mut draft.settings, &outputs)
            {
                if result.changed {
                    draft.numbers = numeric_strings(&draft.settings);
                    draft.dirty = true;
                }
                if result.refresh_outputs {
                    self.command(json!({"command":"soundboard_refresh"}));
                }
                return;
            }
            if let Some(changed) = crate::system_editor::edit(
                ui,
                assigned_action,
                self.selected.controller(),
                &mut draft.settings,
            ) {
                if changed {
                    draft.numbers = numeric_strings(&draft.settings);
                    draft.dirty = true;
                }
                return;
            }
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
        self.profile_management(ui);
        ui.add_space(8.0);
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
                egui::Button::new("Import profile"),
            )
            .clicked()
            && self.command(json!({"command":"import_profile","path":self.profile_path.trim()}))
        {
            self.notice = "Profile added to your library. Existing profiles are preserved.".into();
            self.profile_path.clear();
        }
        ui.small("Import adds a profile to your library.");
    }
}

// Retain application drafts across actual native window destruction. Textures
// belong to the old renderer and must be released before eframe destroys it.
struct EditorWindow {
    app: Option<Desktop>,
    saved: Arc<Mutex<Option<Desktop>>>,
}
impl Drop for EditorWindow {
    fn drop(&mut self) {
        if let Some(mut app) = self.app.take() {
            app.previews.clear();
            app.window.detach();
            *self.saved.lock().unwrap() = Some(app);
        }
    }
}
impl eframe::App for EditorWindow {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.as_mut().unwrap().logic(ctx, frame);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.as_mut().unwrap().ui(ui, frame);
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.as_mut().unwrap().previews.clear();
    }
}
impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.controller.is_running() {
            self.window.request(crate::instance::Command::Quit);
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.closing {
            self.window.request(crate::instance::Command::Hide);
        }
        self.closing |= self.window.apply(ctx);
        if self.closing && self.capture.is_some() {
            // A user/tray hide during capture must return an explicit failure,
            // not leave the capture process waiting in background forever.
            self.window.request(crate::instance::Command::Quit);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.capture_if_requested(ui.ctx());
        self.refresh(ui.ctx());
        if self.snapshot["running"] == false {
            self.window.request(crate::instance::Command::Quit);
            return;
        }
        ui.ctx().request_repaint_after(POLL);
        egui::Frame::central_panel(ui.style())
            .inner_margin(20)
            .show(ui, |ui| {
                self.toolbar(ui);
                self.profile_navigation(ui);
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
        self.deletion_dialog(ui.ctx());
        if let Some(preferences) = &mut self.preferences {
            preferences.ui(ui.ctx());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_window_recreation_preserves_unsaved_drafts_and_selection() {
        let state = tempfile::tempdir().unwrap();
        let running = crate::host::start(crate::host::HostOptions {
            serial: None,
            plugins: vec![],
            profile: None,
            state_dir: state.path().to_owned(),
            port: 0,
            simulate: true,
            seconds: None,
        })
        .unwrap();
        let window = crate::window::Window::new(true);
        let mut app = Desktop::new(
            running.controller.clone(),
            egui::Context::default(),
            window.clone(),
        );
        app.selected = Selection {
            dial: true,
            index: 2,
        };
        let mut draft = Draft::from_slot(None);
        draft.title = "Unsaved title".into();
        draft.raw = "{unfinished".into();
        draft.raw_dirty = true;
        draft.dirty = true;
        app.drafts.insert(app.selected, draft);
        app.plugin_path = "/pending/plugin".into();
        let saved = Arc::new(Mutex::new(None));
        drop(EditorWindow {
            app: Some(app),
            saved: saved.clone(),
        });
        assert_eq!(
            window.request(crate::instance::Command::Status)["window_open"],
            false
        );
        assert!(running.controller.is_running());
        let mut app = saved.lock().unwrap().take().unwrap();
        app.attach(egui::Context::default());
        let draft = app.drafts.get(&app.selected).unwrap();
        assert_eq!(draft.title, "Unsaved title");
        assert_eq!(draft.raw, "{unfinished");
        assert!(draft.dirty && draft.raw_dirty);
        assert_eq!(app.plugin_path, "/pending/plugin");
        assert_eq!(
            window.request(crate::instance::Command::Status)["editor_generation"],
            2
        );
    }

    #[test]
    fn nested_multi_navigation_resolves_runtime_snapshots_without_following_raw_data() {
        let parent = json!({"context":"trigger","uuid":"com.elgato.streamdeck.keys.adaptor",
            "raw":{"context":"fake","uuid":crate::multi::ROUTINE},
            "trigger_children":[null,{"context":"sequence","uuid":crate::multi::SWITCH,
                "multi_children":[[{"context":"counter","uuid":"com.elgato.counter.action"}],[]]},null]});
        let target = nested_container_target(&parent, "sequence").unwrap();
        assert_eq!(target.root_context, "trigger");
        assert_eq!(target.child_context, "sequence");
        assert_eq!(
            nested_snapshot(&parent, "counter").unwrap()["uuid"],
            "com.elgato.counter.action"
        );
        assert!(nested_container_target(&parent, "counter").is_none());
        assert!(nested_container_target(&parent, "trigger").is_none());
        assert!(nested_snapshot(&parent, "fake").is_none());
        assert!(nested_snapshot(&parent, "").is_none());
        assert!(nested_snapshot(&parent, "removed").is_none());
    }

    #[test]
    fn stack_trigger_multi_navigation_keeps_one_root_and_returns_one_level() {
        let root = json!({"context":"stack","uuid":"com.elgato.streamdeck.dial.stack","stack_children":[
            {"context":"trigger","uuid":"com.elgato.streamdeck.keys.adaptor","trigger_children":[
                null,{"context":"sequence","uuid":crate::multi::ROUTINE,"multi_children":[[],[]]},null
            ]}
        ]});
        let trigger = nested_container_target(&root, "trigger").unwrap();
        let sequence = nested_container_target(&root, "sequence").unwrap();
        assert_eq!(trigger.root_context, "stack");
        assert_eq!(sequence.root_context, "stack");
        let mut path = vec![trigger, sequence];
        assert_eq!(path.pop().unwrap().child_context, "sequence");
        assert_eq!(path.last().unwrap().child_context, "trigger");
        assert_eq!(
            nested_snapshot(&root, &path[0].child_context).unwrap()["uuid"],
            "com.elgato.streamdeck.keys.adaptor"
        );
        assert_eq!(path.pop().unwrap().child_context, "trigger");
        assert!(path.is_empty());
    }

    #[test]
    fn dial_descriptions_use_only_present_effective_strings_and_follow_updates() {
        let mut snapshot = json!({"controller":"Encoder","trigger_description":{
            "rotate":" Adjust volume ","push":"","touch":"Mute","longTouch":"   ","future":"Ignored"
        },"raw":{"Encoder":{"TriggerDescription":{"Push":"Must not be inferred"}}}});
        assert_eq!(
            dial_gesture_descriptions(&snapshot),
            vec![("Turn", "Adjust volume"), ("Tap", "Mute")]
        );
        snapshot["trigger_description"] = json!({"push":"Play","longTouch":"Reset","rotate":false});
        assert_eq!(
            dial_gesture_descriptions(&snapshot),
            vec![("Press", "Play"), ("Hold", "Reset")]
        );
        snapshot["trigger_description"] = json!({});
        assert!(dial_gesture_descriptions(&snapshot).is_empty());
        snapshot["trigger_description"] = json!({"rotate":"This is a key action"});
        snapshot["controller"] = json!("Keypad");
        assert!(dial_gesture_descriptions(&snapshot).is_empty());
    }

    #[test]
    fn nested_encoder_description_render_is_read_only_and_uses_child_snapshot() {
        let root = json!({"context":"stack","controller":"Encoder","trigger_description":{"push":"Change action"},
            "stack_children":[{"context":"nested-trigger","controller":"Encoder","uuid":"com.elgato.streamdeck.keys.adaptor",
                "trigger_description":{"rotate":"Choose item","touch":"Run item"}}]});
        let child = nested_snapshot(&root, "nested-trigger").unwrap();
        assert_eq!(
            dial_gesture_descriptions(child),
            vec![("Turn", "Choose item"), ("Tap", "Run item")]
        );
        let before = root.clone();
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            show_dial_gesture_descriptions(ui, child)
        });
        output.drop_without_applying_deltas();
        assert_eq!(root, before);
    }

    #[test]
    fn offline_header_waits_for_physical_usb_and_marks_last_known_firmware() {
        let mut snapshot = json!({"device":"serial-123","firmware":"2.0.3","connected":false,
            "simulate":false,"device_online":true,"connection_error":"Device not found"});
        assert_eq!(connection_label(&snapshot), "Waiting for USB");
        assert!(waiting_for_usb(&snapshot));
        assert_eq!(
            device_summary(&snapshot),
            "Device serial-123   ·   Firmware 2.0.3 (last known)"
        );
        snapshot["connected"] = json!(true);
        assert_eq!(connection_label(&snapshot), "USB connected");
        assert!(!waiting_for_usb(&snapshot));
        assert_eq!(
            device_summary(&snapshot),
            "Device serial-123   ·   Firmware 2.0.3"
        );
    }

    #[test]
    fn missing_firmware_and_simulation_do_not_claim_a_live_firmware_version() {
        for firmware in ["", "unknown", "UNKNOWN", "simulation"] {
            let snapshot = json!({"device":"serial-123","firmware":firmware,"connected":false,"simulate":false});
            assert_eq!(device_summary(&snapshot), "Device serial-123");
        }
        let simulated = json!({"device":"simulated-streamdeck-plus","firmware":"simulation", "connected":false,
            "simulate":true,"device_online":true});
        assert_eq!(connection_label(&simulated), "Simulation");
        assert!(!waiting_for_usb(&simulated));
        assert!(!device_summary(&simulated).contains("Firmware"));
        assert_eq!(device_summary(&json!({})), "Stream Deck +");
    }

    #[test]
    fn profile_name_polling_preserves_edits_but_resets_on_selection_change() {
        let mut name = NameDraft::default();
        name.reconcile("profile-a", "Original");
        name.name = "Unfinished rename".into();
        name.reconcile("profile-a", "Remote rename");
        assert_eq!(name.name, "Unfinished rename");
        name.reconcile("profile-b", "Another profile");
        assert_eq!(name.name, "Another profile");
        name.reconcile("profile-b", "Updated externally");
        assert_eq!(name.name, "Updated externally");
    }
    #[test]
    fn export_filename_cannot_introduce_directories() {
        assert_eq!(
            export_file_name("../../Desk\\work"),
            "_.._Desk_work.streamDeckProfile"
        );
        assert_eq!(export_file_name("..."), "Profile.streamDeckProfile");
        assert_eq!(export_file_name("Musikk æ"), "Musikk æ.streamDeckProfile");
        assert!(!export_file_name("name\nnext").contains('\n'));
    }
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
