//! Independent implementation of Elgato's public plugin WebSocket contract.
use crate::{
    device_session::{self, Draw, Event as DeviceEvent},
    hardware::InputEvent,
    profile,
    render::Renderer,
};
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use image::{DynamicImage, GenericImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::Cursor,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

mod appearance;
mod applications;
mod audio;
mod brightness;
mod children;
mod controller_background;
mod device;
mod device_management;
mod device_pool;
mod display;
mod display_targets;
mod encoder_assets;
mod encoder_routes;
mod export;
mod feedback;
mod file_open;
mod folder_exit;
mod history;
mod icon_packs;
mod image_cropper;
mod inspectors;
mod key_images;
mod library;
mod links;
mod localization;
mod marketplace;
mod mixer;
mod mixer_actions;
mod multiaction;
mod openapp;
mod plugins;
mod reference;
mod removal;
mod resources;
mod screensaver;
mod settings;
mod smart_profiles;
mod stack;
mod system_events;
#[cfg(test)]
mod text_mode_tests;
mod timer;
mod transfer;
mod transport;
mod trigger;
mod update_assets;
mod updates;
mod volume;
mod wheel;
use crate::device_model::Model;
use inspectors::Inspector;
use library::Library;

type Shared = Arc<Mutex<Host>>;

/// The native editor uses this in-process controller, without HTTP or a browser.
#[derive(Clone)]
pub struct Controller {
    shared: Shared,
}
impl Controller {
    pub fn is_running(&self) -> bool {
        self.shared.lock().unwrap().running
    }
    pub fn on_redraw(&self, callback: impl Fn() + Send + Sync + 'static) {
        self.shared.lock().unwrap().redraw = Some(Box::new(callback));
    }
    pub fn snapshot(&self) -> Value {
        snapshot(&self.shared.lock().unwrap())
    }
    /// Resolve only an installed action's declared icon, with file work outside
    /// the engine lock. The desktop bridge never accepts an arbitrary file URL.
    pub fn action_icon(&self, uuid: &str, generation: Option<u64>) -> Result<Option<String>> {
        let source = {
            let host = self.shared.lock().unwrap();
            host.actions
                .iter()
                .find(|action| action.uuid == uuid)
                .and_then(|action| {
                    let icon = action.raw["Icon"].as_str()?;
                    let plugin = host
                        .plugins
                        .iter()
                        .find(|plugin| plugin.id == action.plugin)?;
                    if generation.is_some_and(|expected| plugin.asset_generation != expected) {
                        return None;
                    }
                    Some((plugin.root.clone(), icon.to_owned()))
                })
        };
        let Some((root, icon)) = source else {
            return Ok(None);
        };
        let image = crate::action_icons::png(&root, &icon)?;
        Ok(Some(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(image)
        )))
    }
    pub fn mixer_control(&self, request: Value) -> Result<Value> {
        let mut host = self.shared.lock().unwrap();
        host.mixer_control(request)?;
        Ok(host.mixer_snapshot())
    }
    pub fn command_output(&self, request: &Value) -> Result<Value> {
        let host = self.shared.lock().unwrap();
        let context = request["context"]
            .as_str()
            .context("Action context missing")?;
        let owner = host
            .context_device(context)
            .context("Action no longer exists")?;
        if let Some(device) = request["device"].as_str() {
            anyhow::ensure!(device == owner, "Action belongs to another device");
        }
        let id = request["id"].as_str().context("Command run missing")?;
        Ok(serde_json::to_value(
            host.system_actions
                .as_ref()
                .context("No command results")?
                .command_results
                .preview(context, id)?,
        )?)
    }
    pub fn command(&self, request: Value) -> Result<()> {
        if request["command"] == "plugin_update_cancel" {
            return self.cancel_plugin_update(
                request["ticket"]
                    .as_str()
                    .context("Update ticket missing")?,
            );
        }
        if request["command"] == "appearance" {
            return self.appearance_command(request);
        }
        if request["command"] == "screensaver" {
            let (device, previous, path) = {
                let mut host = self.shared.lock().unwrap();
                let device = request["device"]
                    .as_str()
                    .unwrap_or(&host.device)
                    .to_owned();
                host.with_device(&device, |host| {
                    anyhow::ensure!(host.running, "Host has stopped");
                    Ok((
                        host.device.clone(),
                        host.saver_preferences(),
                        host.state_path.clone(),
                    ))
                })?
            };
            let prepared = screensaver::prepare(&request, &previous, &path)?;
            let mut host = self.shared.lock().unwrap();
            anyhow::ensure!(host.running, "Host stopped while loading");
            return host.with_device(&device, |host| host.apply_saver(prepared));
        }
        let mut host = self.shared.lock().unwrap();
        if !host.running {
            bail!("Host has stopped");
        }
        apply_control(&mut host, request)
    }
}
#[derive(Clone)]
pub struct HostOptions {
    pub model: Option<Model>,
    pub serial: Option<String>,
    pub plugins: Vec<PathBuf>,
    pub profile: Option<PathBuf>,
    pub state_dir: PathBuf,
    pub port: u16,
    pub simulate: bool,
    pub seconds: Option<u64>,
}
struct EmbeddedChannels {
    ready: std::sync::mpsc::Sender<Controller>,
    stop: tokio::sync::oneshot::Receiver<()>,
}
pub struct EmbeddedHost {
    pub controller: Controller,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<Result<()>>>,
}
impl Drop for EmbeddedHost {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(Err(e)) => eprintln!("Host shutdown: {e:#}"),
                Err(_) => eprintln!("Host thread panicked"),
                _ => {}
            }
        }
    }
}
pub fn start(options: HostOptions) -> Result<EmbeddedHost> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let worker = std::thread::spawn(move || {
        tokio::runtime::Runtime::new()?.block_on(run(
            options,
            Some(EmbeddedChannels {
                ready: ready_tx,
                stop: stop_rx,
            }),
        ))
    });
    match ready_rx.recv() {
        Ok(controller) => Ok(EmbeddedHost {
            controller,
            stop: Some(stop_tx),
            worker: Some(worker),
        }),
        Err(_) => {
            let result = worker
                .join()
                .map_err(|_| anyhow::anyhow!("Host thread panicked during startup"))?;
            result?;
            bail!("Host stopped before becoming ready")
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Slot {
    #[serde(default)]
    model: Model,
    #[serde(default, alias = "multi_children")]
    children: Vec<Vec<Slot>>,
    #[serde(default, alias = "multi_id")]
    child_id: String,
    #[serde(default)]
    in_multi_action: bool,
    /// Nested key actions keep the coordinates of their owning control.
    #[serde(skip)]
    event_coordinates: Option<(u8, u8)>,
    #[serde(default)]
    encoder_overrides: HashMap<String, String>,
    #[serde(default)]
    resources: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    asset_source: Option<transfer::AssetSource>,
    #[serde(default)]
    appearances: appearance::States,
    context: String,
    controller: String,
    index: u8,
    uuid: String,
    name: String,
    settings: Value,
    title: String,
    state: u64,
    feedback: Value,
    layout: String,
    #[serde(default)]
    raw: Value,
    #[serde(default)]
    image_changed: bool,
    #[serde(default)]
    title_changed: bool,
    #[serde(skip)]
    pinned: bool,
    #[serde(default)]
    states: Value,
    #[serde(default, with = "image_serde")]
    background: Option<DynamicImage>,
    #[serde(skip)]
    layout_cache: Option<Value>,
    #[serde(skip)]
    image_cache: HashMap<String, (String, DynamicImage)>,
    #[serde(skip)]
    display_warnings: Vec<String>,
    #[serde(skip)]
    preview: String,
    #[serde(skip)]
    software_icon: String,
}
#[derive(Clone, Serialize)]
struct Action {
    uuid: String,
    name: String,
    controllers: Vec<String>,
    plugin: String,
    #[serde(skip)]
    raw: Value,
}
struct Plugin {
    asset_generation: u64,
    id: String,
    name: String,
    root: PathBuf,
    manifest: Value,
    registration: String,
    runtime: plugins::Runtime,
    child: Option<Child>,
    sender: Option<mpsc::UnboundedSender<Value>>,
}
fn stop_child(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        if child.try_wait().ok().flatten().is_none() {
            // Give owned plugins a bounded chance to release held controls and
            // flush remote commands. A non-cooperating child is still reaped.
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
            }
            let deadline = std::time::Instant::now() + Duration::from_millis(750);
            while child.try_wait().ok().flatten().is_none() && std::time::Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
        }
        let _ = child.wait();
    }
}
impl Plugin {
    /// Revoke new output and give the independent writer a bounded flush window.
    /// This is transport completion, not acknowledgement of plugin side effects.
    fn drain_output(&mut self) -> bool {
        self.sender = None;
        self.runtime
            .output_drain
            .take()
            .is_none_or(|drain| drain.wait())
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.drain_output();
        stop_child(&mut self.child);
    }
}
mod image_serde {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        image: &Option<DynamicImage>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let value: Option<String> = image
            .as_ref()
            .map(|image| -> std::result::Result<String, S::Error> {
                let mut data = Cursor::new(Vec::new());
                image
                    .write_to(&mut data, image::ImageFormat::Png)
                    .map_err(serde::ser::Error::custom)?;
                Ok(STANDARD.encode(data.into_inner()))
            })
            .transpose()?;
        value.serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Option<DynamicImage>, D::Error> {
        let data = Option::<String>::deserialize(deserializer)?;
        data.map(|data| {
            if data.len() > 12 * 1024 * 1024 {
                return Err(serde::de::Error::custom("Saved image too large"));
            }
            let bytes = STANDARD.decode(data).map_err(serde::de::Error::custom)?;
            image::load_from_memory(&bytes).map_err(serde::de::Error::custom)
        })
        .transpose()
    }
}
#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    smart_profiles: smart_profiles::Configs,
    #[serde(default)]
    suppress_multi_feedback: bool,
    #[serde(default)]
    plugin_asset_ids: HashMap<PathBuf, (u64, u64)>,
    #[serde(default)]
    disabled_plugins: BTreeSet<PathBuf>,
    #[serde(default = "localization::default_preference")]
    plugin_language: String,
    #[serde(default)]
    application_mappings: applications::Mappings,
    #[serde(default)]
    devices: Vec<device_pool::Record>,
    #[serde(default)]
    active_scope: Option<String>,
    #[serde(default)]
    selected_scope: Option<String>,
    #[serde(default)]
    model: Model,
    #[serde(default)]
    mixer: crate::mixer::Config,
    #[serde(default)]
    screensavers: HashMap<String, screensaver::Preferences>,
    #[serde(default)]
    device_controls: HashMap<String, device_session::Controls>,
    #[serde(default)]
    device_config: Option<device::Config>,
    #[serde(default)]
    library: Option<Library>,
    #[serde(default)]
    reference_app: Option<PathBuf>,
    #[serde(default)]
    plugins: Vec<PathBuf>,
    slots: Vec<Slot>,
    #[serde(default)]
    globals: HashMap<String, Value>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum HeldControl {
    Key(u8),
    Dial(u8),
}
struct Host {
    history: history::History,
    smart_profiles: smart_profiles::Configs,
    smart: smart_profiles::Runtime,
    suppress_multi_feedback: bool,
    plugin_asset_ids: HashMap<PathBuf, (u64, u64)>,
    pending_updates: HashMap<String, updates::Pending>,
    previous_plugins: HashMap<PathBuf, updates::Previous>,
    disabled_plugins: BTreeSet<PathBuf>,
    plugin_language: String,
    language: &'static str,
    localizations: HashMap<String, crate::plugin_localization::Presentation>,
    application_mappings: applications::Mappings,
    application_monitor: Option<crate::application_monitor::Worker>,
    application_delivered: HashMap<String, applications::Delivered>,
    application_error: Option<String>,
    application_artwork: crate::application_artwork::Artwork,
    pending_links: VecDeque<links::Pending>,
    marketplace: crate::marketplace::State,
    registry_routes: HashMap<crate::device_registry::Key, String>,
    registry_commands: Option<std::sync::mpsc::Sender<crate::device_registry::Command>>,
    forgetting_devices: HashSet<crate::device_registry::Key>,
    canonical_graph_update: bool,
    scope: String,
    selected_scope: String,
    other_devices: BTreeMap<String, device_pool::DeviceRuntime>,
    device_name: Option<String>,
    transport_generation: Option<u64>,
    pending_hardware: Option<std::sync::mpsc::Sender<Draw>>,
    model: Model,
    mixer_config: crate::mixer::Config,
    mixer: Option<crate::mixer::Worker>,
    mixer_state: crate::mixer::Snapshot,
    mixer_revision: u64,
    mixer_save_due: Option<std::time::Instant>,
    mixer_save_error: Option<String>,
    reference_app: Option<PathBuf>,
    screensavers: HashMap<String, screensaver::Preferences>,
    saver: screensaver::Runtime,
    session_monitor: Option<crate::session_activity::Worker>,
    session_state: Option<crate::session_activity::State>,
    resume_monitor: Option<crate::system_resume::Worker>,
    resume_available: bool,
    resume_error: Option<String>,
    brightness_repeats: HashMap<String, (u8, Duration)>,
    open_app_holds: HashMap<String, (Duration, crate::system_app::Config)>,
    timers: timer::Runtime,
    device_controls: HashMap<String, device_session::Controls>,
    controls_observed: device_session::Controls,
    controls_error: Option<String>,
    connected: bool,
    connection_error: Option<String>,
    device_config: Option<device::Config>,
    redraw: Option<Box<dyn Fn() + Send + Sync>>,
    badges: HashMap<String, feedback::Badge>,
    trigger_descriptions: HashMap<String, Value>,
    triggers: trigger::Runtime,
    wheels: wheel::Runtime,
    key_image_cache: key_images::Cache,
    key_images: key_images::Runtime,
    folder_exit: Option<folder_exit::Pending>,
    folder_back_holds: HashMap<String, folder_exit::BackPress>,
    virtual_suppressed: HashSet<String>,
    volume: Option<crate::volume::Worker>,
    volume_state: Option<crate::volume::State>,
    volume_error: Option<String>,
    stack_gestures: HashMap<String, crate::dial_stack::Gesture>,
    stack_cooldown: HashMap<String, std::time::Instant>,
    multi_owned: HashSet<String>,
    multi_held: HashMap<String, crate::multi::Entry>,
    multi_pending: HashMap<String, Vec<crate::multi::Entry>>,
    multi_preparing: bool,
    multi: crate::multi::Scheduler,
    multi_clock: std::time::Instant,
    multi_notify: Arc<tokio::sync::Notify>,
    audio: Option<crate::audio_output::Worker>,
    audio_outputs: Vec<(String, String)>,
    audio_active: HashMap<String, usize>,
    system_actions: Option<crate::system_actions::Worker>,
    system_status: String,
    library: Library,
    clipboard: Option<transfer::Clipboard>,
    profile_assets: HashMap<PathBuf, profile::ProfileBundle>,
    controller_backgrounds: controller_background::Runtime,
    diagnostics: bool,
    running: bool,
    inspectors: HashMap<String, Inspector>,
    port: u16,
    plugins: Vec<Plugin>,
    actions: Vec<Action>,
    slots: Vec<Slot>,
    globals: HashMap<String, Value>,
    renderer: Renderer,
    hardware: Option<std::sync::mpsc::Sender<Draw>>,
    device: String,
    firmware: String,
    token: String,
    state_path: PathBuf,
    logs: Vec<String>,
    simulate: bool,
    dial_pressed: Vec<bool>,
    encoder_routing: encoder_routes::Runtime,
    sensor_pressed: Vec<bool>,
    held: HashMap<HeldControl, String>,
    released: HashSet<HeldControl>,
    retired: VecDeque<(String, String)>,
}

