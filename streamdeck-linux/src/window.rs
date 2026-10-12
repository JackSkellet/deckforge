//! Native window lifecycle mailbox. Background mode owns no renderer or window.
use crate::{instance::Command, tray::Event};
use eframe::egui;
use serde_json::{Value, json};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Window(Arc<(Mutex<State>, Condvar)>);
struct State {
    context: Option<egui::Context>,
    visible: bool,
    window_open: bool,
    editor_generation: u64,
    focus: bool,
    quitting: bool,
    tray_online: bool,
    tray_error: Option<String>,
}
impl Window {
    pub fn new(visible: bool) -> Self {
        Self(Arc::new((
            Mutex::new(State {
                context: None,
                visible,
                window_open: false,
                editor_generation: 0,
                focus: false,
                quitting: false,
                tray_online: false,
                tray_error: None,
            }),
            Condvar::new(),
        )))
    }
    pub fn attach(&self, context: egui::Context) {
        let mut state = self.0.0.lock().unwrap();
        context.request_repaint();
        state.context = Some(context);
        state.window_open = true;
        state.editor_generation += 1;
    }
    pub fn detach(&self) {
        let mut state = self.0.0.lock().unwrap();
        state.context = None;
        state.window_open = false;
    }
    pub fn repaint(&self) {
        if let Some(context) = &self.0.0.lock().unwrap().context {
            context.request_repaint();
        }
    }
    pub fn request(&self, command: Command) -> Value {
        let mut state = self.0.0.lock().unwrap();
        if command != Command::Status && !state.quitting {
            match command {
                Command::Show => {
                    state.visible = true;
                    state.focus = true;
                }
                Command::Hide | Command::Close => state.visible = false,
                Command::Quit => state.quitting = true,
                Command::Status => {}
            }
            if let Some(context) = &state.context {
                context.request_repaint();
            }
            self.0.1.notify_one();
        }
        json!({"pid":std::process::id(),"visible":state.visible,
            "window_open":state.window_open,"editor_generation":state.editor_generation,
            "quitting":state.quitting,"tray_online":state.tray_online,"tray_error":state.tray_error})
    }
    /// Sleep without a GUI event loop. The timeout lets the caller observe host
    /// signal shutdown without making the host's redraw callback take its lock.
    pub fn wait(&self) -> bool {
        let state = self.0.0.lock().unwrap();
        let (state, _) = self
            .0
            .1
            .wait_timeout_while(state, Duration::from_secs(1), |state| {
                !state.visible && !state.quitting
            })
            .unwrap();
        state.visible && !state.quitting
    }
    pub fn tray(&self, event: Event) {
        match event {
            Event::Show => {
                self.request(Command::Show);
            }
            Event::Quit => {
                self.request(Command::Quit);
            }
            Event::Online(online) => {
                let mut state = self.0.0.lock().unwrap();
                state.tray_online = online;
                if online {
                    state.tray_error = None;
                }
            }
            Event::Error(error) => {
                eprintln!("Tray: {error}");
                self.0.0.lock().unwrap().tray_error = Some(error);
            }
        }
    }
    pub fn quitting(&self) -> bool {
        self.0.0.lock().unwrap().quitting
    }
    /// Returns true when the current window must close. The host remains alive.
    pub fn apply(&self, context: &egui::Context) -> bool {
        let mut state = self.0.0.lock().unwrap();
        if state.quitting || !state.visible {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
            true
        } else {
            if std::mem::take(&mut state.focus) {
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                context.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visibility_coalesces_but_quit_is_sticky() {
        let window = Window::new(true);
        window.request(Command::Hide);
        window.request(Command::Show);
        assert_eq!(window.request(Command::Status)["visible"], true);
        window.request(Command::Quit);
        window.request(Command::Show);
        assert!(window.quitting());
        assert!(!window.wait());
    }
    #[test]
    fn tray_recovery_clears_error() {
        let window = Window::new(false);
        window.tray(Event::Error("No watcher".into()));
        window.tray(Event::Online(true));
        let status = window.request(Command::Status);
        assert_eq!(status["tray_online"], true);
        assert!(status["tray_error"].is_null());
        assert_eq!(status["visible"], false);
    }
    #[test]
    fn detached_window_retains_a_show_received_while_closing() {
        let window = Window::new(true);
        window.attach(egui::Context::default());
        window.request(Command::Hide);
        window.request(Command::Show);
        window.detach();
        assert!(window.wait());
        let status = window.request(Command::Status);
        assert_eq!(status["window_open"], false);
        assert_eq!(status["editor_generation"], 1);
    }
}
