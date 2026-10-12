//! Native application preferences; changes use the same implementation as CLI.
use crate::startup::{Startup, Status};
use crate::{device_session::Controls, host::Controller};
use eframe::egui;
use serde_json::{Value, json};
use std::path::Path;

pub struct Preferences {
    pub open: bool,
    tab: u8,
    startup: Result<Startup, String>,
    status: Option<Status>,
    error: String,
    device_id: String,
    brightness: u8,
    device_dirty: bool,
    device_error: String,
    screen_device: String,
    screen_minutes: u32,
    screen_lock: bool,
    screen_image: Option<String>,
    screen_dirty: bool,
    screen_error: String,
    screen_job: Option<std::sync::mpsc::Receiver<std::result::Result<(), String>>>,
}
impl Preferences {
    pub fn new(state: &Path, simulate: bool) -> Self {
        Self {
            open: false,
            tab: 0,
            startup: Startup::current(state, simulate).map_err(|error| format!("{error:#}")),
            status: None,
            error: String::new(),
            device_id: String::new(),
            brightness: crate::brightness::DEFAULT,
            device_dirty: false,
            device_error: String::new(),
            screen_device: String::new(),
            screen_minutes: 0,
            screen_lock: true,
            screen_image: None,
            screen_dirty: false,
            screen_error: String::new(),
            screen_job: None,
        }
    }
    pub fn show(&mut self) {
        self.open = true;
        self.device_dirty = false;
        self.screen_dirty = false;
        self.screen_image = None;
        self.refresh();
    }
    pub fn show_screensaver(&mut self) {
        self.show();
        self.tab = 1;
    }
    fn refresh(&mut self) {
        match &self.startup {
            Ok(startup) => match startup.status() {
                Ok(status) => {
                    self.status = Some(status);
                    self.error.clear();
                }
                Err(error) => {
                    self.status = None;
                    self.error = format!("{error:#}");
                }
            },
            Err(error) => {
                self.status = None;
                self.error = error.clone();
            }
        }
    }
    fn set(&mut self, enabled: bool) {
        if let Ok(startup) = &self.startup {
            match startup.set_enabled(enabled) {
                Ok(status) => {
                    self.status = Some(status);
                    self.error.clear();
                }
                Err(error) => self.error = format!("{error:#}"),
            }
        }
    }
    fn device(&mut self, ui: &mut egui::Ui, snapshot: &Value, controller: &Controller) {
        ui.heading("Device brightness");
        let desired: Controls =
            serde_json::from_value(snapshot["device_settings"]["desired"].clone())
                .unwrap_or_default();
        let device = snapshot["device"].as_str().unwrap_or_default();
        if self.device_id != device {
            self.device_id = device.into();
            self.device_dirty = false;
            self.device_error.clear();
        }
        if !self.device_dirty {
            self.brightness = desired.brightness.unwrap_or(crate::brightness::DEFAULT);
        }
        if ui
            .add(
                egui::Slider::new(&mut self.brightness, 0..=100)
                    .suffix("%")
                    .text("Brightness"),
            )
            .changed()
        {
            self.device_dirty = true;
        }
        if desired.brightness.is_none() {
            ui.weak(
                "No brightness setting saved yet. Apply a value to restore it after reconnecting.",
            );
        }
        if ui
            .add_enabled(
                self.device_dirty
                    || desired.brightness.is_none()
                    || snapshot["device_settings"]["error"].is_string(),
                egui::Button::new("Apply brightness"),
            )
            .clicked()
        {
            let controls = Controls {
                brightness: Some(self.brightness),
                ..desired
            };
            match controller.command(json!({"command":"device_controls","settings":controls})) {
                Ok(()) => {
                    self.device_dirty = false;
                    self.device_error.clear();
                }
                Err(error) => self.device_error = format!("{error:#}"),
            }
        }
        if snapshot["simulate"] == true {
            ui.weak("Simulation: no device settings are sent.");
        } else if snapshot["connected"] != true {
            ui.weak("Saved settings apply when the device reconnects.");
        } else if let Some(value) = snapshot["device_settings"]["applied"]["brightness"].as_u64() {
            ui.weak(format!("Sent to device: {value}%"));
        }
        if let Some(error) = snapshot["device_settings"]["error"].as_str() {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if !self.device_error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.device_error);
        }
    }
    fn screensaver(&mut self, ui: &mut egui::Ui, snapshot: &Value, controller: &Controller) {
        let completed = self
            .screen_job
            .as_ref()
            .and_then(|job| match job.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Image loader stopped".into()))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = completed {
            self.screen_job = None;
            match result {
                Ok(()) => {
                    self.screen_dirty = false;
                    self.screen_image = None;
                    self.screen_error.clear();
                }
                Err(error) => {
                    self.screen_dirty = true;
                    self.screen_error = error;
                }
            }
        }
        if self.screen_job.is_some() {
            ui.heading("Screensaver");
            ui.spinner();
            ui.label("Loading screensaver… Your controls keep working.");
            return;
        }
        let device = snapshot["device"].as_str().unwrap_or_default();
        let saved = &snapshot["screensaver"]["preferences"];
        if self.screen_device != device {
            self.screen_device = device.into();
            self.screen_dirty = false;
            self.screen_image = None;
            self.screen_error.clear();
        }
        if !self.screen_dirty {
            self.screen_minutes = saved["after_minutes"].as_u64().unwrap_or(0) as u32;
            self.screen_lock = saved["disable_on_lock"] != true;
        }
        ui.heading("Screensaver");
        ui.weak("Device input and display updates reset the sleep timer.");
        let old = self.screen_minutes;
        egui::ComboBox::from_id_salt("screen_after")
            .selected_text(if old == 0 {
                "Never".into()
            } else {
                format!("After {old} minutes")
            })
            .show_ui(ui, |ui| {
                for minutes in [0, 5, 15, 30, 60, 120] {
                    ui.selectable_value(
                        &mut self.screen_minutes,
                        minutes,
                        if minutes == 0 {
                            "Never".into()
                        } else {
                            format!("After {minutes} minutes")
                        },
                    );
                }
            });
        self.screen_dirty |= old != self.screen_minutes;
        self.screen_dirty |= ui
            .checkbox(
                &mut self.screen_lock,
                "Show screensaver when the desktop locks",
            )
            .changed();
        let image = self
            .screen_image
            .as_deref()
            .or_else(|| saved["image"].as_str())
            .filter(|path| !path.is_empty());
        ui.label(
            image
                .and_then(|path| Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Deckforge logo".into()),
        );
        ui.horizontal(|ui| {
            if ui.button("Choose image or GIF…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Screensaver", &["png", "jpg", "jpeg", "gif"])
                    .pick_file()
            {
                self.screen_image = Some(path.to_string_lossy().into_owned());
                self.screen_dirty = true;
            }
            if ui.button("Use Deckforge logo").clicked() {
                self.screen_image = Some(String::new());
                self.screen_dirty = true;
            }
        });
        ui.weak("Images fit the display with black borders. Animation pauses while the desktop is locked.");
        if ui
            .add_enabled(self.screen_dirty, egui::Button::new("Apply screensaver"))
            .clicked()
        {
            let mut request = json!({"command":"screensaver","device":self.screen_device,"after_minutes":self.screen_minutes,"disable_on_lock":!self.screen_lock});
            if let Some(image) = &self.screen_image {
                request["image"] = json!(image);
            }
            let controller = controller.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            match std::thread::Builder::new()
                .name("deckforge-image-loader".into())
                .spawn(move || {
                    let _ = sender.send(
                        controller
                            .command(request)
                            .map_err(|error| format!("{error:#}")),
                    );
                }) {
                Ok(_) => self.screen_job = Some(receiver),
                Err(error) => self.screen_error = format!("Start image loader: {error}"),
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.screen_dirty, egui::Button::new("Preview on device"))
                .clicked()
                && let Err(error) = controller.command(json!({"command":"sleep_device"}))
            {
                self.screen_error = format!("{error:#}");
            }
            if ui
                .add_enabled(
                    snapshot["screensaver"]["active"] == true,
                    egui::Button::new("Wake device"),
                )
                .clicked()
                && let Err(error) = controller.command(json!({"command":"wake_device"}))
            {
                self.screen_error = format!("{error:#}");
            }
        });
        if snapshot["session"]["locked"].is_null() && snapshot["simulate"] != true {
            ui.weak("Desktop lock detection is currently unavailable.");
        }
        if let Some(error) = snapshot["screensaver"]["error"].as_str() {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if !self.screen_error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.screen_error);
        }
    }
    pub fn ui(&mut self, ctx: &egui::Context, snapshot: &Value, controller: &Controller) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Preferences").open(&mut open).collapsible(false)
            .resizable(true).default_width(460.0).default_height(520.0).vscroll(true).max_height((ctx.content_rect().height()-60.0).max(260.0))
            .default_pos(ctx.content_rect().center() - egui::vec2(230.0, 285.0)).show(ctx,|ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab,0,"Device");
                    ui.selectable_value(&mut self.tab,1,"Screensaver");
                    ui.selectable_value(&mut self.tab,2,"Startup & about");
                });
                ui.separator();
                if self.tab==0 {self.device(ui,snapshot,controller);return;}
                if self.tab==1 {self.screensaver(ui,snapshot,controller);return;}
                ui.heading("Startup");
                let mut enabled=self.status.as_ref().is_some_and(|s|s.enabled&&s.this_session);
                if ui.add_enabled(self.status.is_some(),egui::Checkbox::new(&mut enabled,"Start this session when I log in")).changed() {
                    self.set(enabled);
                }
                ui.label("Deckforge starts in the background. Open the launcher or tray to edit your controls.");
                if self.status.as_ref().is_some_and(|s|s.enabled&&!s.this_session) {
                    ui.label("Another Deckforge session is configured to start at login. Enabling this session replaces it.");
                    if ui.button("Disable login startup").clicked(){self.set(false);}
                }
                if !self.error.is_empty(){ui.colored_label(egui::Color32::LIGHT_RED,&self.error);}
                if ui.button("Refresh startup setting").clicked(){self.refresh();}
                if let Ok(startup)=&self.startup {
                    ui.collapsing("Startup file",|ui|{ui.label(startup.path().display().to_string());});
                }
                ui.separator();
                ui.heading("About Deckforge");
                ui.label(format!("Version {}",env!("CARGO_PKG_VERSION")));
                ui.label("An independent Linux Stream Deck app, made with AI.");
                ui.hyperlink_to("Source code and development progress", "https://github.com/JackSkellet/deckforge");
                ui.weak("Plugin and profile compatibility is still in development.");
            });
        self.open = open;
    }
}