fn action_payload(slot: &Slot) -> Value {
    let mut payload = json!({"controller":slot.controller,"settings":slot.settings,"state":slot.state,"isInMultiAction":slot.in_multi_action,"resources":slot.resources.clone().unwrap_or_default()});
    if !slot.in_multi_action {
        let (column, row) = slot.coordinates();
        payload["coordinates"] = json!({"column":column,"row":row});
    }
    payload
}
impl Host {
    fn import_profile(&mut self, path: &Path) -> Result<()> {
        let bundle = profile::load_bundle(path)?;
        let binding = smart_profiles::archive::read(&bundle)?;
        self.sync_all_devices();
        let (profile, current) = self.prepare_bundle(bundle, None, false)?;
        let source = profile.source.clone();
        let id = profile.id.clone();
        let compatible = profile.model.profile_compatible(self.model);
        let routing = binding.map(|binding| {
            let mut config = self.smart_config();
            config.bindings.insert(id.clone(), binding);
            config
        });
        let mut next = self.library.clone();
        next.profiles.push(profile);
        next.cursors.insert(id.clone(), current.clone());
        if compatible {
            next.previous = Some((next.active_profile.clone(), next.active_page.clone()));
            next.active_profile = id.clone();
            next.active_page = current;
            next.folders.clear();
        }
        let result = self.commit_profile_change(next, routing);
        if result.is_err() && !self.library.profiles.iter().any(|p| p.id == id) {
            // Only remove an uncommitted owned source; a refresh error after save
            // must retain the archive referenced by the durable profile graph.
            if let Some(source) = source {
                let _ = fs::remove_file(source);
            }
        }
        result?;
        if !compatible {
            self.log(
                "Imported profile retained for its own device model; active profile unchanged",
            );
        }
        Ok(())
    }
    fn log(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("{message}");
        self.logs.push(message);
        if self.logs.len() > 100 {
            self.logs.remove(0);
        }
    }
    fn persist(&mut self) -> Result<()> {
        self.persist_without_plugin(None)
    }
    fn persist_without_plugin(&mut self, excluded: Option<usize>) -> Result<()> {
        if !self.canonical_graph_update {
            self.sync_all_devices();
        }
        let pending = self.state_path.with_extension("tmp");
        #[derive(Serialize)]
        struct View<'a> {
            smart_profiles: &'a smart_profiles::Configs,
            plugin_asset_ids: &'a HashMap<PathBuf, (u64, u64)>,
            devices: Vec<device_pool::Record>,
            active_scope: &'a str,
            selected_scope: &'a str,
            model: Model,
            mixer: &'a crate::mixer::Config,
            reference_app: &'a Option<PathBuf>,
            screensavers: &'a HashMap<String, screensaver::Preferences>,
            device_controls: &'a HashMap<String, device_session::Controls>,
            device_config: &'a Option<device::Config>,
            library: &'a Library,
            plugins: Vec<&'a Path>,
            slots: &'a [Slot],
            globals: &'a HashMap<String, Value>,
            application_mappings: &'a applications::Mappings,
            plugin_language: &'a str,
            suppress_multi_feedback: bool,
            disabled_plugins: Vec<&'a Path>,
        }
        let data = serde_json::to_vec_pretty(&View {
            smart_profiles: &self.smart_profiles,
            plugin_asset_ids: &self.plugin_asset_ids,
            devices: self.device_records(),
            active_scope: &self.scope,
            selected_scope: &self.selected_scope,
            model: self.model,
            mixer: &self.mixer_config,
            reference_app: &self.reference_app,
            screensavers: &self.screensavers,
            device_controls: &self.device_controls,
            device_config: &self.device_config,
            library: &self.library,
            plugins: self
                .plugins
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != excluded)
                .map(|(_, p)| p.root.as_path())
                .collect(),
            slots: &self.slots,
            globals: &self.globals,
            application_mappings: &self.application_mappings,
            plugin_language: &self.plugin_language,
            suppress_multi_feedback: self.suppress_multi_feedback,
            disabled_plugins: self
                .disabled_plugins
                .iter()
                .filter(|root| excluded.is_none_or(|index| self.plugins[index].root != **root))
                .map(PathBuf::as_path)
                .collect(),
        })?;
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&pending)?;
            file.write_all(&data)?;
            file.sync_all()?;
        }
        fs::rename(pending, &self.state_path)?;
        self.mixer_save_due = None;
        self.mixer_save_error = None;
        Ok(())
    }
    fn emit(&self, slot: &Slot, event: &str, extra: Value) {
        let action = self.actions.iter().find(|a| a.uuid == slot.uuid);
        if slot.in_multi_action && action.is_some_and(|a| a.raw["SupportedInMultiActions"] == false)
        {
            return;
        }
        let plugin = action.and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin));
        if let Some(sender) = plugin.and_then(|p| p.sender.as_ref()) {
            let mut payload = action_payload(slot);
            if let Some(map) = extra.as_object() {
                for (k, v) in map {
                    payload[k] = v.clone();
                }
            }
            if event == "titleParametersDidChange" {
                payload.as_object_mut().unwrap().remove("isInMultiAction");
            }
            let _=sender.send(json!({"event":event,"action":slot.uuid,"context":slot.context,"device":self.device,"payload":payload}));
        }
    }
    fn advance_key_state(&mut self, slot: &Slot) -> Result<()> {
        if slot.in_multi_action || slot.controller != "Keypad" {
            return Ok(());
        }
        let Some(action) = self.actions.iter().find(|a| a.uuid == slot.uuid) else {
            return Ok(());
        };
        if action.raw["DisableAutomaticStates"] == true
            || (slot.uuid != crate::system_actions::HOTKEY_SWITCH
                && !self
                    .plugins
                    .iter()
                    .any(|p| p.id == action.plugin && p.sender.is_some()))
        {
            return Ok(());
        }
        let count = action.raw["States"]
            .as_array()
            .map_or(1, Vec::len)
            .max(slot.states.as_array().map_or(1, Vec::len));
        if count <= 1 {
            return Ok(());
        }
        // Use the current instance, preserving settings/title changes made while held.
        let Some(mut current) = self.visible_slot(&slot.context).cloned() else {
            return Ok(());
        };
        current.state = (current.state + 1) % count as u64;
        current.background = self.resolve_state_image(&current, current.state);
        current.title = appearance::title(&current, &self.actions);
        current.title_changed = false;
        current.image_changed = false;
        self.store_slot(current);
        self.render()?;
        self.persist()
    }
    fn render(&mut self) -> Result<()> {
        self.update_timer_feedback();
        self.sync_key_images();
        self.wheels.sync(&self.slots);
        self.update_mixer_feedback();
        self.update_volume_feedback();
        self.update_brightness_feedback();
        // Selected container children need the same imported/plugin icon lookup
        // as direct assignments. Keep inactive child caches without drawing them.
        let missing: Vec<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| s.background.is_none())
            .cloned()
            .collect();
        for slot in missing {
            if let Some(image) = self.resolve_state_image(&slot, slot.state)
                && let Some(visible) = self
                    .slots
                    .iter_mut()
                    .find_map(|s| s.find_mut(&slot.context))
            {
                visible.background = Some(image);
            }
        }
        self.sync_encoder_assets();
        let candidates = self
            .slots
            .iter()
            .map(|slot| {
                (display_targets::has_hardware(slot) || self.wheels.requires_hardware_plane(slot))
                    .then(|| slot.clone())
            })
            .collect::<Vec<_>>();
        let mut projections = candidates
            .into_iter()
            .map(|slot| slot.map(|slot| self.hardware_projection(slot)))
            .collect::<Vec<_>>();
        let progress = if self.suppress_multi_feedback {
            BTreeMap::new()
        } else {
            self.multi.progress()
        };
        let wheel_pressed: HashSet<_> = self
            .slots
            .iter()
            .flat_map(Slot::visible_flatten)
            .filter(|s| {
                s.uuid == crate::action_wheel::UUID
                    && (self.key_pulse_active(&s.context)
                        || self.held.get(&HeldControl::Dial(s.index)) == Some(&s.context))
            })
            .map(|s| s.context.clone())
            .collect();
        let folder_progress = self.folder_exit_percent().map(|(id, p)| (id.to_owned(), p));
        let encoder_marker_layer = self.encoder_marker_layer()?;
        let encoder_background = self.encoder_background();
        let display = display_targets::Display {
            encoder_background: encoder_background.as_ref(),
            hardware_plane: false,
            folder_progress: folder_progress.as_ref(),
            key_images: &self.key_images,
            application_artwork: &self.application_artwork,
            wheels: &self.wheels,
            wheel_pressed: &wheel_pressed,
            progress: &progress,
            model: self.model,
            renderer: &self.renderer,
            actions: &self.actions,
            plugins: &self.plugins,
            badges: &self.badges,
            now: self.multi_clock.elapsed(),
        };
        let mut strip = encoder_background.clone().unwrap_or_else(|| {
            DynamicImage::ImageRgb8(RgbImage::from_pixel(
                self.model.strip_width().max(1),
                self.model.strip_height().max(1),
                Rgb([14, 21, 29]),
            ))
        });
        let mut warnings = Vec::new();
        for (slot, projection) in self.slots.iter_mut().zip(&mut projections) {
            appearance::apply_user_titles(slot, &self.actions);
            display.editor_children(slot, &mut warnings)?;
            if slot.controller == "Encoder" {
                slot.software_icon = display.encoder_icon(slot)?;
            }
            if slot.controller == "Encoder" && !self.model.capabilities().strip {
                // Studio has editor icons but no physical encoder LCD.
                slot.preview.clear();
                warnings.append(&mut slot.display_warnings);
                continue;
            }
            let software = display.picture(slot, &mut warnings)?;
            slot.preview = display_targets::png(&software)?;
            let physical = if let Some(projected) = projection {
                appearance::apply_user_titles(projected, &self.actions);
                display_targets::Display {
                    hardware_plane: true,
                    ..display
                }
                .picture(projected, &mut Vec::new())?
            } else {
                software
            };
            if slot.controller == "Encoder" && self.model.capabilities().strip {
                let (x, y) = self.model.encoder_origin(slot.index);
                strip.copy_from(&physical, x, y)?;
            } else if let Some(hardware) = &self.hardware {
                if slot.controller == "Neo" {
                    let _ = hardware.send(Draw::Infobar(physical));
                } else if self.model.capabilities().key_images {
                    let _ = hardware.send(Draw::Key(slot.index, physical));
                }
            }
        }
        if let Some(layer) = encoder_marker_layer {
            image::imageops::overlay(&mut strip, &layer, 0, 0);
        }
        for warning in warnings {
            self.log(warning);
        }
        if let Some(hardware) = &self.hardware {
            if self.model == Model::Neo {
                let navigation = self.page_navigation();
                for (index, enabled) in [navigation.previous, navigation.next]
                    .into_iter()
                    .enumerate()
                {
                    // Solid sensor light: active accent or off at the page boundary.
                    // This is Deckforge's visual policy, not a copied vendor asset.
                    let color = if enabled {
                        Rgb([141, 216, 188])
                    } else {
                        Rgb([0, 0, 0])
                    };
                    let _ = hardware.send(Draw::Sensor(
                        index as u8,
                        DynamicImage::ImageRgb8(RgbImage::from_pixel(96, 16, color)),
                    ));
                }
            }
            for index in 0..self.model.key_count() {
                if !self
                    .slots
                    .iter()
                    .any(|s| s.controller == "Keypad" && s.index == index)
                    && self.model.capabilities().key_images
                {
                    let image = self.renderer.key("", None).resize_exact(
                        self.model.key_width(),
                        self.model.key_height(),
                        image::imageops::FilterType::Lanczos3,
                    );
                    let _ = hardware.send(Draw::Key(index, image));
                }
            }
            if self.model.capabilities().strip && self.model.dial_count() > 0 {
                let _ = hardware.send(Draw::Strip(strip));
            }
            if self.model == Model::Neo && !self.slots.iter().any(|s| s.controller == "Neo") {
                let _ = hardware.send(Draw::Infobar(DynamicImage::ImageRgb8(
                    RgbImage::from_pixel(232, 50, Rgb([0, 0, 0])),
                )));
            }
        }
        if self.simulate {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::hash::DefaultHasher::new();
            for slot in &self.slots {
                slot.preview.hash(&mut hasher);
            }
            let signature = hasher.finish();
            if self.saver.normal_signature != Some(signature) && !self.saver.active {
                self.saver.activity = self.multi_clock.elapsed();
                self.multi_notify.notify_one();
            }
            self.saver.normal_signature = Some(signature);
        }
        self.render_saver()?;
        if let Some(redraw) = &self.redraw {
            redraw();
        }
        Ok(())
    }
    fn assign(&mut self, controller: &str, index: u8, uuid: &str) -> Result<()> {
        if uuid == multiaction::DELAY {
            bail!("Add Delay inside a Multi Action");
        }
        if !device::valid_slot(self.model, controller, index) {
            bail!("Invalid control for {}", self.model.name());
        }
        let action = self
            .actions
            .iter()
            .find(|a| a.uuid == uuid && a.controllers.iter().any(|c| c == controller))
            .cloned()
            .context("Action is not installed or does not support this controller")?;
        anyhow::ensure!(
            !self.action_plugin_disabled(&action.uuid),
            "Enable this plugin before assigning its actions"
        );
        if (controller == "Encoder" && self.model.capabilities().strip) || controller == "Neo" {
            let root = self
                .plugins
                .iter()
                .find(|p| p.id == action.plugin)
                .map(|p| p.root.as_path());
            display::load_layout(
                root,
                if controller == "Neo" {
                    "$Neo"
                } else {
                    action.raw["Encoder"]["layout"].as_str().unwrap_or("$B1")
                },
            )?;
        }
        self.editable()?;
        self.sync_visible();
        let title = action.raw["States"][0]["Title"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let mut slot = Slot {
            model: self.model,
            children: vec![],
            child_id: String::new(),
            resources: None,
            asset_source: None,
            appearances: Default::default(),
            in_multi_action: false,
            event_coordinates: None,
            encoder_overrides: HashMap::new(),
            context: uuid::Uuid::new_v4().to_string(),
            controller: controller.into(),
            index,
            uuid: uuid.into(),
            name: action.name.clone(),
            settings: if controller == "Encoder" && uuid == crate::system_actions::MULTIMEDIA {
                json!({"actionIdx":18})
            } else {
                if uuid == "com.elgato.streamdeck.page.goto" {
                    json!({"PageIndex":1})
                } else if uuid == crate::action_wheel::UUID {
                    wheel::defaults()
                } else if uuid == crate::dial_stack::UUID {
                    stack::defaults()
                } else {
                    mixer_actions::defaults(uuid)
                        .or_else(|| crate::audio_output::defaults(uuid))
                        .unwrap_or_else(|| crate::system_actions::defaults(uuid))
                }
            },
            title,
            state: 0,
            feedback: json!({}),
            layout: if controller == "Neo" {
                "$Neo".into()
            } else {
                action.raw["Encoder"]["layout"]
                    .as_str()
                    .unwrap_or("$B1")
                    .into()
            },
            raw: json!({}),
            image_changed: false,
            title_changed: false,
            pinned: false,
            states: action.raw["States"].clone(),
            background: None,
            layout_cache: None,
            image_cache: HashMap::new(),
            display_warnings: Vec::new(),
            preview: String::new(),
            software_icon: String::new(),
        };
        if let Some(plugin) = self.plugins.iter().find(|p| p.id == action.plugin) {
            slot.background = state_image(&plugin.root, &slot.states, 0);
        }
        if multiaction::container(uuid) {
            slot.raw = json!({"Actions":[{"Actions":[]},{"Actions":[]}]});
        }
        if uuid == crate::dial_stack::UUID || uuid == crate::action_wheel::UUID {
            slot.raw = json!({"Actions":[],"Settings":slot.settings});
        }
        if matches!(uuid, crate::action_trigger::UUID | crate::key_logic::UUID) {
            slot.raw = json!({"Actions":[{"Actions":[]},{"Actions":[]},{"Actions":[]}]});
        }
        if uuid == multiaction::DELAY {
            slot.settings = json!({"delay":1000});
        }
        self.library.place(slot)?;
        self.refresh_page()
    }
    fn remember_removed(&mut self, context: &str, action: &str) {
        self.trigger_descriptions.remove(context);
        self.badges.remove(context);
        if let Some(owner) = self
            .actions
            .iter()
            .find(|a| a.uuid == action)
            .map(|a| a.plugin.clone())
        {
            self.retired.push_back((context.into(), owner));
            while self.retired.len() > 256 {
                self.retired.pop_front();
            }
        }
    }
    fn cancel_departing_inputs(&mut self, visible: &HashSet<String>) {
        self.encoder_routing.retain(visible);
        self.open_app_holds
            .retain(|context, _| visible.contains(context));
        let departing_dials: HashSet<_> = self
            .slots
            .iter()
            .flat_map(Slot::flatten)
            .filter(|slot| {
                slot.uuid == crate::system_actions::HOTKEY && !visible.contains(&slot.context)
            })
            .map(|slot| slot.context.clone())
            .collect();
        if !departing_dials.is_empty()
            && let Some(worker) = &self.system_actions
            && let Err(error) = worker.submit(
                "",
                crate::system_actions::Plan::CancelDials(departing_dials),
            )
        {
            self.log(format!("Dial shortcut cancellation: {error}"));
        }
        let departing: Vec<_> = self
            .held
            .iter()
            .filter(|(_, context)| !visible.contains(*context))
            .map(|(control, context)| (*control, context.clone()))
            .collect();
        for (control, context) in departing {
            self.brightness_repeats.remove(&context);
            self.folder_back_holds.remove(&context);
            if let Some(slot) = self.slots.iter().find(|slot| slot.context == context)
                && matches!(
                    slot.uuid.as_str(),
                    crate::system_actions::HOTKEY | crate::system_actions::HOTKEY_SWITCH
                )
                && let Some(worker) = &self.system_actions
                && let Err(error) = worker.submit(&context, crate::system_actions::Plan::Release)
            {
                self.log(format!("Shortcut release: {error}"));
            }
            if let Some(slot) = self.slots.iter().find(|slot| slot.context == context) {
                self.emit(
                    slot,
                    if matches!(control, HeldControl::Key(_)) {
                        "keyUp"
                    } else {
                        "dialUp"
                    },
                    json!({}),
                );
            }
            self.held.remove(&control);
            self.released.insert(control);
        }
    }
    fn button_input(&mut self, control: HeldControl, pressed: bool) -> Result<()> {
        let (controller, index, down, up) = match control {
            HeldControl::Key(index) if index < self.model.key_count() => {
                ("Keypad", index, "keyDown", "keyUp")
            }
            HeldControl::Dial(index) if index < self.model.dial_count() => {
                self.dial_pressed[index as usize] = pressed;
                ("Encoder", index, "dialDown", "dialUp")
            }
            _ => bail!("Input outside device geometry"),
        };
        if !pressed && self.released.remove(&control) {
            return Ok(());
        }
        let owned_release = !pressed && self.held.contains_key(&control);
        let slot = if pressed {
            self.released.remove(&control);
            let slot = self
                .slots
                .iter()
                .find(|s| s.controller == controller && s.index == index)
                .cloned();
            if let Some(slot) = &slot {
                self.held.insert(control, slot.context.clone());
            }
            slot
        } else if let Some(context) = self.held.remove(&control) {
            self.slots
                .iter()
                .find(|s| s.context == context)
                .or_else(|| self.library.slot(&context))
                .cloned()
        } else {
            self.slots
                .iter()
                .find(|s| s.controller == controller && s.index == index)
                .cloned()
        };
        let Some(slot) = slot else {
            return Ok(());
        };
        if matches!(control, HeldControl::Key(_)) {
            match slot.uuid.as_str() {
                library::OPEN_FOLDER => {
                    if owned_release {
                        self.select_page(
                            slot.settings["ProfileUUID"]
                                .as_str()
                                .context("Folder target missing")?,
                            true,
                        )?;
                    }
                    return Ok(());
                }
                library::BACK_FOLDER => {
                    return self.folder_back_button(&slot.context, pressed, owned_release);
                }
                "com.elgato.streamdeck.page.next" => {
                    if owned_release {
                        self.step_page(1)?;
                    }
                    return Ok(());
                }
                "com.elgato.streamdeck.page.previous" => {
                    if owned_release {
                        self.step_page(-1)?;
                    }
                    return Ok(());
                }
                "com.elgato.streamdeck.page.goto" => {
                    if owned_release {
                        self.goto_page(&slot.settings)?;
                    }
                    return Ok(());
                }
                "com.elgato.streamdeck.page.indicator" => return Ok(()),
                _ => {}
            }
        }
        if slot.uuid == crate::dial_stack::UUID {
            if pressed || owned_release {
                self.stack_button(&slot, pressed)?;
            }
            return Ok(());
        }
        if slot.uuid == crate::action_wheel::UUID {
            if pressed || owned_release {
                self.wheel_button(&slot, pressed)?;
            }
            return Ok(());
        }
        if slot.uuid == crate::key_logic::UUID {
            if pressed || owned_release {
                self.logic_button(&slot, pressed)?;
            }
            return Ok(());
        }
        if slot.uuid == crate::action_trigger::UUID {
            if pressed || owned_release {
                self.trigger_button(&slot, pressed)?;
            }
            return Ok(());
        }
        if multiaction::container(&slot.uuid) {
            if pressed && self.multi_running(&slot.context) {
                self.cancel_multi(&slot.context);
                self.held.remove(&control);
                self.released.insert(control);
                self.system_status = "Multi Action cancelled".into();
            } else if owned_release {
                self.start_multi(&slot)?;
            }
            return Ok(());
        }
        if crate::audio_output::supported(&slot.uuid) {
            if owned_release {
                self.soundboard_action(&slot)?;
            }
            return Ok(());
        }
        if mixer_actions::supported(&slot.uuid) || crate::system_actions::supported(&slot.uuid) {
            if pressed || owned_release {
                self.run_system_action(
                    &slot,
                    if pressed {
                        crate::system_actions::Trigger::Down
                    } else {
                        crate::system_actions::Trigger::Up
                    },
                )?;
            }
            return Ok(());
        }
        self.emit(&slot, if pressed { down } else { up }, json!({}));
        if owned_release {
            self.advance_key_state(&slot)?;
        }
        Ok(())
    }
    fn run_system_action(
        &mut self,
        slot: &Slot,
        trigger: crate::system_actions::Trigger,
    ) -> Result<()> {
        if mixer_actions::supported(&slot.uuid) {
            return self.mixer_input(slot, trigger);
        }
        if slot.uuid == screensaver::UUID {
            if matches!(trigger, crate::system_actions::Trigger::Up) && slot.controller == "Keypad"
            {
                if !self.connected {
                    bail!("Sleep target device is not connected");
                }
                let target = match slot.settings.get("DeviceUUID") {
                    None | Some(Value::Null) => "",
                    Some(value) => value
                        .as_str()
                        .context("Sleep target must be a device identifier")?,
                };
                if !target.is_empty() && target != self.device {
                    bail!("Sleep target device is not connected");
                }
                self.sleep_device("action")?;
            }
            return Ok(());
        }
        if slot.uuid == crate::brightness::UUID {
            return self.brightness_input(slot, trigger);
        }
        if slot.uuid == timer::UUID {
            return self.timer_input(slot, trigger);
        }
        if volume::is_volume(slot) {
            return self.volume_input(slot, trigger);
        }
        if slot.uuid == crate::system_app::UUID {
            return self.open_app_input(slot, trigger);
        }
        if slot.uuid == crate::system_actions::MOUSE_WHEEL {
            if let crate::system_actions::Trigger::Rotate(ticks) = trigger
                && ticks != 0
            {
                let horizontal = self
                    .dial_pressed
                    .get(slot.index as usize)
                    .copied()
                    .unwrap_or(false);
                self.system_actions
                    .get_or_insert_with(crate::system_actions::Worker::new)
                    .submit(
                        &slot.context,
                        crate::system_actions::Plan::Scroll {
                            ticks: i32::from(ticks),
                            horizontal,
                        },
                    )?;
            }
            return Ok(());
        }
        let Some(plan) = crate::system_actions::input_plan_state(
            &slot.uuid,
            &slot.controller,
            &slot.settings,
            trigger,
            slot.state,
        )?
        else {
            return Ok(());
        };
        let worker = self
            .system_actions
            .get_or_insert_with(crate::system_actions::Worker::new);
        worker.submit(&slot.context, plan)?;
        self.system_status = format!("{} queued", slot.name);
        if slot.uuid == crate::system_actions::HOTKEY_SWITCH
            && matches!(trigger, crate::system_actions::Trigger::Up)
        {
            self.advance_key_state(slot)?;
        }
        Ok(())
    }
    fn poll_system_actions(&mut self) {
        let reports = self
            .system_actions
            .as_ref()
            .map(|worker| worker.drain())
            .unwrap_or_default();
        for report in reports {
            let owner = self.context_device(&report.context);
            if report.prepared_keyboard && owner.is_none() {
                continue;
            }
            let device = owner.unwrap_or_else(|| self.device.clone());
            let _ = self.with_device(&device, |host| {
                host.system_action_report(report);
                Ok(())
            });
        }
    }
    fn system_action_report(&mut self, report: crate::system_actions::Report) {
        if report.prepared_keyboard {
            self.multi_keyboard_prepared(report.result.is_ok());
            self.trigger_keyboard_prepared(report.result.is_ok());
        }
        let name = self
            .slots
            .iter()
            .find(|s| s.context == report.context)
            .or_else(|| self.library.slot(&report.context))
            .map(|s| s.name.as_str())
            .unwrap_or("System action");
        self.system_status = match report.result {
            Ok(message) => format!("{name}: {message}"),
            Err(error) => format!("{name} failed: {error:#}"),
        };
        self.log(self.system_status.clone());
    }
    fn input(&mut self, event: InputEvent) -> Result<()> {
        if self.saver_input(&event)? {
            return Ok(());
        }
        let had_countdown = self.folder_exit.is_some();
        let pending = self.folder_exit_input(&event);
        let result = self.dispatch_input(event);
        if result.is_ok() {
            self.arm_folder_exit(pending);
        }
        if had_countdown || self.folder_exit.is_some() {
            self.render()?;
        }
        result
    }
    fn dispatch_input(&mut self, event: InputEvent) -> Result<()> {
        self.log(format!("{event:?}"));
        let (index, name, extra) = match event {
            InputEvent::Sensor { index, pressed } => {
                anyhow::ensure!(
                    self.model == Model::Neo && index < 2,
                    "Invalid sensor input"
                );
                let previous = self.sensor_pressed[index as usize];
                self.sensor_pressed[index as usize] = pressed;
                let navigation = self.page_navigation();
                let enabled = if index == 0 {
                    navigation.previous
                } else {
                    navigation.next
                };
                if pressed && !previous && enabled {
                    self.step_page(if index == 0 { -1 } else { 1 })?;
                }
                return Ok(());
            }
            InputEvent::Key { index, pressed } => {
                return self.button_input(HeldControl::Key(index), pressed);
            }
            InputEvent::DialPress { index, pressed } => {
                return self.button_input(HeldControl::Dial(index), pressed);
            }
            InputEvent::DialRotate { index, ticks } => {
                let pressed = *self
                    .dial_pressed
                    .get(index as usize)
                    .context("Dial index out of range")?;
                (
                    index,
                    "dialRotate",
                    json!({"ticks":ticks,"pressed":pressed}),
                )
            }
            InputEvent::Touch { x, y, long } => {
                anyhow::ensure!(
                    self.model.dial_count() > 0
                        && u32::from(x) < self.model.strip_width()
                        && u32::from(y) < self.model.strip_height(),
                    "Touch outside this device's strip"
                );
                let width = (self.model.strip_width() / u32::from(self.model.dial_count())) as u16;
                (
                    u8::try_from(x / width)?,
                    "touchTap",
                    json!({"hold":long,"tapPos":[x%width,y]}),
                )
            }
            InputEvent::Swipe { start, end } => {
                anyhow::ensure!(
                    self.model.capabilities().touchscreen
                        && [start, end]
                            .iter()
                            .all(|(x, y)| u32::from(*x) < self.model.strip_width()
                                && u32::from(*y) < self.model.strip_height()),
                    "Swipe outside this device's touchscreen"
                );
                if start.0.abs_diff(end.0) > start.1.abs_diff(end.1) {
                    self.step_page(if end.0 < start.0 { 1 } else { -1 })?;
                }
                return Ok(());
            }
        };
        if let Some(slot) = self
            .slots
            .iter()
            .find(|s| s.controller == "Encoder" && s.index == index)
            .cloned()
        {
            if slot.uuid == crate::dial_stack::UUID {
                self.stack_input(&slot, name, extra)?;
            } else if slot.uuid == crate::action_wheel::UUID {
                self.wheel_input(&slot, name, extra)?;
            } else if slot.uuid == crate::action_trigger::UUID {
                self.trigger_input(&slot, name, extra)?;
            } else if mixer_actions::supported(&slot.uuid)
                || crate::system_actions::supported(&slot.uuid)
            {
                let trigger = if name == "dialRotate" {
                    crate::system_actions::Trigger::Rotate(
                        extra["ticks"].as_i64().unwrap_or(0) as i8
                    )
                } else {
                    crate::system_actions::Trigger::Tap
                };
                self.run_system_action(&slot, trigger)?;
            } else {
                self.emit(&slot, name, extra);
            }
        }
        Ok(())
    }
    fn plugin_message(&mut self, plugin_index: usize, message: Value) -> Result<()> {
        let context = message["context"].as_str().unwrap_or("");
        let target = if message["event"] == "switchToProfile" {
            Some(
                message["device"]
                    .as_str()
                    .context("Profile switching requires a target device")?
                    .to_owned(),
            )
        } else {
            self.context_device(context)
        };
        if let Some(target) = target {
            if message["event"] != "switchToProfile"
                && let Some(requested) = message.get("device").and_then(Value::as_str)
            {
                anyhow::ensure!(requested == target, "Action belongs to another device");
            }
            return self.with_device(&target, |host| {
                host.plugin_message_local(plugin_index, message)
            });
        }
        self.plugin_message_local(plugin_index, message)
    }
    fn plugin_message_local(&mut self, plugin_index: usize, message: Value) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        let event = message["event"].as_str().context("Missing event")?;
        let context = message["context"].as_str().unwrap_or("");
        let payload = message.get("payload").cloned().unwrap_or(json!({}));
        if event == "switchToProfile" {
            return self.plugin_switch_profile(plugin_index, &message);
        }
        let plugin = &self.plugins[plugin_index];
        if event == "openUrl" {
            return crate::external_url::open(payload["url"].as_str().context("Missing link URL")?);
        }
        if event == "logMessage" {
            self.log(format!(
                "{}: {}",
                plugin.id,
                payload["message"].as_str().unwrap_or("")
            ));
            return Ok(());
        }
        if event == "getGlobalSettings" {
            let mut reply = json!({"event":"didReceiveGlobalSettings","payload":{"settings":self.globals.get(&plugin.id).cloned().unwrap_or(json!({}))}});
            if let Some(id) = message.get("id") {
                reply["id"] = id.clone();
            }
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(reply);
            }
            return Ok(());
        }
        if event == "setGlobalSettings" {
            if !payload.is_object() {
                bail!("Global settings must be an object");
            }
            self.globals.insert(plugin.id.clone(), payload);
            self.persist()?;
            self.notify_global_settings(plugin_index, None, false);
            return Ok(());
        }
        let active = self.visible_slot(context).is_some();
        let Some(original) = self.slot_any(context).cloned() else {
            if self
                .retired
                .iter()
                .any(|(id, owner)| id == context && owner == &plugin.id)
            {
                return Ok(());
            }
            bail!("Unknown action context");
        };
        if !self
            .actions
            .iter()
            .any(|a| a.uuid == original.uuid && a.plugin == plugin.id)
        {
            bail!("Plugin does not own this context");
        }
        if matches!(event, "showOk" | "showAlert") {
            return self.show_feedback(
                &original,
                if event == "showOk" {
                    crate::feedback::Kind::Ok
                } else {
                    crate::feedback::Kind::Alert
                },
            );
        }
        if event == "setTriggerDescription" {
            return self.set_trigger_description(&original, payload);
        }
        if event == "getResources" {
            let reply = self.resources_event(&original, message.get("id"));
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(reply);
            }
            return Ok(());
        }
        if event == "setResources" {
            let slot = self.set_resources(&original, payload)?;
            self.notify_inspectors(context, self.resources_event(&slot, None), None);
            return Ok(());
        }
        if event == "getSettings" {
            let reply = self.settings_event(&original, message.get("id"));
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(reply);
            }
            return Ok(());
        }
        if event == "setSettings" {
            if !payload.is_object() {
                bail!("Settings must be an object");
            }
            let mut slot = original;
            slot.settings = payload;
            self.store_slot(slot.clone());
            self.persist()?;
            self.notify_inspectors(context, self.settings_event(&slot, None), None);
            return Ok(());
        }
        if event == "sendToPropertyInspector" {
            self.notify_inspectors(context,json!({"event":"sendToPropertyInspector","action":original.uuid,"context":original.context,"device":self.device,"payload":payload}),None);
            return Ok(());
        }
        let root = plugin.root.clone();
        // Validate a replacement before exposing it to any display or saved state.
        let mut replacement = original;
        let slot = &mut replacement;
        match event {
            "setTitle" | "setImage" => self.apply_plugin_display(slot, &root, event, &payload)?,
            "setState" => {
                let state = payload["state"].as_u64().context("Invalid state")?;
                anyhow::ensure!(
                    state < appearance::count(slot, &self.actions) as u64 && state < 1024,
                    "State out of range"
                );
                slot.state = state;
                slot.background = self.resolve_state_image(slot, state);
                slot.title = appearance::title(slot, &self.actions);
                slot.image_changed = false;
                slot.title_changed = false;
            }
            "setFeedback" => {
                let object = payload.as_object().context("Feedback must be an object")?;
                for (key, value) in object {
                    display::merge_feedback(&mut slot.feedback[key], value);
                    slot.image_cache.remove(key);
                }
            }
            "setFeedbackLayout" => {
                let layout = payload["layout"].as_str().context("Missing layout")?;
                let definition = if slot.controller == "Encoder" && !slot.model.capabilities().strip
                {
                    None
                } else {
                    Some(display::load_layout(Some(&root), layout)?)
                };
                slot.layout = layout.into();
                slot.layout_cache = definition;
                slot.image_cache.clear();
            }
            _ => {
                bail!("Unsupported plugin command: {event}");
            }
        }
        if replacement.controller == "Neo"
            || (replacement.controller == "Encoder" && replacement.model.capabilities().strip)
        {
            display::dial_image(&self.renderer, &mut replacement, Some(&root), &self.actions)?;
        }
        self.store_slot(replacement);
        if active {
            self.render()?;
        }
        self.persist()?;
        Ok(())
    }
}

