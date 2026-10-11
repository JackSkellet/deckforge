//! Independent implementation of Elgato's public plugin WebSocket contract.
use crate::{
    hardware::{Deck, InputEvent},
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
use futures_util::{SinkExt, StreamExt};
use image::{DynamicImage, GenericImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Cursor,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

type Shared = Arc<Mutex<Host>>;

/// The native editor uses this in-process controller, without HTTP or a browser.
#[derive(Clone)]
pub struct Controller {
    shared: Shared,
}
impl Controller {
    pub fn snapshot(&self) -> Value {
        snapshot(&self.shared.lock().unwrap())
    }
    pub fn command(&self, request: Value) -> Result<()> {
        apply_control(&mut self.shared.lock().unwrap(), request)
    }
}
#[derive(Clone)]
pub struct HostOptions {
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
    states: Value,
    #[serde(default, with = "image_serde")]
    background: Option<DynamicImage>,
    #[serde(skip)]
    preview: String,
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
    id: String,
    name: String,
    root: PathBuf,
    manifest: Value,
    registration: String,
    child: Option<Child>,
    sender: Option<mpsc::UnboundedSender<Value>>,
}
impl Drop for Plugin {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
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
    plugins: Vec<PathBuf>,
    slots: Vec<Slot>,
    #[serde(default)]
    globals: HashMap<String, Value>,
}
enum Draw {
    Key(u8, DynamicImage),
    Strip(DynamicImage),
    Stop,
}
struct Host {
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
    dial_pressed: [bool; 4],
}

impl Host {
    fn import_profile(&mut self, path: &Path) -> Result<()> {
        let mut imported = profile::load(path)?;
        // Preserve the previous working session before replacing its active page.
        if self.state_path.exists() {
            fs::copy(
                &self.state_path,
                self.state_path.with_extension("before-import.json"),
            )?;
        }
        for slot in &self.slots {
            self.emit(slot, "willDisappear", json!({}));
        }

        imported.warn_unknown_actions(&self.actions.iter().map(|a| a.uuid.clone()).collect());
        self.slots.clear();
        for s in &imported.slots {
            if !((s.controller == "Keypad" && s.column < 4 && s.row < 2)
                || (s.controller == "Encoder" && s.column < 4 && s.row == 0))
            {
                self.log(format!(
                    "Preserved in source but not placed: out-of-range {} {},{}",
                    s.controller, s.column, s.row
                ));
                continue;
            }
            let index = if s.controller == "Encoder" {
                s.column
            } else {
                s.row * 4 + s.column
            };
            let state = s.raw["State"].as_u64().unwrap_or(0);
            let name = s.raw["Name"].as_str().unwrap_or(&s.uuid).to_owned();
            let mut background = None;
            if let Some(path) = s.states[state as usize]["Image"].as_str() {
                background = imported
                    .asset(path)
                    .ok()
                    .and_then(|bytes| image::load_from_memory(&bytes).ok());
            }
            self.slots.push(Slot {
                context: uuid::Uuid::new_v4().to_string(),
                controller: s.controller.clone(),
                index,
                uuid: s.uuid.clone(),
                name: name.clone(),
                settings: s.settings.clone(),
                title: s.states[state as usize]["Title"]
                    .as_str()
                    .unwrap_or(&name)
                    .into(),
                state,
                feedback: json!({}),
                layout: "$B1".into(),
                states: s.states.clone(),
                background,
                preview: String::new(),
            });
            if !self.actions.iter().any(|a| a.uuid == s.uuid) {
                self.log(format!(
                    "Profile action {} has no installed compatible plugin; retained, inactive",
                    s.uuid
                ));
            }
        }
        for warning in &imported.warnings {
            self.log(warning);
        }

        for slot in &self.slots {
            self.emit(slot, "willAppear", json!({}));
        }
        self.render()?;
        self.persist()
    }
    fn log(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("{message}");
        self.logs.push(message);
        if self.logs.len() > 100 {
            self.logs.remove(0);
        }
    }
    fn persist(&self) -> Result<()> {
        let pending = self.state_path.with_extension("tmp");
        let data = serde_json::to_vec_pretty(&Saved {
            plugins: self.plugins.iter().map(|p| p.root.clone()).collect(),
            slots: self.slots.clone(),
            globals: self.globals.clone(),
        })?;
        fs::write(&pending, data)?;
        fs::rename(pending, &self.state_path)?;
        Ok(())
    }
    fn emit(&self, slot: &Slot, event: &str, extra: Value) {
        let action = self.actions.iter().find(|a| a.uuid == slot.uuid);
        let plugin = action.and_then(|a| self.plugins.iter().find(|p| p.id == a.plugin));
        if let Some(sender) = plugin.and_then(|p| p.sender.as_ref()) {
            let mut payload = json!({"controller":slot.controller,"coordinates":{"column":slot.index%4,"row":if slot.controller=="Encoder" {0}else{slot.index/4}},"settings":slot.settings,"state":slot.state,"isInMultiAction":false,"resources":{}});
            if let Some(map) = extra.as_object() {
                for (k, v) in map {
                    payload[k] = v.clone();
                }
            }
            let _=sender.send(json!({"event":event,"action":slot.uuid,"context":slot.context,"device":self.device,"payload":payload}));
        }
    }
    fn render(&mut self) -> Result<()> {
        let mut strip = DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 100, Rgb([14, 21, 29])));
        for slot in &mut self.slots {
            let picture = if slot.controller == "Encoder" {
                self.renderer.dial(&slot.title, &slot.feedback)
            } else {
                self.renderer.key(&slot.title, slot.background.as_ref())
            };
            let mut png = Cursor::new(Vec::new());
            picture.write_to(&mut png, image::ImageFormat::Png)?;
            slot.preview = STANDARD.encode(png.into_inner());
            if slot.controller == "Encoder" {
                strip.copy_from(&picture, slot.index as u32 * 200, 0)?;
            } else if let Some(hardware) = &self.hardware {
                let _ = hardware.send(Draw::Key(slot.index, picture));
            }
        }
        if let Some(hardware) = &self.hardware {
            for index in 0..8 {
                if !self
                    .slots
                    .iter()
                    .any(|s| s.controller == "Keypad" && s.index == index)
                {
                    let _ = hardware.send(Draw::Key(index, self.renderer.key("", None)));
                }
            }
            let _ = hardware.send(Draw::Strip(strip));
        }
        Ok(())
    }
    fn assign(&mut self, controller: &str, index: u8, uuid: &str) -> Result<()> {
        if !((controller == "Keypad" && index < 8) || (controller == "Encoder" && index < 4)) {
            bail!("Invalid Stream Deck+ slot");
        }
        let action = self
            .actions
            .iter()
            .find(|a| a.uuid == uuid && a.controllers.iter().any(|c| c == controller))
            .cloned()
            .context("Action is not installed or does not support this controller")?;
        if controller == "Encoder"
            && action.raw["Encoder"]["layout"]
                .as_str()
                .is_some_and(|layout| layout != "$B1")
        {
            bail!(
                "This action requires a layout that is not implemented yet; choose Feedback for the supported $B1 sample"
            );
        }
        if let Some(old) = self
            .slots
            .iter()
            .find(|s| s.controller == controller && s.index == index)
        {
            self.emit(old, "willDisappear", json!({}));
        }
        self.slots
            .retain(|s| !(s.controller == controller && s.index == index));
        let title = action.raw["States"][0]["Title"]
            .as_str()
            .unwrap_or(&action.name)
            .to_owned();
        let mut slot = Slot {
            context: uuid::Uuid::new_v4().to_string(),
            controller: controller.into(),
            index,
            uuid: uuid.into(),
            name: action.name.clone(),
            settings: json!({}),
            title,
            state: 0,
            feedback: json!({}),
            layout: action.raw["Encoder"]["layout"]
                .as_str()
                .unwrap_or("$B1")
                .into(),
            states: action.raw["States"].clone(),
            background: None,
            preview: String::new(),
        };
        if let Some(plugin) = self.plugins.iter().find(|p| p.id == action.plugin) {
            slot.background = state_image(&plugin.root, &slot.states, 0);
        }
        self.emit(&slot, "willAppear", json!({}));
        self.slots.push(slot);
        self.render()?;
        self.persist()
    }
    fn input(&mut self, event: InputEvent) -> Result<()> {
        let (controller, index, name, extra) = match &event {
            InputEvent::Key { index, pressed } => (
                "Keypad",
                *index,
                if *pressed { "keyDown" } else { "keyUp" },
                json!({}),
            ),
            InputEvent::DialPress { index, pressed } => {
                self.dial_pressed[*index as usize] = *pressed;
                (
                    "Encoder",
                    *index,
                    if *pressed { "dialDown" } else { "dialUp" },
                    json!({}),
                )
            }
            InputEvent::DialRotate { index, ticks } => (
                "Encoder",
                *index,
                "dialRotate",
                json!({"ticks":ticks,"pressed":self.dial_pressed[*index as usize]}),
            ),
            InputEvent::Touch { x, y, long } => (
                "Encoder",
                (*x / 200) as u8,
                "touchTap",
                json!({"hold":long,"tapPos":[x%200,y]}),
            ),
            InputEvent::Swipe { .. } => {
                self.log(format!("{event:?}: page navigation not yet implemented"));
                return Ok(());
            }
        };
        self.log(format!("{event:?}"));
        if let Some(slot) = self
            .slots
            .iter()
            .find(|s| s.controller == controller && s.index == index)
        {
            self.emit(slot, name, extra);
        }
        Ok(())
    }
    fn plugin_message(&mut self, plugin_index: usize, message: Value) -> Result<()> {
        let event = message["event"].as_str().context("Missing event")?;
        let context = message["context"].as_str().unwrap_or("");
        let payload = message.get("payload").cloned().unwrap_or(json!({}));
        let plugin = &self.plugins[plugin_index];
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
            self.globals.insert(plugin.id.clone(), payload);
            return self.persist();
        }
        let index = self
            .slots
            .iter()
            .position(|s| s.context == context)
            .context("Unknown action context")?;
        if !self
            .actions
            .iter()
            .any(|a| a.uuid == self.slots[index].uuid && a.plugin == plugin.id)
        {
            bail!("Plugin does not own this context");
        }
        if event == "getSettings" {
            let slot = &self.slots[index];
            let mut reply = json!({"event":"didReceiveSettings","context":slot.context,"action":slot.uuid,"device":self.device,"payload":{"settings":slot.settings,"state":slot.state,"resources":{},"controller":slot.controller,"coordinates":{"column":slot.index%4,"row":if slot.controller=="Encoder"{0}else{slot.index/4}},"isInMultiAction":false}});
            if let Some(id) = message.get("id") {
                reply["id"] = id.clone();
            }
            if let Some(sender) = &plugin.sender {
                let _ = sender.send(reply);
            }
            return Ok(());
        }
        let root = plugin.root.clone();
        let slot = &mut self.slots[index];
        if matches!(event, "setImage" | "setTitle") {
            if payload
                .get("state")
                .is_some_and(|v| v.as_u64() != Some(slot.state))
            {
                bail!("Updating an inactive state is not implemented");
            }
            if payload.get("target").is_some_and(|v| v.as_u64() != Some(0)) {
                bail!("Separate hardware/software targets are not implemented");
            }
        }
        match event {
            "setSettings" => {
                slot.settings = payload;
                self.persist()?;
                return Ok(());
            }
            "setTitle" => {
                slot.title = payload["title"].as_str().unwrap_or(&slot.name).to_owned();
            }
            "setImage" => {
                let source = payload["image"].as_str().unwrap_or("");
                slot.background = if source.is_empty() {
                    state_image(&root, &slot.states, slot.state)
                } else {
                    Some(load_image(&root, source)?)
                };
            }
            "setState" => {
                let state = payload["state"].as_u64().context("Invalid state")?;
                if state >= slot.states.as_array().map_or(1, Vec::len) as u64 {
                    bail!("State out of range");
                }
                slot.state = state;
                slot.background = state_image(&root, &slot.states, state);
                if let Some(title) = slot.states[state as usize]["Title"].as_str() {
                    slot.title = title.into();
                }
            }
            "setFeedback" => {
                let object = payload.as_object().context("Feedback must be an object")?;
                for (key, value) in object {
                    slot.feedback[key] = value.clone();
                }
            }
            "setFeedbackLayout" => {
                let layout = payload["layout"].as_str().context("Missing layout")?;
                if layout != "$B1" {
                    bail!("Only the $B1 title/value/indicator feedback subset is implemented");
                }
                slot.layout = layout.into();
            }
            "setTriggerDescription" => {
                return Ok(());
            }
            "showOk" | "showAlert" => {
                self.log(format!("{event} for {context}"));
                return Ok(());
            }
            _ => {
                bail!("Unsupported plugin command: {event}");
            }
        }
        self.render()?;
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
fn load_image(root: &Path, source: &str) -> Result<DynamicImage> {
    let data = if source.starts_with("data:image/") {
        let (header, content) = source.split_once(',').context("Invalid image URL")?;
        if !header.ends_with(";base64") {
            bail!("Only base64 image URLs supported");
        }
        STANDARD.decode(content)?
    } else {
        fs::read(safe_file(root, source)?)?
    };
    if data.len() > 8 * 1024 * 1024 {
        bail!("Image exceeds 8 MiB");
    }
    Ok(image::load_from_memory(&data)?)
}
fn state_image(root: &Path, states: &Value, state: u64) -> Option<DynamicImage> {
    let image = states.get(state as usize)?.get("Image")?.as_str()?;
    load_image(root, image)
        .or_else(|_| load_image(root, &format!("{image}.png")))
        .or_else(|_| load_image(root, &format!("{image}.jpg")))
        .ok()
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
    let token = state.lock().unwrap().token.clone();
    Html(include_str!("ui.html").replace("__TOKEN__", &token)).into_response()
}
fn snapshot(host: &Host) -> Value {
    json!({"device":host.device,"firmware":host.firmware,"simulate":host.simulate,"connected":host.hardware.is_some(),
        "slots":host.slots.iter().map(|s|json!({"context":s.context,"controller":s.controller,"index":s.index,"uuid":s.uuid,"name":s.name,"settings":s.settings,"title":s.title,"state":s.state,"feedback":s.feedback,"layout":s.layout,"preview":s.preview})).collect::<Vec<_>>(),
        "actions":host.actions,"logs":host.logs,"plugins":host.plugins.iter().map(|p|json!({"id":p.id,"name":p.name,"connected":p.sender.is_some()})).collect::<Vec<_>>()})
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
    let mut host = state.lock().unwrap();
    if !authorize(&headers, &host) {
        return (StatusCode::FORBIDDEN, "Invalid editor token").into_response();
    }
    match apply_control(&mut host, request) {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":e.to_string()})),
        )
            .into_response(),
    }
}
fn apply_control(host: &mut Host, r: Value) -> Result<()> {
    match r["command"].as_str().context("Missing command")? {
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
            host.emit(&host.slots[index], "willDisappear", json!({}));
            host.slots.remove(index);
            host.render()?;
            host.persist()
        }
        "appearance" => {
            let slot = host
                .slots
                .iter_mut()
                .find(|s| s.context == r["context"])
                .context("Unknown context")?;
            slot.title = r["title"]
                .as_str()
                .context("Title must be text")?
                .to_owned();
            host.render()?;
            host.persist()
        }
        "import_profile" => host.import_profile(Path::new(
            r["path"].as_str().context("Profile path missing")?,
        )),
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
                crate::package::install(source, &state_dir.join("plugins"))?
            };
            let (mut plugins, actions) = load_plugins(vec![installed.clone()])?;
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
            launch_plugin(&mut plugins[0], &host.device, host.port, &state_dir).with_context(
                || format!("Installed {}, but could not launch it", installed.display()),
            )?;
            host.plugins.extend(plugins);
            host.actions.extend(actions);
            host.persist()
        }
        "input" => {
            let index = u8::try_from(r["index"].as_u64().context("Missing index")?)?;
            let event = match r["event"].as_str().unwrap_or("") {
                "keyDown" | "keyUp" if index < 8 => InputEvent::Key {
                    index,
                    pressed: r["event"] == "keyDown",
                },
                "dialDown" | "dialUp" if index < 4 => InputEvent::DialPress {
                    index,
                    pressed: r["event"] == "dialDown",
                },
                "dialRotate" if index < 4 => InputEvent::DialRotate {
                    index,
                    ticks: i8::try_from(r["ticks"].as_i64().context("Missing ticks")?)?,
                },
                _ => bail!("Invalid simulated event"),
            };
            host.input(event)
        }
        "settings" => {
            let index = host
                .slots
                .iter()
                .position(|s| s.context == r["context"])
                .context("Unknown context")?;
            if !r["settings"].is_object() {
                bail!("Settings must be an object");
            }
            host.slots[index].settings = r["settings"].clone();
            host.emit(&host.slots[index], "didReceiveSettings", json!({}));
            host.persist()
        }
        _ => bail!("Unknown command"),
    }
}

