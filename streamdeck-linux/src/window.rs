//! Native window lifecycle mailbox. Background mode owns no renderer or window.
use crate::{instance::Command, tray::Event};
use eframe::egui;
use serde_json::{Value, json};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU8, Ordering},
};
use std::time::Duration;

/// Use the same artwork as the launcher, including when run outside an install.
pub fn icon() -> anyhow::Result<tao::window::Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/deckforge.png"))?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(tao::window::Icon::from_rgba(
        image.into_raw(),
        width,
        height,
    )?)
}

#[derive(Clone)]
pub struct Window(Arc<(Mutex<State>, Condvar)>);
struct State {
    wakeup: Option<Arc<dyn Fn() + Send + Sync>>,
    visible: bool,
    minimized: bool,
    smart_visibility: Option<Arc<AtomicU8>>,
    window_open: bool,
    editor_generation: u64,
    focus: bool,
    quitting: bool,
    tray_online: bool,
    tray_error: Option<String>,
}
impl State {
    fn report_visibility(&self) {
        if let Some(flag) = &self.smart_visibility {
            flag.store(
                if self.quitting {
                    3
                } else if !self.visible {
                    0
                } else if self.minimized {
                    2
                } else {
                    1
                },
                Ordering::Release,
            );
        }
    }
}
impl Window {
    pub fn new(visible: bool) -> Self {
        Self(Arc::new((
            Mutex::new(State {
                wakeup: None,
                visible,
                minimized: false,
                smart_visibility: None,
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
        self.attach_wakeup(Arc::new(move || context.request_repaint()));
    }
    /// Both native renderers use the same lifecycle. Waking only schedules UI
    /// work; the callback must never perform host commands or synchronous I/O.
    pub fn attach_wakeup(&self, wakeup: Arc<dyn Fn() + Send + Sync>) {
        {
            let mut state = self.0.0.lock().unwrap();
            state.wakeup = Some(wakeup.clone());
            state.window_open = true;
            state.minimized = false;
            state.editor_generation += 1;
        }
        wakeup();
    }
    pub fn detach(&self) {
        let mut state = self.0.0.lock().unwrap();
        state.wakeup = None;
        state.window_open = false;
    }
    pub fn repaint(&self) {
        let wakeup = self.0.0.lock().unwrap().wakeup.clone();
        if let Some(wakeup) = wakeup {
            wakeup();
        }
    }
    pub fn request(&self, command: Command) -> Value {
        let mut state = self.0.0.lock().unwrap();
        if command != Command::Status && !state.quitting {
            match command {
                Command::Show => {
                    state.visible = true;
                    state.minimized = false;
                    state.focus = true;
                }
                Command::Hide | Command::Close => state.visible = false,
                Command::Quit => state.quitting = true,
                Command::Status => {}
            }
            self.0.1.notify_one();
        }
        state.report_visibility();
        let wakeup = (command != Command::Status)
            .then(|| state.wakeup.clone())
            .flatten();
        let status = json!({"pid":std::process::id(),"visible":state.visible,
            "window_open":state.window_open,"editor_generation":state.editor_generation,
            "quitting":state.quitting,"tray_online":state.tray_online,"tray_error":state.tray_error});
        drop(state);
        if let Some(wakeup) = wakeup {
            wakeup();
        }
        status
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
    pub fn track_smart_visibility(&self, flag: Arc<AtomicU8>) {
        let mut state = self.0.0.lock().unwrap();
        state.smart_visibility = Some(flag);
        state.report_visibility();
    }
    pub fn minimized(&self, minimized: bool) {
        let mut state = self.0.0.lock().unwrap();
        state.minimized = minimized;
        state.report_visibility();
    }
    pub fn quitting(&self) -> bool {
        self.0.0.lock().unwrap().quitting
    }
    /// Atomically consumes a focus request and returns (keep_open, focus).
    pub fn presentation(&self) -> (bool, bool) {
        let mut state = self.0.0.lock().unwrap();
        (
            !state.quitting && state.visible,
            std::mem::take(&mut state.focus),
        )
    }
    /// Legacy egui adapter; desktop WebKit uses presentation() directly.
    pub fn apply(&self, context: &egui::Context) -> bool {
        let (keep_open, focus) = self.presentation();
        if !keep_open {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
            true
        } else {
            if focus {
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
    fn smart_profile_guard_tracks_visibility_minimize_and_quit_immediately() {
        let window = Window::new(true);
        let flag = Arc::new(AtomicU8::new(0));
        window.track_smart_visibility(flag.clone());
        assert_eq!(flag.load(Ordering::Acquire), 1);
        window.minimized(true);
        assert_eq!(flag.load(Ordering::Acquire), 2);
        window.minimized(false);
        assert_eq!(flag.load(Ordering::Acquire), 1);
        window.request(Command::Hide);
        assert_eq!(flag.load(Ordering::Acquire), 0);
        window.request(Command::Show);
        assert_eq!(flag.load(Ordering::Acquire), 1);
        window.minimized(true);
        window.request(Command::Quit);
        assert_eq!(flag.load(Ordering::Acquire), 3);
    }

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
    fn generic_wakeup_runs_outside_lifecycle_lock() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let window = Window::new(false);
        let weak = Arc::downgrade(&window.0);
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        window.attach_wakeup(Arc::new(move || {
            let shared = weak.upgrade().unwrap();
            assert!(shared.0.try_lock().is_ok());
            calls.fetch_add(1, Ordering::Relaxed);
        }));
        window.request(Command::Show);
        assert_eq!(window.presentation(), (true, true));
        assert_eq!(window.presentation(), (true, false));
        assert_eq!(count.load(Ordering::Relaxed), 2);
        window.detach();
        window.request(Command::Hide);
        assert_eq!(count.load(Ordering::Relaxed), 2);
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