fn safe_file(root: &Path, path: &str) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let file = root.join(path).canonicalize()?;
    if !file.starts_with(&root) || !file.is_file() {
        bail!("Plugin resource escapes its directory");
    }
    Ok(file)
}
fn image_bytes(root: &Path, source: &str) -> Result<Vec<u8>> {
    let data = if source.starts_with("data:image/") {
        let (header, content) = source.split_once(',').context("Invalid image URL")?;
        if content.len() > 12 * 1024 * 1024 {
            bail!("Encoded image exceeds 12 MiB");
        }
        if header.ends_with(";base64") {
            STANDARD.decode(content)?
        } else {
            percent_encoding::percent_decode_str(content).collect::<Vec<_>>()
        }
    } else {
        let path = safe_file(root, source)?;
        if fs::metadata(&path)?.len() > 8 * 1024 * 1024 {
            bail!("Image exceeds 8 MiB");
        }
        fs::read(path)?
    };
    if data.len() > 8 * 1024 * 1024 {
        bail!("Image exceeds 8 MiB");
    }
    Ok(data)
}
fn decode_image_bytes(data: &[u8], source: &str) -> Result<DynamicImage> {
    let svg = source.starts_with("data:image/svg+xml")
        || source.ends_with(".svg")
        || std::str::from_utf8(&data[..data.len().min(512)])
            .is_ok_and(|text| text.contains("<svg"));
    display::decode_image(data, svg)
}
fn load_image(root: &Path, source: &str) -> Result<DynamicImage> {
    decode_image_bytes(&image_bytes(root, source)?, source)
}

fn state_image(root: &Path, states: &Value, state: u64) -> Option<DynamicImage> {
    let image = states.get(state as usize)?.get("Image")?.as_str()?;
    display::IMAGE_SUFFIXES
        .iter()
        .find_map(|suffix| load_image(root, &format!("{image}{suffix}")).ok())
}