async fn socket_session(socket: WebSocket, state: Shared) {
    let (mut sink, mut stream) = socket.split();
    let registration = tokio::time::timeout(Duration::from_secs(5), stream.next()).await;
    let Some(Ok(Message::Text(text))) = registration.ok().flatten() else {
        return;
    };
    let Ok(frame) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let plugin_index = {
        let mut host = state.lock().unwrap();
        let index = host.plugins.iter().position(|p| {
            frame["event"] == "registerPlugin"
                && frame["uuid"] == p.registration
                && p.sender.is_none()
        });
        let Some(index) = index else {
            return;
        };
        let _ = sender.send(json!({"event":"deviceDidConnect","device":host.device,"deviceInfo":{"name":"Stream Deck +","type":7,"size":{"columns":4,"rows":2}}}));
        host.plugins[index].sender = Some(sender);
        let id = host.plugins[index].id.clone();
        host.log(format!("Plugin connected: {id}"));
        for slot in &host.slots {
            if host
                .actions
                .iter()
                .any(|a| a.uuid == slot.uuid && a.plugin == id)
            {
                host.emit(slot, "willAppear", json!({}));
            }
        }
        index
    };
    loop {
        tokio::select! {
            outgoing=receiver.recv()=>{let Some(outgoing)=outgoing else{break;};if sink.send(Message::Text(outgoing.to_string().into())).await.is_err(){break;}}
            incoming=stream.next()=>{match incoming {
                Some(Ok(Message::Text(text)))=>{
                    let result=serde_json::from_str::<Value>(&text).map_err(anyhow::Error::from).and_then(|message|state.lock().unwrap().plugin_message(plugin_index,message));
                    if let Err(error)=result {state.lock().unwrap().log(format!("Plugin command rejected: {error}"));}
                }
                Some(Ok(Message::Ping(data)))=>{if sink.send(Message::Pong(data)).await.is_err(){break;}}
                Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,
                _=>{}
            }}
        }
    }
    let mut host = state.lock().unwrap();
    host.plugins[plugin_index].sender = None;
    let id = host.plugins[plugin_index].id.clone();
    host.log(format!("Plugin disconnected: {id}"));
}

