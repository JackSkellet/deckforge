//! Native application preferences; changes use the same implementation as CLI.
use crate::startup::{Startup, Status};
use crate::{device_session::Controls, host::Controller};
use eframe::egui;
use serde_json::{Value, json};
use std::path::Path;

pub struct Preferences {
    pub open: bool,
    startup: Result<Startup, String>,
    status: Option<Status>,
    error: String,
    device_id: String,
    brightness: u8,
    device_dirty: bool,
    device_error: String,
}
impl Preferences {
    pub fn new(state: &Path, simulate: bool) -> Self {
        Self {
            open: false,
            startup: Startup::current(state, simulate).map_err(|error| format!("{error:#}")),
            status: None,
            error: String::new(),
            device_id: String::new(),
            brightness: crate::brightness::DEFAULT,
            device_dirty: false,
            device_error: String::new(),
        }
    }
    pub fn show(&mut self) {
        self.open = true;
        self.device_dirty = false;
        self.refresh();
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
    pub fn ui(&mut self, ctx: &egui::Context, snapshot: &Value, controller: &Controller) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Preferences").open(&mut open).collapsible(false)
            .resizable(false).default_width(460.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(230.0, 280.0)).show(ctx,|ui| {
                self.device(ui, snapshot, controller);
                ui.separator();
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
