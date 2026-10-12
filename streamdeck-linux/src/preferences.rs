//! Native application preferences; changes use the same implementation as CLI.
use crate::startup::{Startup, Status};
use eframe::egui;
use std::path::Path;

pub struct Preferences {
    pub open: bool,
    startup: Result<Startup, String>,
    status: Option<Status>,
    error: String,
}
impl Preferences {
    pub fn new(state: &Path, simulate: bool) -> Self {
        Self {
            open: false,
            startup: Startup::current(state, simulate).map_err(|error| format!("{error:#}")),
            status: None,
            error: String::new(),
        }
    }
    pub fn show(&mut self) {
        self.open = true;
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
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Preferences").open(&mut open).collapsible(false)
            .resizable(false).default_width(460.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(230.0, 200.0)).show(ctx,|ui| {
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