fn load_plugins(paths: Vec<PathBuf>) -> Result<(Vec<Plugin>, Vec<Action>)> {
    let mut plugins = Vec::new();
    let mut actions = Vec::new();
    for path in paths {
        let root = path.canonicalize()?;
        let manifest: Value = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
        let id = manifest["UUID"]
            .as_str()
            .context("Plugin UUID missing")?
            .to_owned();
        if plugins.iter().any(|p: &Plugin| p.id == id) {
            bail!("Duplicate plugin {id}");
        }
        for a in manifest["Actions"]
            .as_array()
            .context("Plugin Actions missing")?
        {
            let uuid = a["UUID"]
                .as_str()
                .context("Action UUID missing")?
                .to_owned();
            if actions.iter().any(|a: &Action| a.uuid == uuid) {
                bail!("Duplicate action {uuid}");
            }
            actions.push(Action {
                uuid,
                name: a["Name"].as_str().unwrap_or("Action").into(),
                controllers: a["Controllers"]
                    .as_array()
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["Keypad".into()]),
                plugin: id.clone(),
                raw: a.clone(),
            });
        }
        plugins.push(Plugin {
            id,
            name: manifest["Name"].as_str().unwrap_or("Plugin").into(),
            root,
            manifest,
            registration: uuid::Uuid::new_v4().to_string(),
            child: None,
            sender: None,
        });
    }
    Ok((plugins, actions))
}
fn node_runtime(required: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os(format!("DECKFORGE_NODE{required}")) {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([PathBuf::from("node"), PathBuf::from("/usr/bin/node")]);
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        let mut roots = vec![data.join("nvm"), home.join(".nvm/versions/node")];
        if let Some(nvm) = std::env::var_os("NVM_DIR") {
            roots.push(PathBuf::from(nvm).join("versions/node"));
        }
        for root in roots {
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(&format!("v{required}."))
                    {
                        candidates.push(entry.path().join("bin/node"));
                    }
                }
            }
        }
    }
    candidates
        .into_iter()
        .find(|node| {
            Command::new(node)
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .is_some_and(|o| {
                    String::from_utf8_lossy(&o.stdout).starts_with(&format!("v{required}."))
                })
        })
        .with_context(|| {
            format!(
                "Node.js {required} is required; set DECKFORGE_NODE{required} to its executable"
            )
        })
}
fn launch_plugins(host: &mut Host, port: u16, state_dir: &Path) -> Result<()> {
    for plugin in &mut host.plugins {
        launch_plugin(plugin, &host.device, port, state_dir)?;
    }
    Ok(())
}
fn launch_plugin(plugin: &mut Plugin, device: &str, port: u16, state_dir: &Path) -> Result<()> {
    let code = plugin.manifest["CodePath"]
        .as_str()
        .context("Plugin has no generic CodePath; native Windows-only plugins are not supported")?;
    let file = safe_file(&plugin.root, code)?;
    if file.extension().and_then(|s| s.to_str()) != Some("js")
        && file.extension().and_then(|s| s.to_str()) != Some("mjs")
    {
        bail!(
            "{} is not a Node.js plugin; native/HTML runtimes are not yet implemented",
            plugin.id
        );
    }
    let required = plugin.manifest["Nodejs"]["Version"]
        .as_str()
        .context("Plugin does not declare a Nodejs.Version")?;
    let node = node_runtime(required)?;
    let info = json!({"application":{"font":"DejaVu Sans","language":"en","platform":"linux","platformVersion":std::env::consts::OS,"version":"7.6.0"},"plugin":{"uuid":plugin.id,"version":plugin.manifest["Version"]},"devicePixelRatio":1,"colors":{"buttonMouseOverBackgroundColor":"#303030","buttonPressedBackgroundColor":"#303030","buttonPressedBorderColor":"#505050","buttonPressedTextColor":"#ffffff","highlightColor":"#4bd3b0"},"devices":[{"id":device,"type":7,"name":"Stream Deck +","size":{"columns":4,"rows":2}}]});
    let log = File::create(state_dir.join(format!("plugin-{}.log", plugin.registration)))?;
    plugin.child = Some(
        Command::new(node)
            .arg(file)
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
) -> Result<()> {
    run(
        HostOptions {
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
    let HostOptions {
        serial,
        plugins,
        profile: profile_path,
        state_dir,
        port,
        simulate,
        seconds,
    } = options;
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
    let saved = if state_path.exists() {
        serde_json::from_slice::<Saved>(&fs::read(&state_path)?)
            .context("Invalid saved session; preserved without overwriting")?
    } else {
        Saved::default()
    };
    let plugin_paths = if plugins.is_empty() {
        saved.plugins.clone()
    } else {
        plugins
    };
    let (plugins, actions) = load_plugins(plugin_paths)?;
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<Result<InputEvent>>();
    let (draw_tx, draw_rx) = std::sync::mpsc::channel();
    let mut deck = if simulate {
        None
    } else {
        Some(Deck::connect(serial.as_deref())?)
    };
    let device = deck
        .as_ref()
        .map(|d| d.serial().to_owned())
        .unwrap_or_else(|| "simulated-streamdeck-plus".into());
    let firmware = deck
        .as_ref()
        .map(Deck::firmware_version)
        .transpose()?
        .unwrap_or_else(|| "simulation".into());
    let mut host = Host {
        port,
        plugins,
        actions,
        slots: saved.slots,
        globals: saved.globals,
        renderer: Renderer::new()?,
        hardware: if simulate {
            None
        } else {
            Some(draw_tx.clone())
        },
        device,
        firmware,
        token: uuid::Uuid::new_v4().to_string(),
        state_path,
        logs: Vec::new(),
        simulate,
        dial_pressed: [false; 4],
    };
    if let Some(path) = profile_path {
        host.import_profile(&path)?;
    } else if host.slots.is_empty() {
        let key = host
            .actions
            .iter()
            .find(|a| a.controllers.iter().any(|c| c == "Keypad"))
            .map(|a| a.uuid.clone());
        let dial = host
            .actions
            .iter()
            .find(|a| {
                a.controllers.iter().any(|c| c == "Encoder")
                    && a.raw["Encoder"]["layout"]
                        .as_str()
                        .is_none_or(|l| l == "$B1")
            })
            .map(|a| a.uuid.clone());
        if let Some(uuid) = key {
            host.assign("Keypad", 0, &uuid)?;
        }
        if let Some(uuid) = dial {
            host.assign("Encoder", 0, &uuid)?;
        }
    }
    let mut occupied = std::collections::HashSet::new();
    for slot in &mut host.slots {
        if !((slot.controller == "Keypad" && slot.index < 8)
            || (slot.controller == "Encoder" && slot.index < 4))
        {
            bail!("Saved session contains invalid slot coordinates");
        }
        if !occupied.insert((slot.controller.clone(), slot.index)) {
            bail!("Saved session has duplicate slot coordinates");
        }
        if slot.background.is_none()
            && let Some(action) = host.actions.iter().find(|a| a.uuid == slot.uuid)
            && let Some(plugin) = host.plugins.iter().find(|p| p.id == action.plugin)
        {
            slot.background = state_image(&plugin.root, &slot.states, slot.state);
        }
    }
    // Validate and start plugins before writing any hardware image.
    if let Err(error) = launch_plugins(&mut host, port, &state_dir) {
        stop_plugins(&mut host);
        return Err(error);
    }
    let worker = deck.take().map(|mut deck| {
        std::thread::spawn(move || {
            loop {
                // Coalesce plugin bursts to one latest image for each physical surface.
                let mut keys: HashMap<u8, DynamicImage> = HashMap::new();
                let mut strip = None;
                loop {
                    let draw = match draw_rx.try_recv() {
                        Ok(draw) => draw,
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    };
                    match draw {
                        Draw::Stop => return,
                        Draw::Key(i, img) => {
                            keys.insert(i, img);
                        }
                        Draw::Strip(img) => strip = Some(img),
                    }
                }
                let write: Result<()> = (|| {
                    for (i, img) in keys {
                        deck.set_key_image(i, &img)?;
                    }
                    if let Some(img) = strip {
                        deck.set_touchscreen_image(&img)?;
                    }
                    Ok(())
                })();
                if let Err(e) = write {
                    let _ = events_tx.send(Err(e));
                    return;
                }
                match deck.read_events(20) {
                    Ok(events) => {
                        for event in events {
                            let _ = events_tx.send(Ok(event));
                        }
                    }
                    Err(e) => {
                        let _ = events_tx.send(Err(e));
                        return;
                    }
                }
            }
        })
    });
    host.render()?;
    host.persist()?;
    host.log(format!(
        "Editor http://127.0.0.1:{port} | {} | firmware {}",
        host.device, host.firmware
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
            let _ = draw_tx.send(Draw::Stop);
            if let Some(worker) = worker {
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
            match event {
                Ok(e) => {
                    if let Err(e) = host.input(e) {
                        host.log(e.to_string());
                    }
                }
                Err(e) => {
                    host.hardware = None;
                    host.log(format!(
                        "USB disconnected/error: {e}. Restart host after reconnect."
                    ));
                }
            }
        }
    });
    let app = Router::new()
        .route("/", get(root))
        .route("/api/state", get(dashboard))
        .route("/api/control", post(control))
        .with_state(shared.clone());
    let stop = async move {
        let duration = seconds.map(Duration::from_secs).unwrap_or(Duration::MAX);
        tokio::select! {
            _=async {if let Some(stop)=embedded_stop.as_mut(){let _=stop.await;}else{std::future::pending::<()>().await;}}=>{},
            _=async{if seconds.is_some(){tokio::time::sleep(duration).await;}else{std::future::pending::<()>().await;}}=>{},
            _=tokio::signal::ctrl_c()=>{},
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
    {
        let mut host = shared.lock().unwrap();
        stop_plugins(&mut host);
        host.persist()?;
    }
    let _ = draw_tx.send(Draw::Stop);
    event_task.abort();
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    result?;
    Ok(())
}

fn stop_plugins(host: &mut Host) {
    for plugin in &mut host.plugins {
        if let Some(mut child) = plugin.child.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_host_has_direct_controller_and_releases_its_session_on_close() {
        let temp = tempfile::tempdir().unwrap();
        let options = HostOptions {
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
        assert!(
            running
                .controller
                .command(json!({"command":"input","event":"dialRotate","index":9,"ticks":1}))
                .is_err()
        );
        // A second owner must not overwrite this session, even on another port.
        assert!(start(options.clone()).is_err());
        drop(running);
        // Joining the embedded host releases its file lock before the next launch.
        let restarted = start(options).unwrap();
        assert_eq!(
            restarted.controller.snapshot()["device"],
            "simulated-streamdeck-plus"
        );
        drop(restarted);
    }
    fn fixture(root: &Path) -> Host {
        Host {
            port: 0,
            plugins: vec![Plugin {
                id: "test.plugin".into(),
                name: "Test".into(),
                root: root.to_owned(),
                manifest: json!({}),
                registration: "secret".into(),
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
                states: json!([{}, {}]),
                background: None,
                preview: String::new(),
            }],
            globals: HashMap::new(),
            renderer: Renderer::new().unwrap(),
            hardware: None,
            device: "test-device".into(),
            firmware: "test".into(),
            token: "test".into(),
            state_path: root.join("state.json"),
            logs: Vec::new(),
            simulate: true,
            dial_pressed: [false; 4],
        }
    }
    #[test]
    fn inactive_state_and_software_target_cannot_change_hardware_title() {
        let temp = tempfile::tempdir().unwrap();
        let mut host = fixture(temp.path());
        for payload in [
            json!({"title":"bad","state":1}),
            json!({"title":"bad","target":2}),
        ] {
            assert!(
                host.plugin_message(
                    0,
                    json!({"event":"setTitle","context":"test.context","payload":payload})
                )
                .is_err()
            );
            assert_eq!(host.slots[0].title, "before");
        }
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
}