async fn root(
    State(state): State<Shared>,
    upgrade: std::result::Result<
        WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
) -> Response {
    if let Ok(upgrade) = upgrade {
        return upgrade
            .max_message_size(12 * 1024 * 1024)
            .on_upgrade(move |socket| socket_session(socket, state));
    }
    let host = state.lock().unwrap();
    if !host.diagnostics {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = host.token.clone();
    Html(include_str!("ui.html").replace("__TOKEN__", &token)).into_response()
}
fn slot_snapshot(host: &Host, s: &Slot) -> Value {
    json!({"appearance":appearance::snapshot(s,&host.actions),"state_count":appearance::count(s,&host.actions),"plugin_disabled":host.action_plugin_disabled(&s.uuid),"available":host.actions.iter().any(|a|a.uuid==s.uuid)&&!host.action_plugin_disabled(&s.uuid),"pinned":s.pinned,"pin_excluded_pages":library::pin_exclusions(s),"context":s.context,"controller":s.controller,"index":s.index,"uuid":s.uuid,"name":host.presented_slot_name(s),"state_names":host.state_names(s),"settings":s.settings,"resources":s.resources.clone().unwrap_or_default(),"title":s.title,"state":s.state,"feedback":s.feedback,"layout":s.layout,"preview":s.preview,"software_icon":s.software_icon,"raw":s.raw,
        "trigger_description":host.trigger_description(s),
        "application_running":host.application_artwork.get(&s.context).filter(|a|a.settings==s.settings && s.uuid==crate::system_app::UUID).and_then(|a|a.running),
        "command_run":if s.uuid == crate::system_command::UUID {host.system_actions.as_ref().and_then(|worker|worker.command_results.summary(&s.context))} else {None},
        "trigger_children":if matches!(s.uuid.as_str(),crate::action_trigger::UUID|crate::key_logic::UUID) {s.children.iter().map(|g|g.first().map(|c|slot_snapshot(host,c)).unwrap_or(Value::Null)).collect::<Vec<_>>()}else{vec![]},
        "stack_children":if s.uuid==crate::dial_stack::UUID {s.children.first().map(|children|children.iter().map(|c|slot_snapshot(host,c)).collect::<Vec<_>>()).unwrap_or_default()}else{vec![]},
        "multi_running":host.multi_running(&s.context),"multi_children":s.children.iter().map(|g|g.iter().map(|c|slot_snapshot(host,c)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "has_inspector":host.actions.iter().find(|a|a.uuid==s.uuid).is_some_and(|a|a.raw["PropertyInspectorPath"].is_string()||host.plugins.iter().find(|p|p.id==a.plugin).is_some_and(|p|p.manifest["PropertyInspectorPath"].is_string()))})
}
fn action_snapshot(host: &Host, action: &Action) -> Value {
    let plugin = host.plugins.iter().find(|p| p.id == action.plugin);
    let presentation = host.presented_action(action);
    let plugin_name = plugin
        .map(|p| {
            host.presented_manifest(p)["Name"]
                .as_str()
                .unwrap_or(&p.name)
        })
        .unwrap_or("Deckforge");
    let category = if mixer_actions::supported(&action.uuid) {
        "Audio mixer"
    } else if matches!(
        action.uuid.as_str(),
        crate::system_actions::TIMER | crate::system_actions::SLEEP | crate::brightness::UUID
    ) {
        "Stream Deck"
    } else if crate::system_actions::supported(&action.uuid) {
        "System"
    } else if action.uuid.starts_with("com.elgato.streamdeck.soundboard") {
        "Soundboard"
    } else {
        "Stream Deck"
    };
    let defaults = if action.uuid == "com.elgato.streamdeck.page.goto" {
        json!({"PageIndex":1})
    } else if action.uuid == crate::action_wheel::UUID {
        wheel::defaults()
    } else if action.uuid == multiaction::DELAY {
        json!({"delay":1000})
    } else {
        mixer_actions::defaults(&action.uuid)
            .or_else(|| crate::audio_output::defaults(&action.uuid))
            .unwrap_or_else(|| crate::system_actions::defaults(&action.uuid))
    };
    json!({
        "uuid":action.uuid,"name":presentation["Name"].as_str().unwrap_or(&action.name),"plugin":action.plugin,
        "source_name":action.name,"source_states":action.raw["States"],"enabled":!host.action_plugin_disabled(&action.uuid),
        "plugin_name":plugin_name,
        "category":plugin.map(|p|p.manifest["Category"].as_str().unwrap_or(plugin_name)).unwrap_or(category),
        "icon":action.raw["Icon"],"plugin_root":plugin.map(|p|p.root.as_path()),
        "icon_generation":plugin.map(|p|p.asset_generation),
        "tooltip":presentation["Tooltip"],"user_title_enabled":action.raw["UserTitleEnabled"]!=false,
        "visible":action.raw["VisibleInActionsList"]!=false,"controllers":action.controllers,
        "states":presentation["States"],
        "supported_in_multi":action.raw["SupportedInMultiActions"]!=false && !multiaction::container(&action.uuid),
        "supported_in_key_logic":action.raw["SupportedInKeyLogicActions"]!=false,
        "multi_only":action.uuid==multiaction::DELAY,"defaults":defaults
    })
}
fn snapshot(host: &Host) -> Value {
    let mut value = json!({"devices":host.devices_snapshot(),"page_navigation":host.page_navigation(),"device_model":host.model,"device_name":host.device_display_name(),"device_model_name":host.model.name(),"device_custom_name":host.device_name,"overflow_slots":host.overflow_slots(),"device_capabilities":host.model.capabilities(),"device_geometry":{"columns":host.model.columns(),"rows":host.model.rows(),"keys":host.model.key_count(),"dials":host.model.dial_count(),"physical_dials":host.model.physical_dial_count(),"encoder_columns":host.model.encoder_columns(),"encoder_rows":host.model.encoder_rows(),"key_width":host.model.key_width(),"key_height":host.model.key_height(),"strip_width":host.model.strip_width(),"strip_height":host.model.strip_height()},"clipboard":host.clipboard_snapshot(),"mixer":host.mixer_snapshot(),"screensaver":{"supported":host.model==Model::Plus,"active":host.saver.active,"reason":host.saver.reason,"generation":host.saver.generation,"preferences":host.saver_preferences(),"preview":host.saver.preview,"error":host.saver.error,"frames":host.saver.animation.as_ref().map(|animation|animation.len())},"system_resume":{"available":host.resume_available,"error":host.resume_error,"enabled":host.resume_monitor.is_some()},"session":host.session_state,"device_settings":{"desired":host.configured_controls(),"applied":host.controls_observed,"error":host.controls_error},"library":host.library.snapshot(),"running":host.running,"device":host.device,"firmware":host.firmware,"simulate":host.simulate,"connected":host.connected && !host.simulate,"device_online":host.connected,"connection_error":host.connection_error,
        "volume":host.volume_state,"volume_error":host.volume_error,"multi_runs":host.multi.snapshot(),"multi_pending":host.multi_pending.len(),"audio_outputs":host.audio_outputs,"audio_active":host.audio_active,"system_status":host.system_status,"slots":host.slots.iter().map(|s|slot_snapshot(host,s)).collect::<Vec<_>>(),
        "multi_action_feedback":!host.suppress_multi_feedback,"marketplace":host.marketplace,"plugin_language":host.language_snapshot(),"reference_app":host.reference_app,"inspectors":host.inspectors.values().map(|i|json!({"context":i.context,"plugin":host.plugins[i.plugin].id,"connected":i.sender.is_some()})).collect::<Vec<_>>(),
        "actions":host.actions.iter().map(|action|action_snapshot(host,action)).collect::<Vec<_>>(),
        "logs":host.logs,"application_monitor":{"enabled":host.application_monitor.is_some(),"error":host.application_error},"plugins":host.plugins.iter().map(|p|host.plugin_snapshot(p)).collect::<Vec<_>>()});
    value["typing_status"] = serde_json::to_value(
        host.system_actions
            .as_ref()
            .map(|w| w.typing_status.snapshot())
            .unwrap_or_default(),
    )
    .unwrap();
    value["history"] = host.history.snapshot();
    value["history"]["contexts"] = json!(
        host.library
            .profiles
            .iter()
            .flat_map(|p| &p.pages)
            .flat_map(|p| &p.slots)
            .flat_map(Slot::flatten)
            .map(|s| &s.context)
            .collect::<Vec<_>>()
    );
    value["smart_profiles"] = host.smart_snapshot();
    value["encoder_routes"] = json!(host.encoder_routes());
    value["encoder_markers"] = json!(host.encoder_routing.preview());
    value["encoder_background"] = json!(host.encoder_background_preview());
    value["encoder_background_configured"] = json!(host.encoder_background_configured());
    value["encoder_background_tiles"] = json!(host.encoder_background_tiles());
    value
}
async fn dashboard(State(state): State<Shared>) -> Json<Value> {
    Json(snapshot(&state.lock().unwrap()))
}
fn authorize(headers: &HeaderMap, host: &Host) -> bool {
    headers
        .get("x-deckforge-token")
        .and_then(|h| h.to_str().ok())
        == Some(&host.token)
}
async fn control(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if !authorize(&headers, &state.lock().unwrap()) {
        return (StatusCode::FORBIDDEN, "Invalid editor token").into_response();
    }
    let result = if matches!(
        request["command"].as_str(),
        Some("screensaver" | "appearance" | "plugin_update_apply" | "plugin_update_cancel")
    ) {
        tokio::task::spawn_blocking(move || Controller { shared: state }.command(request))
            .await
            .unwrap_or_else(|error| Err(anyhow::anyhow!("Image loader failed: {error}")))
    } else {
        apply_control(&mut state.lock().unwrap(), request)
    };
    match result {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":e.to_string()})),
        )
            .into_response(),
    }
}
async fn prepare_update(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if !authorize(&headers, &state.lock().unwrap()) {
        return (StatusCode::FORBIDDEN, "Invalid editor token").into_response();
    }
    let rollback = request["kind"] == "rollback";
    let result = tokio::task::spawn_blocking(move || {
        Controller { shared: state }.prepare_plugin_update(request, rollback)
    })
    .await
    .unwrap_or_else(|error| Err(anyhow::anyhow!("Package inspection failed: {error}")));
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}
fn apply_control(host: &mut Host, r: Value) -> Result<()> {
    let command = r["command"].as_str().context("Command missing")?;
    if matches!(command, "history_undo" | "history_redo") {
        return host.history_command(&r);
    }
    if let Some(label) = history::label(command) {
        let group = r["history_group"].as_str().map(str::to_owned);
        return host.history_edit(label, group.as_deref(), |host| apply_control_inner(host, r));
    }
    let barrier = matches!(
        command,
        "device_forget"
            | "device_add"
            | "install_plugin"
            | "uninstall_plugin"
            | "plugin_update_apply"
            | "plugin_rollback_apply"
            | "plugin_enabled"
    );
    let installing = command == "install_plugin";
    let plugin_count = host.plugins.len();
    let result = apply_control_inner(host, r);
    if barrier && (!installing || result.is_ok() || host.plugins.len() != plugin_count) {
        host.history.clear("Device or plugin configuration changed");
    }
    result
}
fn apply_control_inner(host: &mut Host, r: Value) -> Result<()> {
    let command = r["command"].as_str().context("Command missing")?;
    if command == "device_forget" {
        anyhow::ensure!(
            r["confirm"] == true,
            "Confirm removal of this device and its profiles"
        );
        return host.forget_device(r["device"].as_str().context("Device missing")?);
    }
    if command == "device_select" {
        return host.select_device(r["device"].as_str().context("Device missing")?);
    }
    if command == "device_add" {
        anyhow::ensure!(
            host.simulate && host.diagnostics,
            "Device fixtures require simulated diagnostic mode"
        );
        return host.add_simulated_device(
            r["device"].as_str().context("Device missing")?,
            serde_json::from_value(r["model"].clone())?,
        );
    }
    let global = matches!(
        command,
        "install_plugin"
            | "text_typing_setup"
            | "text_typing_disable"
            | "smart_preview"
            | "system_wake"
            | "plugin_applications"
            | "plugin_language"
            | "multi_action_feedback"
            | "plugin_enabled"
            | "uninstall_plugin"
            | "plugin_update_apply"
            | "plugin_update_cancel"
            | "restart_plugin"
            | "reference_app"
            | "mixer_config"
            | "mixer_refresh"
            | "mixer_control"
            | "soundboard_refresh"
            | "soundboard_stop"
    );
    let requested = r.get("device").and_then(Value::as_str);
    let owner = r
        .get("context")
        .and_then(Value::as_str)
        .and_then(|context| host.context_device(context));
    if command != "slot_transfer"
        && let (Some(requested), Some(owner)) = (requested, owner.as_deref())
    {
        anyhow::ensure!(requested == owner, "Action belongs to another device");
    }
    let target = if global {
        None
    } else if command == "slot_transfer" {
        requested
            .map(str::to_owned)
            .or_else(|| Some(host.device.clone()))
    } else {
        owner.or_else(|| requested.map(str::to_owned))
    };
    if let Some(target) = target {
        return host.with_device(&target, |host| apply_control_local(host, r));
    }
    apply_control_local(host, r)
}
fn apply_control_local(host: &mut Host, r: Value) -> Result<()> {
    if r["command"] == "device_rename" {
        return host.rename_device(r["name"].as_str().context("Device name missing")?);
    }
    if r["command"] == "profile_copy_to_device" {
        return host.copy_profile_to_device(&r);
    }
    let command = r["command"].as_str().context("Missing command")?;
    if command == "text_typing_setup" {
        return host
            .system_actions
            .get_or_insert_with(crate::system_actions::Worker::new)
            .submit("", crate::system_actions::Plan::PrepareTyping);
    }
    if command == "text_typing_disable" {
        if let Some(worker) = &host.system_actions {
            worker.typing_status.cancel();
        }
        return Ok(());
    }
    if matches!(command, "command_cancel" | "command_copy_output") {
        let context = r["context"].as_str().context("Action context missing")?;
        let slot = host.slot_any(context).context("Action no longer exists")?;
        anyhow::ensure!(
            slot.uuid == crate::system_command::UUID,
            "Select a Run command action"
        );
        let id = r["id"].as_str().context("Command run missing")?.to_owned();
        let plan = if command == "command_cancel" {
            crate::system_actions::Plan::CommandCancel(id)
        } else {
            crate::system_actions::Plan::CommandCopy(id)
        };
        return host
            .system_actions
            .as_ref()
            .context("No command results")?
            .submit(context, plan);
    }
    if matches!(command, "smart_config" | "smart_preview") {
        return host.smart_command(&r);
    }
    if matches!(
        r["command"].as_str(),
        Some(
            "slot_copy"
                | "slot_file_clipboard"
                | "slot_cut"
                | "slot_paste"
                | "slot_transfer"
                | "slot_clipboard_clear"
                | "slot_import"
                | "slot_export"
                | "child_paste"
        )
    ) {
        return host.transfer_command(&r);
    }
    if matches!(
        command,
        "profile_create"
            | "profile_select"
            | "profile_rename"
            | "profile_duplicate"
            | "profile_delete"
            | "page_create"
            | "page_select"
            | "page_rename"
            | "page_duplicate"
            | "page_delete"
            | "page_move"
            | "page_step"
            | "folder_back"
            | "slot_pin"
            | "slot_pin_exclusions"
            | "folder_create"
    ) {
        return host.library_command(&r);
    }
    match command {
        "page_background" => host.set_controller_background(&r),
        "encoder_appearance" => host.encoder_option(&r),
        "mixer_refresh" => host.ensure_mixer(),
        "mixer_control" => host.mixer_control(r.clone()),
        "mixer_config" => {
            let config = serde_json::from_value(
                r.get("settings").context("Mixer settings missing")?.clone(),
            )?;
            let expected = serde_json::from_value(
                r.get("expected")
                    .context("Previous mixer settings missing; refresh the editor")?
                    .clone(),
            )?;
            host.set_mixer_config(config, Some(&expected))
        }
        "sleep_device" => host.sleep_device("manual"),
        "wake_device" => host.wake_device(),
        "system_wake" => {
            anyhow::ensure!(
                host.simulate && host.diagnostics,
                "System-event injection is only available in simulated diagnostic mode"
            );
            host.system_wake()
        }
        "session_state" => {
            if !host.simulate || !host.diagnostics {
                bail!("Session injection is only available in simulated diagnostic mode");
            }
            host.session_update(crate::session_activity::State {
                locked: r["locked"].as_bool(),
                idle_ms: r["idle_ms"].as_u64(),
                source: "simulation".into(),
                error: None,
            })
        }
        "device_controls" => host.set_device_controls(serde_json::from_value(
            r.get("settings")
                .context("Device settings missing")?
                .clone(),
        )?),
        "export_profile" => host.export_profile(Path::new(
            r["path"].as_str().context("Export path missing")?,
        )),
        "open_inspector" => {
            host.open_inspector(r["context"].as_str().context("Missing action context")?)
        }
        "close_inspector" => {
            host.close_inspectors(Some(
                r["context"].as_str().context("Missing action context")?,
            ));
            Ok(())
        }
        "assign" => host.assign(
            r["controller"].as_str().unwrap_or(""),
            u8::try_from(r["index"].as_u64().context("Invalid index")?)?,
            r["uuid"].as_str().unwrap_or(""),
        ),
        "remove" => {
            let index = host
                .slots
                .iter()
                .position(|s| s.context == r["context"])
                .context("Unknown context")?;
            host.editable()?;
            host.sync_visible();
            let context = host.slots[index].context.clone();
            host.library.remove(&context);
            host.refresh_page()
        }
        "appearance" => appearance::legacy(host, &r),
        "import_profile" => host.import_profile(Path::new(
            r["path"].as_str().context("Profile path missing")?,
        )),
        "reference_app" => {
            let reference = r["path"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from);
            let reference = reference
                .map(|path| -> Result<PathBuf> {
                    let path = path.canonicalize()?;
                    crate::protected_manifest::validate_reference(&path)?;
                    Ok(path)
                })
                .transpose()?;
            let previous = std::mem::replace(&mut host.reference_app, reference);
            if let Err(error) = host.persist() {
                host.reference_app = previous;
                return Err(error);
            }
            Ok(())
        }
        "restart_plugin" => host.restart_plugin(r["id"].as_str().context("Plugin id missing")?),
        "plugin_applications" => host.save_application_mappings(&r),
        "plugin_language" => host.save_plugin_language(&r),
        "multi_action_feedback" => host.save_multi_feedback(&r),
        "plugin_enabled" => host.set_plugin_enabled(&r),
        "uninstall_plugin" => host.uninstall_plugin(&r),
        "plugin_update_apply" => host.apply_plugin_update(&r),
        "plugin_update_cancel" => {
            host.pending_updates
                .remove(r["ticket"].as_str().context("Update ticket missing")?);
            Ok(())
        }
        "install_plugin" => {
            let state_dir = host
                .state_path
                .parent()
                .context("Missing state directory")?
                .to_owned();
            let source = Path::new(
                r["path"]
                    .as_str()
                    .context("Plugin package or folder path missing")?,
            );
            let installed = if source.is_dir() {
                source.canonicalize()?
            } else {
                crate::package::install_with_reference(
                    source,
                    &state_dir.join("plugins"),
                    host.reference_app.as_deref(),
                )?
            };
            let (mut plugins, actions) =
                load_plugins(vec![installed.clone()], host.reference_app.as_deref())?;
            if host.plugins.iter().any(|p| p.id == plugins[0].id) {
                bail!(
                    "Package installed at {}, but this plugin is already loaded",
                    installed.display()
                );
            }
            if actions
                .iter()
                .any(|incoming| host.actions.iter().any(|a| a.uuid == incoming.uuid))
            {
                bail!("Plugin declares an action UUID already owned by another installed plugin");
            }
            if host.disabled_plugins.contains(&plugins[0].root) {
                plugins[0].runtime.status = "disabled";
            }
            if host.plugin_enabled(&plugins[0])
                && !plugins[0].runtime.load_failed
                && let Err(error) = launch_plugin(
                    &mut plugins[0],
                    &host.registration_devices(),
                    host.port,
                    &state_dir,
                    host.language,
                )
            {
                host.log(format!(
                    "Installed {}, but could not launch it: {error:#}",
                    installed.display()
                ));
            }
            host.plugins.extend(plugins);
            host.actions.extend(actions);
            host.refresh_localizations();
            host.refresh_previous_plugins();
            for device in host.device_ids() {
                host.with_device(&device, |host| host.install_bundled_profiles(true))?;
            }
            host.persist()
        }
        "device_event" => {
            if !host.simulate || !host.diagnostics {
                bail!("Device event injection is only available in simulated diagnostic mode");
            }
            let event = match r["event"].as_str() {
                Some("connected") => DeviceEvent::Connected {
                    model: r
                        .get("model")
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()?
                        .unwrap_or(host.model),
                    serial: r["serial"].as_str().context("Missing serial")?.into(),
                    firmware: r["firmware"].as_str().unwrap_or("test").into(),
                },
                Some("disconnected") => DeviceEvent::Disconnected {
                    reason: r["reason"]
                        .as_str()
                        .unwrap_or("Simulated disconnect")
                        .into(),
                },
                _ => bail!("Unknown device event"),
            };
            host.device_event(event)
        }
        "input" => {
            let index = u8::try_from(r["index"].as_u64().context("Missing index")?)?;
            let event = match r["event"].as_str().unwrap_or("") {
                "sensorDown" | "sensorUp" if host.model == Model::Neo && index < 2 => {
                    InputEvent::Sensor {
                        index,
                        pressed: r["event"] == "sensorDown",
                    }
                }
                "keyDown" | "keyUp" if index < host.model.key_count() => InputEvent::Key {
                    index,
                    pressed: r["event"] == "keyDown",
                },
                "dialDown" | "dialUp" if index < host.model.dial_count() => InputEvent::DialPress {
                    index,
                    pressed: r["event"] == "dialDown",
                },
                "dialRotate" if index < host.model.dial_count() => InputEvent::DialRotate {
                    index,
                    ticks: i8::try_from(r["ticks"].as_i64().context("Missing ticks")?)?,
                },
                "touchTap"
                    if host.model.capabilities().touchscreen && index < host.model.dial_count() =>
                {
                    let width = host.model.strip_width() / u32::from(host.model.dial_count());
                    let x =
                        u16::try_from(r["x"].as_u64().unwrap_or(
                            u64::from(index) * u64::from(width) + u64::from(width / 2),
                        ))?;
                    let y = u16::try_from(
                        r["y"]
                            .as_u64()
                            .unwrap_or(u64::from(host.model.strip_height() / 2)),
                    )?;
                    anyhow::ensure!(
                        u32::from(x) < host.model.strip_width()
                            && u32::from(y) < host.model.strip_height()
                            && u32::from(x) / width == u32::from(index),
                        "Touch coordinates do not match the selected dial"
                    );
                    InputEvent::Touch {
                        x,
                        y,
                        long: r["hold"].as_bool().unwrap_or(false),
                    }
                }
                "swipe" => {
                    anyhow::ensure!(
                        host.model.capabilities().touchscreen,
                        "This device has no touchscreen"
                    );
                    let point = |key: &str| -> Result<(u16, u16)> {
                        let x = u16::try_from(r[key][0].as_u64().context("Swipe x missing")?)?;
                        let y = u16::try_from(r[key][1].as_u64().context("Swipe y missing")?)?;
                        anyhow::ensure!(
                            u32::from(x) < host.model.strip_width()
                                && u32::from(y) < host.model.strip_height(),
                            "Swipe outside touch strip"
                        );
                        Ok((x, y))
                    };
                    InputEvent::Swipe {
                        start: point("start")?,
                        end: point("end")?,
                    }
                }
                _ => bail!("Invalid simulated event"),
            };
            if r["physical"] == true {
                host.physical_input(event)
            } else {
                host.input(event)
            }
        }
        "physical_dial_action" => host.test_physical_dial(
            u8::try_from(r["index"].as_u64().context("Dial index missing")?)?,
            r["event"].as_str().context("Dial operation missing")?,
        ),
        "physical_dial_cancel" => host.cancel_physical_dial(u8::try_from(
            r["index"].as_u64().context("Dial index missing")?,
        )?),
        "folder_exit" => host.save_folder_exit(&r),
        "folder_exit_reset" => host.reset_folder_exits(&r),
        "wheel_save" => host.save_wheel_draft(
            r["context"].as_str().context("Context missing")?,
            r["raw"].clone(),
            r.get("base"),
        ),
        "wheel_select" => host.select_wheel(
            r["context"].as_str().context("Context missing")?,
            usize::try_from(r["index"].as_u64().context("Index missing")?)?,
            true,
        ),
        "trigger_save" | "logic_save" => host.save_trigger(
            r["context"].as_str().context("Missing context")?,
            r["raw"].clone(),
        ),
        "stack_save" => host.save_stack(
            r["context"].as_str().context("Stack context missing")?,
            r["raw"].clone(),
        ),
        "stack_select" => host.select_stack(
            r["context"].as_str().context("Stack context missing")?,
            usize::try_from(r["index"].as_u64().context("Stack index missing")?)?,
            true,
        ),
        "multi_save" => host.save_multi(
            r["context"]
                .as_str()
                .context("Multi Action context missing")?,
            r["raw"].clone(),
        ),
        "multi_cancel" => {
            host.cancel_departing_triggers(&HashSet::new());
            host.cancel_all_multi();
            Ok(())
        }
        "soundboard_refresh" => host
            .audio
            .get_or_insert_with(crate::audio_output::Worker::new)
            .refresh(),
        "soundboard_stop" => {
            if let Some(worker) = &host.audio {
                worker.stop_all();
            }
            Ok(())
        }
        "settings" | "settings_patch" => {
            host.editable()?;
            let context = r["context"].as_str().context("Missing context")?;
            let mut slot = host
                .visible_slot(context)
                .cloned()
                .context("Unknown context")?;
            if !r["settings"].is_object() {
                bail!("Settings must be an object");
            }
            slot.settings = if r["command"] == "settings_patch" {
                settings::patch(&slot.settings, &r["base"], &r["settings"])?
            } else {
                r["settings"].clone()
            };
            if slot.uuid == crate::system_actions::HOTKEY
                && let Some(worker) = &host.system_actions
            {
                worker.submit(
                    context,
                    crate::system_actions::Plan::CancelDials(HashSet::from([context.to_owned()])),
                )?;
            }
            host.open_app_holds.remove(context);
            host.store_slot(slot.clone());
            host.persist()?;
            host.emit(&slot, "didReceiveSettings", json!({}));
            host.notify_inspectors(&slot.context, host.settings_event(&slot, None), None);
            Ok(())
        }

        _ => bail!("Unknown command"),
    }
}

enum Client {
    Plugin { launch: String, session: String },
    Inspector(String),
}
async fn socket_session(socket: WebSocket, state: Shared) {
    let (sink, mut stream) = socket.split();
    let registration = tokio::time::timeout(Duration::from_secs(5), stream.next()).await;
    let Some(Ok(Message::Text(text))) = registration.ok().flatten() else {
        return;
    };
    let Ok(frame) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let (sender, receiver) = mpsc::unbounded_channel();
    let drain = transport::Drain::default();
    let client = {
        let mut host = state.lock().unwrap();
        if frame["event"] == "registerPlugin" {
            let Some(index) = host.plugins.iter().position(|p| {
                host.plugin_enabled(p)
                    && frame["uuid"] == p.registration
                    && p.sender.is_none()
                    && matches!(p.runtime.status, "starting" | "disconnected")
            }) else {
                return;
            };
            let launch = host.plugins[index].registration.clone();
            let session = uuid::Uuid::new_v4().to_string();
            host.plugins[index].sender = Some(sender);
            host.plugins[index].runtime.session = Some(session.clone());
            host.plugins[index].runtime.output_drain = Some(drain.clone());
            host.plugins[index].runtime.status = "connected";
            host.plugins[index].runtime.error = None;
            host.hydrate_resources();
            let id = host.plugins[index].id.clone();
            host.log(format!("Plugin connected: {id}"));
            for device in host.device_ids() {
                let result=host.with_device(&device,|host| {
                    if let Some(sender)=&host.plugins[index].sender {
                        // The launch-time registration can precede a rename. Refresh
                        // offline metadata too, without claiming a connection.
                        let event = if host.connected { "deviceDidConnect" } else { "deviceDidChange" };
                        let _=sender.send(json!({"event":event,"device":host.device,"deviceInfo":host.device_info()}));
                    }
                    for slot in host.slots.iter().flat_map(Slot::visible_flatten) {
                        if host.actions.iter().any(|a|a.uuid==slot.uuid&&a.plugin==id) {host.emit(slot,"willAppear",json!({}));}
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    host.log(format!("Device replay failed: {error:#}"));
                }
            }
            // Callback messages follow registration and visible-action replay.
            host.flush_deep_links(&id);
            host.application_delivered.remove(&id);
            host.poll_applications();
            Client::Plugin { launch, session }
        } else if frame["event"] == "registerPropertyInspector" {
            let Some(registration) = frame["uuid"].as_str() else {
                return;
            };
            let Some(device) = host.inspector_device(registration) else {
                return;
            };
            if host
                .with_device(&device, |host| {
                    let inspector = host
                        .inspectors
                        .get_mut(registration)
                        .context("Inspector session expired")?;
                    anyhow::ensure!(inspector.sender.is_none(), "Inspector already connected");
                    inspector.sender = Some(sender);
                    let context = inspector.context.clone();
                    if let Some(slot) = host.visible_slot(&context) {
                        host.emit(slot, "propertyInspectorDidAppear", json!({}));
                    }
                    host.log(format!("Property inspector connected: {context}"));
                    Ok(())
                })
                .is_err()
            {
                return;
            }
            Client::Inspector(registration.to_owned())
        } else {
            return;
        }
    };
    // The writer never takes Host's mutex. Disabling/restarting may hold that
    // mutex while waiting for queued cleanup frames to leave this channel.
    let (controls, control_rx) = mpsc::channel(16);
    let mut writer = transport::Writer(tokio::spawn(transport::write(
        sink, receiver, control_rx, drain,
    )));
    let mut writer_done = false;
    loop {
        tokio::select! {
            _=&mut writer.0=>{writer_done=true;break;},
            incoming=stream.next()=>{match incoming {
                Some(Ok(Message::Text(text)))=>{
                    let result=serde_json::from_str::<Value>(&text).map_err(anyhow::Error::from).and_then(|message|{
                        let mut host=state.lock().unwrap();match &client {Client::Plugin { launch, session }=>{if let Some(index)=host.plugin_session_index(launch,session){host.plugin_message(index,message)}else{Ok(())}},Client::Inspector(id)=>host.inspector_message(id,message)}
                    });
                    if let Err(error)=result {state.lock().unwrap().log(format!("SDK command rejected: {error}"));}
                }
                Some(Ok(Message::Ping(data)))=>{if controls.try_send(Message::Pong(data)).is_err(){break;}},
                Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,
                _=>{}
            }}
        }
    }
    // A lost peer/reader must not leave an orphan writer awaiting Host messages.
    if !writer_done {
        writer.0.abort();
        let _ = (&mut writer.0).await;
    }
    let mut host = state.lock().unwrap();
    match client {
        Client::Plugin { launch, session } => {
            if let Some(index) = host.plugin_session_index(&launch, &session) {
                host.plugin_disconnected(index, &launch, &session);
            }
        }
        Client::Inspector(id) => host.close_inspector(&id),
    }
}

fn load_plugins(
    paths: Vec<PathBuf>,
    reference: Option<&Path>,
) -> Result<(Vec<Plugin>, Vec<Action>)> {
    plugins::load(paths, reference)
}
fn launch_plugins(host: &mut Host, port: u16, state_dir: &Path) {
    let devices = host.registration_devices();
    let mut failures = Vec::new();
    for plugin in &mut host.plugins {
        if host.disabled_plugins.contains(&plugin.root) {
            plugin.runtime.status = "disabled";
            continue;
        }
        if plugin.runtime.load_failed {
            continue;
        }
        if let Err(error) = launch_plugin(plugin, &devices, port, state_dir, host.language) {
            failures.push(format!("Plugin {} could not start: {error:#}", plugin.id));
        }
    }
    for error in failures {
        host.log(error);
    }
}
fn launch_plugin(
    plugin: &mut Plugin,
    devices: &[Value],
    port: u16,
    state_dir: &Path,
    language: &str,
) -> Result<()> {
    plugin.registration = uuid::Uuid::new_v4().to_string();
    plugin.sender = None;
    plugin.runtime = plugins::Runtime::starting();
    let result = (|| {
        let entry = crate::plugin_entry::resolve(&plugin.root, &plugin.manifest)?;
        plugin.runtime.entry_path = Some(entry.path.clone());
        plugin.runtime.entry_field = Some(entry.field);
        plugin.runtime.log_path = Some(std::path::absolute(state_dir)?.join(format!(
            "{}-{}.log",
            if entry.kind == crate::plugin_entry::Kind::Html {
                "webview"
            } else {
                "plugin"
            },
            plugin.registration
        )));
        launch_plugin_process(plugin, devices, port, state_dir, entry, language)
    })();
    if let Err(error) = &result {
        plugin.runtime.fail(format!("{error:#}"));
    }
    result
}

fn registration_info(plugin: &Plugin, devices: &[Value], language: &str) -> Value {
    json!({"application":{"font":"DejaVu Sans","language":language,"platform":"linux","platformVersion":std::env::consts::OS,"version":"7.6.0"},"plugin":{"uuid":plugin.id,"version":plugin.manifest["Version"]},"devicePixelRatio":1,"colors":{"buttonMouseOverBackgroundColor":"#303030","buttonPressedBackgroundColor":"#303030","buttonPressedBorderColor":"#505050","buttonPressedTextColor":"#ffffff","highlightColor":"#4bd3b0"},"devices":devices})
}
#[allow(clippy::too_many_arguments)]
fn spawn_webview(
    path: &Path,
    root: &Path,
    port: u16,
    uuid: &str,
    event: &str,
    info: &Value,
    action_info: Option<&Value>,
    visible: bool,
    state_dir: &Path,
) -> Result<Child> {
    let log = File::create(state_dir.join(format!("webview-{uuid}.log")))?;
    // Re-execute this exact Linux binary, even if installation replaced its path
    // while the desktop was running. Helpers must match the host's protocol.
    let mut command = Command::new("/proc/self/exe");
    command
        .arg("webview")
        .arg("--path")
        .arg(path)
        .arg("--root")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--uuid")
        .arg(uuid)
        .arg("--register-event")
        .arg(event)
        .arg("--info")
        .arg(info.to_string());
    if let Some(action) = action_info {
        command.arg("--action-info").arg(action.to_string());
    }
    if visible {
        command.arg("--visible");
    }
    command
        .current_dir(path.parent().context("HTML document has no directory")?)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    Ok(command.spawn()?)
}
fn launch_plugin_process(
    plugin: &mut Plugin,
    devices: &[Value],
    port: u16,
    state_dir: &Path,
    entry: crate::plugin_entry::Entry,
    language: &str,
) -> Result<()> {
    let info = registration_info(plugin, devices, language);
    match entry.kind {
        crate::plugin_entry::Kind::Html => {
            plugin.runtime.runtime_path = Some(std::env::current_exe()?);
            plugin.child = Some(spawn_webview(
                &entry.path,
                &plugin.root,
                port,
                &plugin.registration,
                "registerPlugin",
                &info,
                None,
                false,
                state_dir,
            )?);
        }
        crate::plugin_entry::Kind::Node | crate::plugin_entry::Kind::Native => {
            let mut command = if entry.kind == crate::plugin_entry::Kind::Node {
                let required = plugin.manifest["Nodejs"]["Version"]
                    .as_str()
                    .context("Nodejs.Version missing")?;
                let node = crate::plugin_entry::node_runtime(required)?;
                plugin.runtime.runtime_path = Some(node.clone());
                let mut command = Command::new(node);
                command
                    .args(crate::plugin_entry::NODE_FLAGS)
                    .arg(&entry.path);
                command
            } else {
                use sha2::{Digest, Sha256};
                use std::os::unix::fs::PermissionsExt;
                let parent = state_dir.join("plugin-data");
                let data = parent.join(format!("{:x}", Sha256::digest(plugin.id.as_bytes())));
                for path in [&parent, &data] {
                    anyhow::ensure!(
                        !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
                        "Plugin data directory is a symlink"
                    );
                    fs::create_dir_all(path)?;
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
                }
                plugin.runtime.runtime_path = Some(entry.path.clone());
                let mut command = Command::new(&entry.path);
                command.env("DECKFORGE_PLUGIN_DATA", data.canonicalize()?);
                command
            };
            let log = File::create(state_dir.join(format!("plugin-{}.log", plugin.registration)))?;
            plugin.child = Some(
                command
                    .args([
                        "-port",
                        &port.to_string(),
                        "-pluginUUID",
                        &plugin.registration,
                        "-registerEvent",
                        "registerPlugin",
                        "-info",
                        &info.to_string(),
                    ])
                    .current_dir(&plugin.root)
                    .stdin(Stdio::null())
                    .stdout(Stdio::from(log.try_clone()?))
                    .stderr(Stdio::from(log))
                    .spawn()?,
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn serve(
    serial: Option<String>,
    plugins: Vec<PathBuf>,
    profile_path: Option<PathBuf>,
    state_dir: PathBuf,
    port: u16,
    simulate: bool,
    seconds: Option<u64>,
    model: Option<Model>,
) -> Result<()> {
    run(
        HostOptions {
            model,
            serial,
            plugins,
            profile: profile_path,
            state_dir,
            port,
            simulate,
            seconds,
        },
        None,
    )
    .await
}
async fn run(options: HostOptions, embedded: Option<EmbeddedChannels>) -> Result<()> {
    let diagnostics = embedded.is_none();
    let HostOptions {
        model: requested_model,
        serial,
        plugins,
        profile: profile_path,
        state_dir,
        port,
        simulate,
        seconds,
    } = options;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    fs::create_dir_all(&state_dir)?;
    let session_lock = File::create(state_dir.join("session.lock"))?;
    session_lock
        .try_lock()
        .context("Another host is using this state directory")?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .context("Editor port is in use; choose another --port")?;
    let port = listener.local_addr()?.port();
    let state_path = state_dir.join("session.json");
    let fresh_session = !state_path.exists();
    let saved = if state_path.exists() {
        serde_json::from_slice::<Saved>(&fs::read(&state_path)?)
            .context("Invalid saved session; preserved without overwriting")?
    } else {
        Saved::default()
    };
    if state_path.exists()
        && serde_json::from_slice::<Value>(&fs::read(&state_path)?)?
            .get("model")
            .is_none()
    {
        let backup = state_path.with_extension("before-device-model.json");
        if !backup.exists() {
            fs::copy(&state_path, backup)?;
        }
    }
    for preferences in saved.screensavers.values() {
        preferences.validate()?;
    }
    saved.mixer.validate()?;
    for controls in saved.device_controls.values() {
        device::validate_controls(*controls)?;
    }
    let plugin_paths = if plugins.is_empty() {
        saved.plugins.clone()
    } else {
        plugins
    };
    let (plugins, mut actions) = load_plugins(plugin_paths, saved.reference_app.as_deref())?;
    let plugin_language = if saved.plugin_language.is_empty() {
        localization::default_preference()
    } else {
        saved.plugin_language.clone()
    };
    let language = crate::plugin_localization::resolve(&plugin_language)
        .context("Invalid saved plugin language")?;
    actions.extend(library::navigation_actions());
    actions.extend(multiaction::definitions());
    actions.extend(mixer_actions::definitions());
    actions.push(stack::action());
    actions.push(trigger::action());
    actions.push(trigger::logic_action());
    actions.push(wheel::action());
    for (uuid, name, _) in crate::system_actions::definitions()
        .into_iter()
        .chain(crate::audio_output::definitions())
    {
        if actions.iter().any(|a| a.uuid == uuid) {
            bail!("Plugin claims built-in action {uuid}");
        }
        actions.push(Action {
            uuid: uuid.into(),
            name: name.into(),
            controllers: if uuid == crate::system_actions::MOUSE_WHEEL {vec!["Encoder".into()]} else if matches!(
                uuid,
                crate::system_actions::HOTKEY | crate::system_actions::MULTIMEDIA | crate::brightness::UUID
            ) {
                vec!["Keypad".into(), "Encoder".into()]
            } else {
                vec!["Keypad".into()]
            },
            plugin: "deckforge.system".into(),
            raw: if uuid == crate::system_actions::HOTKEY_SWITCH {
                json!({"States":[{"Title":"Shortcut 1"},{"Title":"Shortcut 2"}]})
            } else {json!({"States":[{"Title":if uuid==crate::system_actions::OPEN {"Open"} else if uuid==crate::brightness::UUID {"Brightness"} else {name}}],"Encoder":{"layout":"$X1"}})},
        });
    }
    if state_path.exists() && saved.devices.is_empty() {
        let backup = state_path.with_extension("before-multi-device.json");
        if !backup.exists() {
            fs::copy(&state_path, backup)?;
        }
    }
    let saved_records = saved.devices.clone();
    let saved_scope = saved
        .active_scope
        .clone()
        .unwrap_or_else(|| "primary".into());
    let selected_scope = saved.selected_scope.clone();
    let has_pool = !saved_records.is_empty();
    let requested_serial = serial.clone();
    let remembered = device::Config::remembered(&saved);
    let selected_model =
        device::selected_model(remembered.as_ref(), serial.as_deref(), requested_model);
    let model = if has_pool {
        saved.model
    } else {
        selected_model.unwrap_or(saved.model)
    };
    let previous_device = remembered.as_ref().map(|config| config.id.clone());
    let serial_filter = serial.clone();
    let mut device_config = if has_pool {
        let record = saved_records
            .iter()
            .find(|r| r.scope == saved_scope)
            .context("Saved active device missing")?;
        remembered.unwrap_or_else(|| device::Config {
            model: record.model,
            id: record.device.clone(),
            serial: None,
            firmware: record.firmware.clone(),
        })
    } else {
        device::Config::select(remembered, serial)
    };
    device_config.validate()?;
    let registry_filter = crate::device_registry::Filter {
        serial: serial_filter,
        model: requested_model,
    };
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<crate::device_registry::Event>();
    let connection_error = (!simulate).then(|| "Waiting for device discovery".into());
    device_config.model = model;
    let connected = simulate;
    let device = if has_pool {
        saved_records
            .iter()
            .find(|record| record.scope == saved_scope)
            .context("Saved active device missing")?
            .device
            .clone()
    } else if simulate {
        if model == Model::Plus {
            "simulated-streamdeck-plus".into()
        } else {
            format!(
                "simulated-streamdeck-{}",
                serde_json::to_value(model)?.as_str().unwrap()
            )
        }
    } else {
        device_config.id.clone()
    };
    let firmware = if simulate {
        "simulation".into()
    } else {
        device_config.firmware.clone()
    };
    let mut library = if let Some(library) = saved.library {
        library
    } else {
        if state_path.exists() {
            let backup = state_path.with_extension("before-library.json");
            if !backup.exists() {
                fs::copy(&state_path, &backup)?;
            }
        }
        let legacy_model = if saved.slots.is_empty() {
            model
        } else {
            saved.model
        };
        Library::for_model(saved.slots, &device, legacy_model)
    };
    if !simulate && !has_pool {
        for profile in &mut library.profiles {
            if profile.device == "simulated-streamdeck-plus"
                || profile.device.starts_with("offline-streamdeck-plus-")
                || previous_device.as_deref() == Some(profile.device.as_str())
            {
                profile.device = device.clone();
            }
        }
    }
    library.restore_models()?;
    if !library.current()?.model.profile_compatible(model) {
        library.activate_model(model, &device)?;
    }
    library.validate()?;
    let slots = library.visible()?;
    let mut saver_preferences = saved.screensavers;
    if !simulate {
        screensaver::adopt_preferences(&mut saver_preferences, previous_device.as_deref(), &device);
    }
    let mut controls = saved.device_controls;
    if !simulate
        && !controls.contains_key(&device)
        && let Some(previous) = previous_device
            .as_ref()
            .filter(|id| id.starts_with("offline-streamdeck-plus-"))
        && let Some(previous_controls) = controls.remove(previous)
    {
        controls.insert(device.clone(), previous_controls);
    }
    let mut host = Host {
        plugin_asset_ids: saved.plugin_asset_ids,
        pending_updates: HashMap::new(),
        previous_plugins: HashMap::new(),
        pending_links: VecDeque::new(),
        marketplace: crate::marketplace::State::default(),
        registry_routes: HashMap::new(),
        registry_commands: None,
        forgetting_devices: HashSet::new(),
        canonical_graph_update: false,
        scope: saved_scope.clone(),
        selected_scope: saved_scope,
        other_devices: BTreeMap::new(),
        device_name: None,
        transport_generation: None,
        pending_hardware: None,
        model,
        mixer_config: saved.mixer,
        mixer: None,
        mixer_state: crate::mixer::Snapshot::default(),
        mixer_revision: 0,
        mixer_save_due: None,
        mixer_save_error: None,
        reference_app: saved.reference_app,
        screensavers: saver_preferences,
        saver: screensaver::Runtime::default(),
        session_monitor: if simulate {
            None
        } else {
            Some(crate::session_activity::Worker::new())
        },
        session_state: None,
        resume_monitor: (!simulate).then(crate::system_resume::Worker::new),
        resume_available: false,
        resume_error: None,
        brightness_repeats: HashMap::new(),
        open_app_holds: HashMap::new(),
        timers: Default::default(),
        device_controls: controls,
        controls_observed: device_session::Controls::default(),
        controls_error: None,
        connected,
        connection_error,
        device_config: if simulate {
            saved.device_config.clone()
        } else {
            Some(device_config.clone())
        },
        volume: None,
        volume_state: None,
        volume_error: None,
        redraw: None,
        badges: HashMap::new(),
        trigger_descriptions: HashMap::new(),
        triggers: trigger::Runtime::default(),
        wheels: wheel::Runtime::default(),
        key_image_cache: Default::default(),
        key_images: Default::default(),
        folder_exit: None,
        folder_back_holds: HashMap::new(),
        virtual_suppressed: HashSet::new(),
        stack_gestures: HashMap::new(),
        stack_cooldown: HashMap::new(),
        multi_owned: HashSet::new(),
        multi_held: HashMap::new(),
        multi_pending: HashMap::new(),
        multi_preparing: false,
        multi: crate::multi::Scheduler::new(),
        multi_clock: std::time::Instant::now(),
        multi_notify: Arc::new(tokio::sync::Notify::new()),
        audio: None,
        audio_outputs: Vec::new(),
        audio_active: HashMap::new(),
        system_actions: None,
        system_status: String::new(),
        library,
        clipboard: None,
        profile_assets: HashMap::new(),
        controller_backgrounds: controller_background::Runtime::default(),
        diagnostics,
        running: true,
        inspectors: HashMap::new(),
        port,
        plugins,
        actions,
        slots,
        globals: saved.globals,
        application_mappings: saved.application_mappings,
        history: Default::default(),
        smart_profiles: saved.smart_profiles,
        smart: smart_profiles::Runtime::default(),
        disabled_plugins: saved.disabled_plugins,
        suppress_multi_feedback: saved.suppress_multi_feedback,
        plugin_language,
        language,
        localizations: HashMap::new(),
        application_monitor: None,
        application_delivered: HashMap::new(),
        application_error: None,
        application_artwork: Default::default(),
        renderer: Renderer::new()?,
        hardware: None,
        device,
        firmware,
        token: uuid::Uuid::new_v4().to_string(),
        state_path,
        logs: Vec::new(),
        simulate,
        dial_pressed: vec![false; model.dial_count() as usize],
        encoder_routing: encoder_routes::Runtime::default(),
        sensor_pressed: vec![false; model.sensor_count() as usize],
        held: HashMap::new(),
        released: HashSet::new(),
        retired: VecDeque::new(),
    };
    host.refresh_localizations();
    host.refresh_previous_plugins();
    host.recover_updated_plugin_assets()?;
    host.restore_device_records(saved_records, selected_scope)?;
    if has_pool {
        host.startup_device_selection(requested_serial.as_deref(), requested_model)?;
    }
    let selected_device = host.device.clone();
    for device in host.device_ids() {
        let activate = profile_path.is_none() || device != selected_device;
        host.with_device(&device, |host| host.install_bundled_profiles(activate))?;
    }
    if host.mixer_config.enabled
        && let Err(error) = host.ensure_mixer()
    {
        host.mixer_state.error = Some(error.to_string());
        host.log(format!("Audio mixer unavailable: {error:#}"));
    }
    if let Some(path) = profile_path {
        host.import_profile(&path)?;
    } else if host.slots.is_empty() && fresh_session {
        let key = host
            .actions
            .iter()
            .find(|a| {
                !a.plugin.starts_with("deckforge.") && a.controllers.iter().any(|c| c == "Keypad")
            })
            .map(|a| a.uuid.clone());
        let dial = host
            .actions
            .iter()
            .find(|a| {
                !a.plugin.starts_with("deckforge.")
                    && a.controllers.iter().any(|c| c == "Encoder")
                    && a.raw["Encoder"]["layout"]
                        .as_str()
                        .is_none_or(|l| l == "$B1")
            })
            .map(|a| a.uuid.clone());
        if let Some(uuid) = key {
            host.assign("Keypad", 0, &uuid)?;
        }
        if let Some(uuid) = dial.filter(|_| host.model.dial_count() > 0) {
            host.assign("Encoder", 0, &uuid)?;
        }
    }
    let mut occupied = std::collections::HashSet::new();
    for slot in &mut host.slots {
        if !device::valid_slot(host.model, &slot.controller, slot.index) {
            bail!("Saved session contains invalid slot coordinates");
        }
        if !occupied.insert((slot.controller.clone(), slot.index)) {
            bail!("Saved session has duplicate slot coordinates");
        }
    }
    for index in 0..host.slots.len() {
        if host.slots[index].background.is_none() {
            let slot = host.slots[index].clone();
            host.slots[index].background = host.resolve_state_image(&slot, slot.state);
        }
    }
    if host.saver_preferences().image.is_some() {
        host.saver_animation()?;
    }
    // Validate and start plugins before writing any hardware image.
    launch_plugins(&mut host, port, &state_dir);
    let (registry_stop, registry_wait) =
        std::sync::mpsc::channel::<crate::device_registry::Command>();
    host.registry_commands = (!simulate).then(|| registry_stop.clone());
    let registry_records = host.device_records();
    let registry_worker = (!simulate).then(|| {
        std::thread::spawn(move || {
            let mut registry =
                crate::device_registry::Registry::new(events_tx.clone(), registry_filter.clone());
            for record in registry_records {
                if let Some(config) = record.config
                    && let Some(serial) = config.serial
                {
                    let allowed = registry_filter
                        .serial
                        .as_ref()
                        .is_none_or(|filter| filter == &serial)
                        && registry_filter
                            .model
                            .is_none_or(|model| model == record.model);
                    // A new serial with no known family must be discovered before a
                    // fixed-model worker is created. Existing identities can retry offline.
                    if allowed
                        && (config.firmware != "unknown" || registry_filter.model.is_some())
                        && let Err(error) = registry.watch(serial, record.model)
                    {
                        let _ = events_tx.send(crate::device_registry::Event::Diagnostic {
                            error: format!("{error:#}"),
                        });
                    }
                }
            }
            loop {
                if let Err(error) = registry.refresh() {
                    let _ = events_tx.send(crate::device_registry::Event::Diagnostic {
                        error: format!("{error:#}"),
                    });
                }
                match registry_wait.recv_timeout(Duration::from_secs(1)) {
                    Ok(crate::device_registry::Command::Forget(key)) => {
                        if let Err(error) = registry.forget(key) {
                            let _ = events_tx.send(crate::device_registry::Event::Diagnostic {
                                error: format!("Device retirement: {error:#}"),
                            });
                        }
                    }
                    Ok(crate::device_registry::Command::Stop)
                    | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            registry.shutdown()
        })
    });
    host.send_device_controls();
    host.sync_visible();
    host.hydrate_library_children();
    host.hydrate_resources();
    host.slots = host.library.visible()?;
    host.render()?;
    host.persist()?;
    host.log(format!(
        "{} http://127.0.0.1:{port} | {} | firmware {}",
        if diagnostics {
            "Editor"
        } else {
            "Plugin endpoint"
        },
        host.device,
        host.firmware
    ));
    let shared = Arc::new(Mutex::new(host));
    let mut embedded_stop = if let Some(channels) = embedded {
        if channels
            .ready
            .send(Controller {
                shared: shared.clone(),
            })
            .is_err()
        {
            stop_plugins(&mut shared.lock().unwrap());
            let _ = registry_stop.send(crate::device_registry::Command::Stop);
            if let Some(worker) = registry_worker {
                let _ = worker.join();
            }
            bail!("Desktop startup was cancelled");
        }
        Some(channels.stop)
    } else {
        None
    };
    let events_state = shared.clone();
    let event_task = tokio::spawn(async move {
        while let Some(event) = events_rx.recv().await {
            let mut host = events_state.lock().unwrap();
            if let Err(error) = host.registry_event(event) {
                host.system_status = format!("Device/action failed: {error:#}");
                let message = host.system_status.clone();
                host.log(message);
            }
        }
    });
    let mut app = Router::new().route("/", get(root));
    if diagnostics {
        app = app
            .route("/api/state", get(dashboard))
            .route("/api/plugin-update/prepare", post(prepare_update))
            .route("/api/control", post(control));
    }
    let app = app
        .with_state(shared.clone())
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let allowed = request
                    .headers()
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .is_some_and(|host| {
                        [
                            format!("127.0.0.1:{port}"),
                            format!("localhost:{port}"),
                            format!("[::1]:{port}"),
                        ]
                        .iter()
                        .any(|expected| host.eq_ignore_ascii_case(expected))
                    });
                if !allowed {
                    return StatusCode::FORBIDDEN.into_response();
                }
                next.run(request).await
            },
        ));
    let multi_state = shared.clone();
    let multi_notify = shared.lock().unwrap().multi_notify.clone();
    let multi_task = tokio::spawn(async move {
        loop {
            let wait = {
                let mut host = multi_state.lock().unwrap();
                if !host.running {
                    break;
                }
                let mut earliest = None;
                for device in host.device_ids() {
                    let due = host.with_device(&device, |host| {
                        host.poll_multi();
                        host.poll_controller_background();
                        host.poll_encoder_routes();
                        host.poll_logic();
                        host.poll_triggers();
                        host.poll_wheels();
                        host.poll_folder_exit();
                        host.poll_key_images();
                        host.poll_feedback();
                        host.poll_brightness();
                        host.poll_open_apps();
                        host.poll_timers();
                        host.poll_saver();
                        Ok(host
                            .multi
                            .next_due()
                            .into_iter()
                            .chain(host.triggers.repeater.next_due())
                            .chain(host.encoder_routing.next_due())
                            .chain(host.controller_backgrounds.next_due())
                            .chain(host.triggers.logic.next_due())
                            .chain(host.wheels.next_due())
                            .chain(host.next_folder_exit())
                            .chain(host.key_images.next_due())
                            .chain(host.next_feedback())
                            .chain(host.next_brightness())
                            .chain(host.next_open_app())
                            .chain(host.next_timer())
                            .chain(host.next_saver())
                            .min())
                    });
                    if let Ok(Some(due)) = due {
                        earliest = Some(earliest.map_or(due, |old: Duration| old.min(due)));
                    }
                }
                earliest.map(|deadline| deadline.saturating_sub(host.multi_clock.elapsed()))
            };
            tokio::select! {
                _=multi_notify.notified()=>{},
                _=tokio::time::sleep(wait.unwrap_or(Duration::from_secs(86400)))=>{},
            }
        }
    });
    let monitor_state = shared.clone();
    let monitor_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let expired = {
                let mut host = monitor_state.lock().unwrap();
                if !host.running {
                    break;
                }
                host.monitor_children();
                host.poll_system_actions();
                host.poll_session();
                host.poll_system_resume();
                host.poll_applications();
                host.poll_smart_profiles();
                host.poll_audio();
                host.poll_volume();
                host.poll_mixer();
                host.take_expired_updates()
            };
            if !expired.is_empty() {
                let _ = tokio::task::spawn_blocking(move || drop(expired)).await;
            }
        }
    });
    let stop = async move {
        let duration = seconds.map(Duration::from_secs).unwrap_or(Duration::MAX);
        tokio::select! {
            _=async {if let Some(stop)=embedded_stop.as_mut(){let _=stop.await;}else{std::future::pending::<()>().await;}}=>{},
            _=async{if seconds.is_some(){tokio::time::sleep(duration).await;}else{std::future::pending::<()>().await;}}=>{},
            _=tokio::signal::ctrl_c()=>{},
            _=terminate.recv()=>{},
        }
    };
    // Stop our children before graceful HTTP shutdown waits for their sockets.
    let stop_shared = shared.clone();
    let stop = async move {
        stop.await;
        stop_plugins(&mut stop_shared.lock().unwrap());
    };
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(stop)
        .await;
    // Worker shutdown can wait for audio-server acknowledgements. Do not hold
    // the host lock while restoring its explicitly assigned streams.
    let mixer = shared.lock().unwrap().mixer.take();
    drop(mixer);
    let focus_workers = {
        let mut host = shared.lock().unwrap();
        stop_plugins(&mut host);
        let workers = host.stop_smart_profiles();
        host.persist()?;
        host.replace_clipboard(None);
        workers
    };
    drop(focus_workers);
    let _ = registry_stop.send(crate::device_registry::Command::Stop);
    if let Some(worker) = registry_worker {
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("Device registry panicked"))??;
    }
    event_task.abort();
    monitor_task.abort();
    multi_task.abort();
    result?;
    Ok(())
}

fn stop_plugins(host: &mut Host) {
    if !host.running {
        return;
    }
    if let Some(worker) = &host.volume {
        worker.cancel();
    }
    for device in host.device_ids() {
        let result = host.with_device(&device, |host| {
            host.cancel_departing_triggers(&HashSet::new());
            host.cancel_all_multi();
            host.release_device_holds();
            if host.model == Model::Plus && !host.saver.locked {
                host.wake_device()?;
            }
            host.brightness_repeats.clear();
            host.open_app_holds.clear();
            if !host.badges.is_empty() {
                host.badges.clear();
                host.render()?;
            }
            host.close_inspectors(None);
            Ok(())
        });
        if let Err(error) = result {
            host.log(format!("Device shutdown: {error:#}"));
        }
    }
    if let Some(worker) = &host.audio {
        worker.cancel();
    }
    if let Some(worker) = &host.system_actions {
        worker.cancel();
    }
    if host.mixer_save_due.is_some()
        && let Err(error) = host.persist()
    {
        host.log(format!("Save mixer controls on exit: {error:#}"));
    }
    host.running = false;
    if let Some(redraw) = &host.redraw {
        redraw();
    }
    let mut incomplete_drains = Vec::new();
    for plugin in &mut host.plugins {
        plugin.runtime.session = None;
        plugin.runtime.status = "stopped";
        if !plugin.drain_output() {
            incomplete_drains.push(plugin.id.clone());
        }
        stop_child(&mut plugin.child);
    }
    for id in incomplete_drains {
        host.log(format!(
            "Plugin output drain failed or timed out; stopped owned child: {id}"
        ));
    }
}

pub(crate) fn decode_crop_svg(data: &[u8]) -> Result<DynamicImage> {
    display::decode_image(data, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_reference_cannot_replace_saved_compatibility_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut host = fixture(root.path());
        let original = root.path().join("previous-reference.exe");
        host.reference_app = Some(original.clone());
        let bad = root.path().join("bad.exe");
        std::fs::write(&bad, b"invalid").unwrap();
        assert!(apply_control(&mut host, json!({"command":"reference_app","path":bad})).is_err());
        assert_eq!(host.reference_app, Some(original));
        apply_control(&mut host, json!({"command":"reference_app","path":null})).unwrap();
        let saved: Value =
            serde_json::from_slice(&std::fs::read(&host.state_path).unwrap()).unwrap();
        assert!(host.reference_app.is_none());
        assert!(saved["reference_app"].is_null());
    }

    #[test]
    fn embedded_host_has_direct_controller_and_releases_its_session_on_close() {
        let temp = tempfile::tempdir().unwrap();
        let options = HostOptions {
            model: None,
            serial: None,
            plugins: vec![],
            profile: None,
            state_dir: temp.path().to_owned(),
            port: 0,
            simulate: true,
            seconds: None,
        };
        let running = start(options.clone()).unwrap();
        assert_eq!(running.controller.snapshot()["simulate"], true);
        {
            use std::io::{Read, Write};
            let port = running.controller.shared.lock().unwrap().port;
            for path in ["/", "/api/state"] {
                let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                write!(
                    socket,
                    "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                let mut response = String::new();
                socket.read_to_string(&mut response).unwrap();
                assert!(response.starts_with("HTTP/1.1 404"));
            }
        }
        assert!(
            running
                .controller
                .command(json!({"command":"input","event":"dialRotate","index":9,"ticks":1}))
                .is_err()
        );
        // A second owner must not overwrite this session, even on another port.
        assert!(start(options.clone()).is_err());
        let controller = running.controller.clone();
        drop(running);
        assert_eq!(controller.snapshot()["running"], false);
        assert!(
            controller
                .command(json!({"command":"input","event":"keyDown","index":0}))
                .is_err()
        );
        // Joining the embedded host releases its file lock before the next launch.
        let restarted = start(options).unwrap();
        assert_eq!(
            restarted.controller.snapshot()["device"],
            "simulated-streamdeck-plus"
        );
        drop(restarted);
    }
    pub(super) fn fixture(root: &Path) -> Host {
        let mut host = Host {
            plugin_asset_ids: HashMap::new(),
            pending_updates: HashMap::new(),
            previous_plugins: HashMap::new(),
            pending_links: VecDeque::new(),
            marketplace: crate::marketplace::State::default(),
            registry_routes: HashMap::new(),
            registry_commands: None,
            forgetting_devices: HashSet::new(),
            canonical_graph_update: false,
            scope: "primary".into(),
            selected_scope: "primary".into(),
            other_devices: BTreeMap::new(),
            device_name: None,
            transport_generation: None,
            pending_hardware: None,
            model: Model::Plus,
            mixer_config: crate::mixer::Config::default(),
            mixer: None,
            mixer_state: crate::mixer::Snapshot::default(),
            mixer_revision: 0,
            mixer_save_due: None,
            mixer_save_error: None,
            reference_app: None,
            screensavers: HashMap::new(),
            saver: screensaver::Runtime::default(),
            session_monitor: None,
            session_state: None,
            resume_monitor: None,
            resume_available: false,
            resume_error: None,
            brightness_repeats: HashMap::new(),
            open_app_holds: HashMap::new(),
            timers: Default::default(),
            connected: true,
            connection_error: None,
            device_config: None,
            device_controls: HashMap::new(),
            controls_observed: device_session::Controls::default(),
            controls_error: None,
            volume: None,
            volume_state: None,
            volume_error: None,
            redraw: None,
            badges: HashMap::new(),
            trigger_descriptions: HashMap::new(),
            triggers: trigger::Runtime::default(),
            wheels: wheel::Runtime::default(),
            key_image_cache: Default::default(),
            key_images: Default::default(),
            folder_exit: None,
            folder_back_holds: HashMap::new(),
            virtual_suppressed: HashSet::new(),
            stack_gestures: HashMap::new(),
            stack_cooldown: HashMap::new(),
            multi_owned: HashSet::new(),
            multi_held: HashMap::new(),
            multi_pending: HashMap::new(),
            multi_preparing: false,
            multi: crate::multi::Scheduler::new(),
            multi_clock: std::time::Instant::now(),
            multi_notify: Arc::new(tokio::sync::Notify::new()),
            audio: None,
            audio_outputs: Vec::new(),
            audio_active: HashMap::new(),
            system_actions: None,
            system_status: String::new(),
            library: Library::default(),
            clipboard: None,
            profile_assets: HashMap::new(),
            controller_backgrounds: controller_background::Runtime::default(),
            diagnostics: false,
            running: true,
            inspectors: HashMap::new(),
            port: 0,
            plugins: vec![Plugin {
                asset_generation: plugins::new_asset_generation(),
                id: "test.plugin".into(),
                name: "Test".into(),
                root: root.to_owned(),
                manifest: json!({}),
                registration: "secret".into(),
                runtime: plugins::Runtime::default(),
                child: None,
                sender: None,
            }],
            actions: vec![Action {
                uuid: "test.action".into(),
                name: "Test".into(),
                controllers: vec!["Keypad".into()],
                plugin: "test.plugin".into(),
                raw: json!({"States":[{}]}),
            }],
            slots: vec![Slot {
                model: Model::Plus,
                children: vec![],
                child_id: String::new(),
                resources: None,
                asset_source: None,
                appearances: Default::default(),
                in_multi_action: false,
                event_coordinates: None,
                encoder_overrides: HashMap::new(),
                context: "test.context".into(),
                controller: "Keypad".into(),
                index: 0,
                uuid: "test.action".into(),
                name: "Test".into(),
                settings: json!({}),
                title: "before".into(),
                state: 0,
                feedback: json!({}),
                layout: "$B1".into(),
                raw: json!({}),
                image_changed: false,
                title_changed: false,
                pinned: false,
                states: json!([{}, {}]),
                background: None,
                layout_cache: None,
                image_cache: HashMap::new(),
                display_warnings: Vec::new(),
                preview: String::new(),
                software_icon: String::new(),
            }],
            globals: HashMap::new(),
            application_mappings: HashMap::new(),
            history: Default::default(),
            smart_profiles: HashMap::new(),
            smart: smart_profiles::Runtime::default(),
            disabled_plugins: BTreeSet::new(),
            suppress_multi_feedback: false,
            plugin_language: "system".into(),
            language: "en",
            localizations: HashMap::new(),
            application_monitor: None,
            application_delivered: HashMap::new(),
            application_error: None,
            application_artwork: Default::default(),
            renderer: Renderer::new().unwrap(),
            hardware: None,
            device: "test-device".into(),
            firmware: "test".into(),
            token: "test".into(),
            state_path: root.join("state.json"),
            logs: Vec::new(),
            simulate: true,
            dial_pressed: vec![false; Model::Plus.dial_count() as usize],
            encoder_routing: encoder_routes::Runtime::default(),
            sensor_pressed: vec![],
            held: HashMap::new(),
            released: HashSet::new(),
            retired: VecDeque::new(),
        };
        host.library = Library::legacy(host.slots.clone(), "test-device");
        host
    }
    fn multi_fixture(host: &mut Host, action: &str, settings: Value) -> Slot {
        host.actions.extend(multiaction::definitions());
        host.assign("Keypad", 0, multiaction::ROUTINE).unwrap();
        let context = host.slots[0].context.clone();
        host.save_multi(&context,json!({"Actions":[{"Actions":[{"UUID":action,"ActionID":"child","Settings":settings,"State":0,"States":[{}]}]},{"Actions":[]}]})).unwrap();
        host.slots[0].clone()
    }
    #[test]
    fn legacy_multi_child_storage_migrates_without_changing_contexts() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let parent = multi_fixture(&mut host, "test.action", json!({"keep":73}));
        let context = parent.children[0][0].context.clone();
        fn legacy(value: &mut Value) {
            match value {
                Value::Object(fields) => {
                    fields.remove("in_multi_action");
                    if let Some(value) = fields.remove("children") {
                        fields.insert("multi_children".into(), value);
                    }
                    if let Some(value) = fields.remove("child_id") {
                        fields.insert("multi_id".into(), value);
                    }
                    for value in fields.values_mut() {
                        legacy(value);
                    }
                }
                Value::Array(items) => {
                    for value in items {
                        legacy(value);
                    }
                }
                _ => {}
            }
        }
        let mut saved = serde_json::to_value(parent).unwrap();
        legacy(&mut saved);
        let mut loaded: Slot = serde_json::from_value(saved).unwrap();
        loaded.hydrate_children(&host.actions).unwrap();
        assert_eq!(loaded.children[0][0].context, context);
        assert_eq!(loaded.children[0][0].settings, json!({"keep":73}));
        let payload = action_payload(&loaded.children[0][0]);
        assert_eq!(payload["isInMultiAction"], true);
        assert!(payload.get("coordinates").is_none());
    }
    #[test]
    fn stack_inspector_updates_only_its_selected_child() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.actions[0].controllers.push("Encoder".into());
        host.actions.push(stack::action());
        host.assign("Encoder", 1, crate::dial_stack::UUID).unwrap();
        let parent = host
            .slots
            .iter()
            .find(|s| s.uuid == crate::dial_stack::UUID)
            .unwrap()
            .context
            .clone();
        host.save_stack(&parent,json!({"Actions":[{"UUID":"test.action","ActionID":"a","Settings":{"value":10}},{"UUID":"test.action","ActionID":"b","Settings":{"value":20}}],"Settings":{"CurrentIdx":0}})).unwrap();
        let selected = host.slot_any(&parent).unwrap().children[0][0]
            .context
            .clone();
        let inactive = host.slot_any(&parent).unwrap().children[0][1]
            .context
            .clone();
        let (tx, _rx) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "stack-pi".into(),
            Inspector {
                context: selected.clone(),
                plugin: 0,
                child: None,
                sender: Some(tx),
            },
        );
        host.inspector_message(
            "stack-pi",
            json!({"event":"setSettings","context":selected,"payload":{"value":19}}),
        )
        .unwrap();
        assert_eq!(
            host.slot_any(&parent).unwrap().children[0][0].settings["value"],
            19
        );
        assert_eq!(
            host.slot_any(&parent).unwrap().children[0][1].settings["value"],
            20
        );
        assert_eq!(
            host.slot_any(&parent).unwrap().raw["Actions"][0]["Settings"]["value"],
            19
        );
        assert!(
            host.inspector_message(
                "stack-pi",
                json!({"event":"setSettings","context":inactive,"payload":{"value":99}})
            )
            .is_err()
        );
        let payload = action_payload(host.slot_any(&selected).unwrap());
        assert_eq!(payload["coordinates"], json!({"column":1,"row":0}));
        assert_eq!(payload["isInMultiAction"], false);
    }
    #[test]
    fn cancelling_a_due_batch_releases_only_dispatched_children() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        let parent = multi_fixture(&mut host, "test.action", json!({}));
        while rx.try_recv().is_ok() {}
        host.start_multi(&parent).unwrap();
        let due = host.multi.due(host.multi_clock.elapsed());
        host.cancel_multi(&parent.context);
        for event in due {
            host.dispatch_multi(event, false).unwrap();
        }
        assert!(
            rx.try_recv().is_err(),
            "Cancelled queued press must emit neither down nor up"
        );
        host.start_multi(&parent).unwrap();
        host.poll_multi();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        host.cancel_multi(&parent.context);
        host.cancel_multi(&parent.context);
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn cancelling_keyboard_preparation_cannot_start_the_sequence_later() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let parent = multi_fixture(&mut host, "test.action", json!({}));
        host.multi_pending.insert(parent.context.clone(), vec![]);
        host.multi_owned.insert(parent.context.clone());
        host.cancel_multi(&parent.context);
        host.multi_keyboard_prepared(true);
        assert!(!host.multi_running(&parent.context));
        assert!(host.multi.snapshot().is_empty());
    }
    #[test]
    fn nested_audio_is_embedded_on_export() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let audio = temp.path().join("nested.wav");
        fs::write(&audio, b"RIFF-preserved-audio").unwrap();
        multi_fixture(&mut host, crate::audio_output::PLAY, json!({"path":audio}));
        let export = temp.path().join("multi.streamDeckProfile");
        host.export_profile(&export).unwrap();
        let bundle = profile::load_bundle(&export).unwrap();
        let parent = bundle
            .pages
            .iter()
            .flat_map(|p| &p.slots)
            .find(|s| s.uuid == multiaction::ROUTINE)
            .unwrap();
        let resource = parent.raw["Actions"][0]["Actions"][0]["Resources"]["file"]
            .as_str()
            .unwrap();
        assert_eq!(
            bundle.resource(resource).unwrap().1,
            b"RIFF-preserved-audio"
        );
    }
    #[test]
    fn inactive_state_updates_do_not_change_live_title_and_invalid_targets_are_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.plugin_message(0,json!({"event":"setTitle","context":"test.context","payload":{"title":"inactive","state":1}})).unwrap();
        assert_eq!(host.slots[0].title, "before");
        assert!(host.plugin_message(0,json!({"event":"setTitle","context":"test.context","payload":{"title":"bad","target":99}})).is_err());
        assert_eq!(host.slots[0].title, "before");
        host.plugin_message(
            0,
            json!({"event":"setState","context":"test.context","payload":{"state":1}}),
        )
        .unwrap();
        assert_eq!(host.slots[0].title, "inactive");
    }
    #[test]
    fn plugin_context_ownership_and_settings_correlation_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.plugin_message(
            0,
            json!({"event":"getSettings","context":"test.context","id":"request-7"}),
        )
        .unwrap();
        let response = rx.try_recv().unwrap();
        assert_eq!(response["id"], "request-7");
        assert_eq!(response["payload"]["state"], 0);
        assert_eq!(response["payload"]["resources"], json!({}));
        host.actions[0].plugin = "another.plugin".into();
        assert!(
            host.plugin_message(
                0,
                json!({"event":"setTitle","context":"test.context","payload":{"title":"bad"}})
            )
            .is_err()
        );
        assert_eq!(host.slots[0].title, "before");
    }
    #[test]
    fn imported_images_survive_session_restart() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.slots[0].background = Some(DynamicImage::ImageRgb8(RgbImage::from_pixel(
            3,
            2,
            Rgb([23, 45, 67]),
        )));
        host.persist().unwrap();
        let restored: Saved = serde_json::from_slice(&fs::read(&host.state_path).unwrap()).unwrap();
        assert_eq!(
            restored.slots[0]
                .background
                .as_ref()
                .unwrap()
                .to_rgb8()
                .get_pixel(1, 1),
            &Rgb([23, 45, 67])
        );
    }
    #[test]
    fn resources_cannot_escape_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("plugin");
        fs::create_dir(&root).unwrap();
        fs::write(tmp.path().join("secret"), "secret").unwrap();
        assert!(safe_file(&root, "../secret").is_err());
        std::os::unix::fs::symlink(tmp.path().join("secret"), root.join("link")).unwrap();
        assert!(safe_file(&root, "link").is_err());
    }
    #[test]
    fn desktop_plugin_install_cannot_claim_another_plugins_action() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("another.sdPlugin");
        fs::create_dir(&plugin).unwrap();
        fs::write(plugin.join("manifest.json"),json!({"UUID":"another.plugin","Name":"Other","Actions":[{"UUID":"test.action","Name":"Collision"}]}).to_string()).unwrap();
        let mut host = fixture(temp.path());
        let error = apply_control(&mut host, json!({"command":"install_plugin","path":plugin}))
            .unwrap_err();
        assert!(error.to_string().contains("already owned"));
        assert_eq!(host.plugins.len(), 1);
        assert_eq!(host.actions[0].plugin, "test.plugin");
    }
    #[test]
    fn inspector_identity_is_bound_to_one_action_and_replies_use_real_context() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "inspector-token".into(),
            Inspector {
                context: "test.context".into(),
                plugin: 0,
                child: None,
                sender: Some(tx),
            },
        );
        host.inspector_message(
            "inspector-token",
            json!({"event":"getSettings","context":"inspector-token","id":"request"}),
        )
        .unwrap();
        let reply = rx.try_recv().unwrap();
        assert_eq!(reply["context"], "test.context");
        assert_eq!(reply["action"], "test.action");
        assert_eq!(reply["id"], "request");
        for extra in [
            json!({"context":"other.context"}),
            json!({"context":"inspector-token","targetContext":"other.context"}),
            json!({"context":"inspector-token","action":"other.action"}),
        ] {
            let mut message = extra;
            message["event"] = json!("setSettings");
            message["payload"] = json!({"bad":true});
            assert!(host.inspector_message("inspector-token", message).is_err());
            assert_eq!(host.slots[0].settings, json!({}));
        }
    }
    #[test]
    fn inspector_cannot_edit_readonly_profile_settings() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.library.current_mut().unwrap().readonly = true;
        let (tx, _rx) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "pi".into(),
            Inspector {
                context: "test.context".into(),
                plugin: 0,
                child: None,
                sender: Some(tx),
            },
        );
        assert!(
            host.inspector_message("pi", json!({"event":"setSettings","payload":{"bad":true}}))
                .is_err()
        );
        assert_eq!(host.slots[0].settings, json!({}));
    }
    #[test]
    fn plugin_settings_and_messages_reach_only_the_bound_inspector() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (other_tx, mut other_rx) = mpsc::unbounded_channel();
        host.inspectors.insert(
            "pi".into(),
            Inspector {
                context: "test.context".into(),
                plugin: 0,
                child: None,
                sender: Some(tx),
            },
        );
        host.inspectors.insert(
            "other".into(),
            Inspector {
                context: "other.context".into(),
                plugin: 0,
                child: None,
                sender: Some(other_tx),
            },
        );
        host.plugin_message(
            0,
            json!({"event":"setSettings","context":"test.context","payload":{"count":7}}),
        )
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["payload"]["settings"]["count"], 7);
        assert!(other_rx.try_recv().is_err());
        host.plugin_message(0,json!({"event":"sendToPropertyInspector","context":"test.context","payload":{"message":"hello"}})).unwrap();
        assert_eq!(
            rx.try_recv().unwrap()["payload"],
            json!({"message":"hello"})
        );
        assert!(other_rx.try_recv().is_err());
    }
    #[test]
    fn invalid_dial_feedback_does_not_poison_the_previous_display_state() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.slots[0].controller = "Encoder".into();
        host.slots[0].feedback = json!({"indicator":30});
        let result=host.plugin_message(0,json!({"event":"setFeedback","context":"test.context","payload":{"indicator":{"bar_fill_c":"not a CSS color"}}}));
        assert!(result.is_err());
        assert_eq!(host.slots[0].feedback, json!({"indicator":30}));
    }
    #[test]
    fn failed_unregistered_inspector_is_reaped_and_its_error_is_visible() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        fs::write(temp.path().join("webview-failed.log"), "callback missing").unwrap();
        let mut child = Command::new("/bin/false").spawn().unwrap();
        child.wait().unwrap();
        host.inspectors.insert(
            "failed".into(),
            Inspector {
                context: "test.context".into(),
                plugin: 0,
                child: Some(child),
                sender: None,
            },
        );
        host.monitor_children();
        assert!(!host.inspectors.contains_key("failed"));
        assert!(
            host.logs
                .iter()
                .any(|line| line.contains("callback missing"))
        );
    }
    #[test]
    fn page_picker_follows_reordered_navigation() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.library_command(&json!({"command":"page_create","name":"Second"}))
            .unwrap();
        let id = host.library.active_page.clone();
        host.library_command(&json!({"command":"page_move","id":id,"direction":-1}))
            .unwrap();
        let snapshot = host.library.snapshot();
        assert_eq!(snapshot["profiles"][0]["pages"][0]["id"], id);
        assert_eq!(snapshot["profiles"][0]["page_order"][0], id);
    }
    #[test]
    fn goto_page_uses_one_based_reordered_pages_and_rejects_invalid_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let first = host.library.active_page.clone();
        host.library_command(&json!({"command":"page_create","name":"Second"}))
            .unwrap();
        let second = host.library.active_page.clone();
        host.goto_page(&json!({})).unwrap();
        assert_eq!(host.library.active_page, first);
        host.goto_page(&json!({"PageIndex":2})).unwrap();
        assert_eq!(host.library.active_page, second);
        for bad in [
            json!(0),
            json!(-1),
            json!(3),
            json!(1.5),
            json!("1"),
            Value::Null,
        ] {
            assert!(host.goto_page(&json!({"PageIndex":bad})).is_err());
            assert_eq!(host.library.active_page, second);
        }
        host.library_command(&json!({"command":"page_move","id":second,"direction":-1}))
            .unwrap();
        host.goto_page(&json!({"PageIndex":2})).unwrap();
        assert_eq!(host.library.active_page, first);
    }
    #[test]
    fn export_cannot_replace_retained_source() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let source = temp.path().join("retained.streamDeckProfile");
        fs::write(&source, b"original").unwrap();
        host.library.current_mut().unwrap().source = Some(source.clone());
        assert!(host.export_profile(&source).is_err());
        assert_eq!(fs::read(source).unwrap(), b"original");
    }
    #[test]
    fn changing_pages_releases_the_old_button_before_disappearance() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        host.library_command(&json!({"command":"page_create","name":"Other"}))
            .unwrap();
        let release = rx.try_recv().unwrap();
        assert_eq!(release["event"], "keyUp");
        assert_eq!(release["context"], "test.context");
        assert_eq!(rx.try_recv().unwrap()["event"], "willDisappear");
        host.assign("Keypad", 0, "test.action").unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "willAppear");
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn pin_exclusions_hide_only_selected_pages_and_release_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let first = host.library.active_page.clone();
        host.library_command(&json!({"command":"slot_pin","context":"test.context","pinned":true}))
            .unwrap();
        host.library_command(&json!({"command":"page_create","name":"Other"}))
            .unwrap();
        let other = host.library.active_page.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        host.library_command(&json!({"command":"slot_pin_exclusions","context":"test.context","expected":[],"excluded_pages":[other]})).unwrap();
        assert!(host.slots.is_empty());
        assert_eq!(rx.try_recv().unwrap()["event"], "keyUp");
        assert_eq!(rx.try_recv().unwrap()["event"], "willDisappear");
        host.library_command(&json!({"command":"page_select","id":first}))
            .unwrap();
        assert_eq!(host.slots[0].context, "test.context");
        let restored: library::Library =
            serde_json::from_value(serde_json::to_value(&host.library).unwrap()).unwrap();
        assert_eq!(
            library::pin_exclusions(restored.slot("test.context").unwrap()),
            json!([other])
        );
        let pins = host.library.current().unwrap().pin_page.clone();
        host.library_command(&json!({"command":"page_select","id":pins}))
            .unwrap();
        assert_eq!(host.slots[0].context, "test.context");
        for pages in [json!(["missing"]), json!([pins]), json!([other, other])] {
            assert!(host.library_command(&json!({"command":"slot_pin_exclusions","context":"test.context","expected":[other],"excluded_pages":pages})).is_err());
        }
        assert!(host.library_command(&json!({"command":"slot_pin_exclusions","context":"test.context","expected":[],"excluded_pages":[]})).is_err());
        host.library_command(&json!({"command":"page_delete","id":other}))
            .unwrap();
        assert_eq!(
            library::pin_exclusions(host.library.slot("test.context").unwrap()),
            json!([])
        );
    }
    #[test]
    fn pin_exclusions_are_remapped_when_copying_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.library_command(&json!({"command":"slot_pin","context":"test.context","pinned":true}))
            .unwrap();
        host.library_command(&json!({"command":"page_create","name":"Excluded"}))
            .unwrap();
        let excluded = host.library.active_page.clone();
        host.library_command(&json!({"command":"slot_pin_exclusions","context":"test.context","expected":[],"excluded_pages":[excluded]})).unwrap();
        let original = host.library.active_profile.clone();
        host.library_command(&json!({"command":"profile_duplicate","id":original,"name":"Copy"}))
            .unwrap();
        let profile = host.library.current().unwrap();
        let copied = profile.pages.iter().find(|p| p.name == "Excluded").unwrap();
        let pins = profile
            .pages
            .iter()
            .find(|p| p.id == profile.pin_page)
            .unwrap();
        assert_ne!(copied.id, excluded);
        assert_eq!(library::pin_exclusions(&pins.slots[0]), json!([copied.id]));
    }
    #[test]
    fn pinned_button_keeps_its_press_ownership_when_page_changes() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        host.plugins[0].sender = Some(tx);
        host.library_command(&json!({"command":"slot_pin","context":"test.context","pinned":true}))
            .unwrap();
        host.input(InputEvent::Key {
            index: 0,
            pressed: true,
        })
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["event"], "keyDown");
        host.library_command(&json!({"command":"page_create","name":"Other"}))
            .unwrap();
        assert!(rx.try_recv().is_err());
        host.input(InputEvent::Key {
            index: 0,
            pressed: false,
        })
        .unwrap();
        let release = rx.try_recv().unwrap();
        assert_eq!(release["event"], "keyUp");
        assert_eq!(release["context"], "test.context");
    }
    #[test]
    fn dedicated_pin_editor_can_remove_a_hidden_pin_without_deleting_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.library_command(&json!({"command":"slot_pin","context":"test.context","pinned":true}))
            .unwrap();
        host.library_command(&json!({"command":"page_create","name":"Other"}))
            .unwrap();
        let other = host.library.active_page.clone();
        host.assign("Keypad", 0, "test.action").unwrap();
        let override_id = host.slots[0].context.clone();
        let pins = host.library.current().unwrap().pin_page.clone();
        host.library_command(&json!({"command":"page_select","id":pins}))
            .unwrap();
        assert_eq!(host.slots[0].context, "test.context");
        assert!(host.slots[0].pinned);
        apply_control(
            &mut host,
            json!({"command":"remove","context":"test.context"}),
        )
        .unwrap();
        assert!(host.slots.is_empty());
        host.select_page(&other, false).unwrap();
        assert_eq!(host.slots[0].context, override_id);
    }
    #[test]
    fn duplicate_profiles_remap_folder_links_and_action_instance_ids() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        host.slots[0].raw = json!({"UUID":"test.action","ActionID":"original-id"});
        host.sync_visible();
        host.library_command(&json!({"command":"folder_create","name":"Nested","index":3}))
            .unwrap();
        let original = host.library.current().unwrap().clone();
        host.library_command(
            &json!({"command":"profile_duplicate","id":original.id,"name":"Copy"}),
        )
        .unwrap();
        let copy = host.library.current().unwrap();
        assert!(
            copy.pages
                .iter()
                .all(|page| !original.pages.iter().any(|old| old.id == page.id))
        );
        for slot in copy.pages.iter().flat_map(|page| &page.slots) {
            assert!(
                !original
                    .pages
                    .iter()
                    .flat_map(|p| &p.slots)
                    .any(|old| old.context == slot.context)
            );
            if slot.uuid == library::OPEN_FOLDER {
                assert!(
                    copy.pages
                        .iter()
                        .any(|p| p.id == slot.settings["ProfileUUID"])
                );
            }
            if slot.uuid == "test.action" {
                assert_ne!(slot.raw["ActionID"], "original-id");
            }
        }
        host.library.validate().unwrap();
    }
}
