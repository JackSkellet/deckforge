//! Native WebKit editor shell. Devices, profiles and plugin policy stay in Host.
#[path = "desktop_canvas.rs"]
mod canvas;
use crate::{host::Controller, instance::Command, window::Window};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::run_return::EventLoopExtRunReturn,
    window::{Theme, WindowBuilder},
};
use wry::{PageLoadEvent, WebViewBuilder};

const MAX_IPC: usize = 1024 * 1024;
const POLL: Duration = Duration::from_millis(200);
const BRIDGE: &str = r#"(() => {
if (window !== window.top || window.deckforge) return;
let next=0,lastSnapshot=null;const pending=new Map(),listeners=new Set();
Object.defineProperty(window,'__deckforgeReceive',{value:message=>{
 const item=pending.get(message.id);if(!item)return;pending.delete(message.id);clearTimeout(item.timer);
 if(message.error!==undefined)item.reject(new Error(message.error));else item.resolve(message.result);
}});
Object.defineProperty(window,'__deckforgeSnapshot',{value:snapshot=>{
 lastSnapshot=snapshot;for(const listener of listeners){try{listener(snapshot)}catch(error){console.error(error)}}
}});
const api=Object.freeze({
 request(method,payload={}){
  if(pending.size>=128)return Promise.reject(new Error('Too many pending editor requests'));
  const id=++next;return new Promise((resolve,reject)=>{
   const timer=setTimeout(()=>{pending.delete(id);reject(new Error('Editor request timed out'))},300000);
   pending.set(id,{resolve,reject,timer});
   try{const body=JSON.stringify({id,method,payload});if(new TextEncoder().encode(body).length>1048576)throw new Error('Editor request exceeds 1 MiB');window.ipc.postMessage(body)}catch(error){clearTimeout(timer);pending.delete(id);reject(error)}
  });
 },
 onSnapshot(listener){listeners.add(listener);if(lastSnapshot!==null)listener(lastSnapshot);return()=>listeners.delete(listener)}
});
Object.defineProperty(window,'deckforge',{value:api});
window.addEventListener('deckforge-ready',()=>api.request('ui_ready').catch(console.error));
})();"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: Value,
    method: String,
    #[serde(default)]
    payload: Value,
}
#[derive(Clone)]
enum UiEvent {
    Wake,
    Snapshot,
    Reply { generation: u64, script: String },
    Loaded(u64),
    Ready(u64),
    TestDone { generation: u64, result: Value },
    CaptureDone(std::result::Result<(), String>),
}
struct Latest {
    snapshot: Value,
    script: String,
}
struct Shared {
    latest: Mutex<Latest>,
    proxy: Mutex<Option<EventLoopProxy<UiEvent>>>,
    ui_state: Mutex<Value>,
    stop: AtomicBool,
    running: AtomicBool,
    refresh_queued: AtomicBool,
    snapshot_queued: AtomicBool,
    picker: AtomicBool,
    importing_reference: AtomicBool,
}
impl Shared {
    fn notify(&self, event: UiEvent) {
        if let Some(proxy) = self.proxy.lock().unwrap().as_ref() {
            let _ = proxy.send_event(event);
        }
    }
}
enum Job {
    Request { generation: u64, request: Request },
    Refresh,
}
struct Worker {
    sender: mpsc::SyncSender<Job>,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.sender.try_send(Job::Refresh);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Worker {
    fn start(controller: Controller, state: &Path, window: Window) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(64);
        let shared = Arc::new(Shared {
            latest: Mutex::new(Latest {
                snapshot: Value::Null,
                script: String::new(),
            }),
            proxy: Mutex::new(None),
            ui_state: Mutex::new(canvas::load(state)),
            stop: AtomicBool::new(false),
            running: AtomicBool::new(true),
            refresh_queued: AtomicBool::new(false),
            snapshot_queued: AtomicBool::new(false),
            picker: AtomicBool::new(false),
            importing_reference: AtomicBool::new(false),
        });
        let redraw = shared.clone();
        let redraw_sender = sender.clone();
        controller.on_redraw(move || {
            if !redraw.refresh_queued.swap(true, Ordering::AcqRel)
                && redraw_sender.try_send(Job::Refresh).is_err()
            {
                redraw.refresh_queued.store(false, Ordering::Release);
            }
        });
        let work = shared.clone();
        let state = state.to_owned();
        let thread = std::thread::Builder::new()
            .name("desktop-bridge".into())
            .spawn(move || {
                refresh(&controller, &work, &window);
                while !work.stop.load(Ordering::Acquire) {
                    match receiver.recv_timeout(POLL) {
                        Ok(Job::Request {
                            generation,
                            request,
                        }) => {
                            if request.method == "pick_file" {
                                if work.picker.swap(true, Ordering::AcqRel) {
                                    reply(
                                        &work,
                                        generation,
                                        request.id,
                                        Err(anyhow::anyhow!("A file dialog is already open")),
                                    );
                                    continue;
                                }
                                let picker = work.clone();
                                std::thread::spawn(move || {
                                    let result = pick_file(&request.payload);
                                    picker.picker.store(false, Ordering::Release);
                                    reply(&picker, generation, request.id, result);
                                });
                            } else if matches!(
                                request.method.as_str(),
                                "reference_import"
                                    | "plugin_update_prepare"
                                    | "plugin_rollback_prepare"
                            ) {
                                if work.importing_reference.swap(true, Ordering::AcqRel) {
                                    reply(
                                        &work,
                                        generation,
                                        request.id,
                                        Err(anyhow::anyhow!(
                                            "An installer or package is already being inspected"
                                        )),
                                    );
                                    continue;
                                }
                                let importer = work.clone();
                                let importer_controller = controller.clone();
                                let request_id = request.id.clone();
                                let spawned = std::thread::Builder::new()
                                    .name("reference-import".into())
                                    .spawn(move || {
                                        let result = if request.method == "reference_import" {
                                            import_reference(&importer_controller, request.payload)
                                        } else {
                                            importer_controller.prepare_plugin_update(
                                                request.payload,
                                                request.method == "plugin_rollback_prepare",
                                            )
                                        };
                                        importer
                                            .importing_reference
                                            .store(false, Ordering::Release);
                                        reply(&importer, generation, request.id, result);
                                    });
                                if let Err(error) = spawned {
                                    work.importing_reference.store(false, Ordering::Release);
                                    reply(&work, generation, request_id, Err(error.into()));
                                }
                            } else {
                                let result = dispatch(
                                    &controller,
                                    &state,
                                    &window,
                                    &work,
                                    &request.method,
                                    request.payload,
                                );
                                reply(&work, generation, request.id, result);
                                if request.method == "ui_ready" {
                                    work.notify(UiEvent::Ready(generation));
                                }
                            }
                        }
                        Ok(Job::Refresh) => {
                            work.refresh_queued.store(false, Ordering::Release);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    refresh(&controller, &work, &window);
                }
            })?;
        Ok(Self {
            sender,
            shared,
            thread: Some(thread),
        })
    }
}
fn refresh(controller: &Controller, shared: &Shared, window: &Window) {
    let running = controller.is_running();
    shared.running.store(running, Ordering::Release);
    if !running {
        window.request(Command::Quit);
        return;
    }
    let snapshot = controller.snapshot();
    let mut latest = shared.latest.lock().unwrap();
    if latest.snapshot != snapshot {
        latest.script = format!("window.__deckforgeSnapshot({});", js_json(&snapshot));
        latest.snapshot = snapshot;
        drop(latest);
        if !shared.snapshot_queued.swap(true, Ordering::AcqRel) {
            shared.notify(UiEvent::Snapshot);
        }
    }
}
fn reply(shared: &Shared, generation: u64, id: Value, result: Result<Value>) {
    let response = match result {
        Ok(result) => json!({"id":id,"result":result}),
        Err(error) => json!({"id":id,"error":error.to_string()}),
    };
    shared.notify(UiEvent::Reply {
        generation,
        script: format!("window.__deckforgeReceive({});", js_json(&response)),
    });
}
fn js_json(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}
fn trusted_document(uri: &str) -> bool {
    uri == "about:blank" || uri.starts_with("about:blank#")
}
fn parse_request(origin: &str, body: &str) -> Result<Request> {
    ensure!(
        trusted_document(origin),
        "Editor request came from an untrusted document"
    );
    ensure!(body.len() <= MAX_IPC, "Editor request exceeds 1 MiB");
    let request: Request = serde_json::from_str(body).context("Invalid editor request")?;
    ensure!(
        request.id.as_u64().is_some() || request.id.as_str().is_some_and(|s| s.len() <= 128),
        "Invalid editor request ID"
    );
    ensure!(request.method.len() <= 64, "Invalid editor method");
    Ok(request)
}
fn import_reference(controller: &Controller, payload: Value) -> Result<Value> {
    let path = payload["path"].as_str().context("Installer path missing")?;
    let expected: Option<PathBuf> = serde_json::from_value(payload["expected"].clone())
        .context("Previous reference selection missing")?;
    Ok(serde_json::to_value(
        controller.import_reference(Path::new(path), expected.as_deref())?,
    )?)
}

fn dispatch(
    controller: &Controller,
    state: &Path,
    window: &Window,
    shared: &Shared,
    method: &str,
    payload: Value,
) -> Result<Value> {
    match method {
        "snapshot" => Ok(shared.latest.lock().unwrap().snapshot.clone()),
        "fonts" => Ok(serde_json::to_value(crate::appearance::font_families()?)?),
        "marketplace" => {
            controller.marketplace_request(
                payload["action"]
                    .as_str()
                    .context("Marketplace action missing")?,
                None,
            )?;
            Ok(Value::Null)
        }
        "appearance_preview" => controller.appearance_preview(payload),
        "mixer_control" => controller.mixer_control(payload),
        "command" => {
            ensure!(payload.is_object(), "Host command must be an object");
            controller.command(payload)?;
            Ok(Value::Null)
        }
        "action_icon" => {
            let uuid = payload["uuid"].as_str().context("Missing action UUID")?;
            ensure!(uuid.len() <= 512, "Invalid action UUID");
            let generation = payload
                .get("generation")
                .filter(|value| !value.is_null())
                .map(|value| value.as_u64().context("Invalid action generation"))
                .transpose()?;
            Ok(controller
                .action_icon(uuid, generation)?
                .map_or(Value::Null, Value::String))
        }
        "ui_state_get" => Ok(shared.ui_state.lock().unwrap().clone()),
        "ui_state_set" => {
            let value = payload.get("state").context("Missing editor state")?;
            ensure!(
                serde_json::to_vec(value)?.len() <= MAX_IPC,
                "Editor drafts exceed 1 MiB"
            );
            let mut ui_state = shared.ui_state.lock().unwrap();
            canvas::save(state, &ui_state, value).context("Save audio canvas layout")?;
            *ui_state = value.clone();
            Ok(Value::Null)
        }
        "window" => {
            let command = match payload["action"].as_str() {
                Some("hide") => Command::Hide,
                Some("quit") => Command::Quit,
                _ => bail!("Unknown window action"),
            };
            window.request(command);
            Ok(Value::Null)
        }
        "startup_status" | "startup_set" => {
            let simulate = shared.latest.lock().unwrap().snapshot["simulate"] == true;
            let startup = crate::startup::Startup::current(state, simulate)?;
            let status = if method == "startup_set" {
                startup.set_enabled(
                    payload["enabled"]
                        .as_bool()
                        .context("Missing startup enabled flag")?,
                )?
            } else {
                startup.status()?
            };
            Ok(serde_json::to_value(status)?)
        }
        "open_url" => {
            let url = external_url(&payload)?;
            crate::external_url::open(&url)?;
            Ok(Value::Null)
        }
        "ui_ready" => Ok(Value::Null),
        _ => bail!("Unknown editor method: {method}"),
    }
}
fn external_url(payload: &Value) -> Result<String> {
    crate::external_url::validate(payload["url"].as_str().context("Missing link URL")?)
}

fn pick_file(payload: &Value) -> Result<Value> {
    let dialog = rfd::FileDialog::new();
    let path = match payload["kind"]
        .as_str()
        .context("Missing file dialog kind")?
    {
        "reference_installer" => dialog
            .add_filter("Stream Deck macOS installer", &["pkg"])
            .pick_file(),
        "plugin" => dialog
            .add_filter("Stream Deck plugin", &["streamDeckPlugin"])
            .pick_file(),
        "profile" => dialog
            .add_filter("Stream Deck profile", &["streamDeckProfile"])
            .pick_file(),
        "image" => dialog
            .add_filter("Image", &["png", "jpg", "jpeg", "gif", "svg", "webp"])
            .pick_file(),
        "audio" => dialog
            .add_filter("Audio", &["wav", "mp3", "ogg", "flac", "m4a"])
            .pick_file(),
        "directory" => dialog.pick_folder(),
        "export_profile" => dialog
            .add_filter("Stream Deck profile", &["streamDeckProfile"])
            .set_file_name("Profile.streamDeckProfile")
            .save_file(),
        "file" => dialog.pick_file(),
        _ => bail!("Unknown file dialog kind"),
    };
    path.map(|path| {
        path.to_str()
            .map(str::to_owned)
            .context("Selected path is not UTF-8")
    })
    .transpose()
    .map(|path| path.map_or(Value::Null, Value::String))
}
fn protected_html(html: &str) -> String {
    let policy = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'">"#;
    if let Some(index) = html.find("<head>") {
        let at = index + 6;
        format!("{}{}{}", &html[..at], policy, &html[at..])
    } else {
        format!("{policy}{html}")
    }
}

/// Runs on the process main thread. HTML is embedded by the caller; no editor
/// HTTP listener or external browser is created. Background start creates no GUI.
pub fn run(controller: Controller, state: &Path, background: bool, html: &str) -> Result<()> {
    let window = Window::new(!background);
    let endpoint = window.clone();
    let link_window = window.clone();
    let link_controller = controller.clone();
    let _instance = crate::instance::spawn_with_deep_links(
        state,
        Arc::new(move |command| Ok(endpoint.request(command))),
        Arc::new(move |uri| {
            let link = crate::deep_link::DeepLink::parse(uri)?;
            let delivery = link_controller.deliver_deep_link(uri);
            if !link.hidden {
                link_window.request(Command::Show);
            }
            delivery?;
            Ok(serde_json::json!({"accepted": true}))
        }),
    )?;
    let tray_window = window.clone();
    let _tray = match crate::tray::spawn(Arc::new(move |event| tray_window.tray(event))) {
        Ok(tray) => Some(tray),
        Err(error) => {
            window.tray(crate::tray::Event::Error(error.to_string()));
            None
        }
    };
    let worker = Worker::start(controller, state, window.clone())?;
    let capture = std::env::var_os("DECKFORGE_CAPTURE").map(PathBuf::from);
    ensure!(
        !background || capture.is_none(),
        "A screenshot requires a visible editor"
    );
    let test_script = std::env::var_os("DECKFORGE_WEBVIEW_TEST_SCRIPT")
        .map(PathBuf::from)
        .map(fs::read_to_string)
        .transpose()
        .context("Read editor fixture script")?;
    ensure!(
        test_script.as_ref().is_none_or(|s| s.len() <= MAX_IPC),
        "Editor fixture script exceeds 1 MiB"
    );
    let test_result = std::env::var_os("DECKFORGE_WEBVIEW_TEST_RESULT").map(PathBuf::from);
    ensure!(
        test_script.is_none() || test_result.is_some(),
        "Editor fixture script requires a result path"
    );
    let mut event_loop = None;
    let mut capture_result = None;
    let html = protected_html(html);
    eprintln!("Native background host ready");
    while worker.shared.running.load(Ordering::Acquire) && !window.quitting() {
        if !window.wait() {
            continue;
        }
        if event_loop.is_none() {
            event_loop = Some(EventLoopBuilder::<UiEvent>::with_user_event().build());
        }
        let event_loop = event_loop.as_mut().unwrap();
        let proxy = event_loop.create_proxy();
        *worker.shared.proxy.lock().unwrap() = Some(proxy.clone());
        let native = WindowBuilder::new()
            .with_title("Deckforge · Stream Deck+")
            .with_theme(Some(Theme::Dark))
            .with_inner_size(LogicalSize::new(1180.0, 850.0))
            .with_min_inner_size(LogicalSize::new(760.0, 620.0))
            .build(event_loop)
            .context("Create native editor window")?;
        let generation = window.request(Command::Status)["editor_generation"]
            .as_u64()
            .unwrap_or(0)
            + 1;
        let sender = worker.sender.clone();
        let ipc_proxy = proxy.clone();
        let load_proxy = proxy.clone();
        let testing = test_script.is_some();
        let builder = WebViewBuilder::new()
            .with_html(&html)
            .with_background_color((19, 23, 29, 255))
            .with_initialization_script(BRIDGE)
            .with_navigation_handler(|uri| trusted_document(&uri))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_ipc_handler(move |request| {
                match parse_request(&request.uri().to_string(), request.body()) {
                    Ok(request) if testing && request.method == "_test_result" => {
                        let _ = ipc_proxy.send_event(UiEvent::TestDone {
                            generation,
                            result: request.payload,
                        });
                    }
                    Ok(request) => {
                        let id = request.id.clone();
                        if sender
                            .try_send(Job::Request {
                                generation,
                                request,
                            })
                            .is_err()
                        {
                            let _ = ipc_proxy.send_event(UiEvent::Reply {
                                generation,
                                script: format!(
                                    "window.__deckforgeReceive({});",
                                    js_json(
                                        &json!({"id":id,"error":"Editor worker is busy; try again"})
                                    )
                                ),
                            });
                        }
                    }
                    Err(error) => {
                        eprintln!("Editor bridge rejected request: {error}");
                    }
                }
            })
            .with_on_page_load_handler(move |event, url| {
                if matches!(event, PageLoadEvent::Finished) && trusted_document(&url) {
                    let _ = load_proxy.send_event(UiEvent::Loaded(generation));
                }
            });
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let webview = builder
            .build_gtk(
                native
                    .default_vbox()
                    .context("Native editor has no GTK container")?,
            )
            .context("Create native editor WebKit view")?;
        let wake_proxy = proxy.clone();
        window.attach_wakeup(Arc::new(move || {
            let _ = wake_proxy.send_event(UiEvent::Wake);
        }));
        let mut loaded = false;
        let mut ready = false;
        let mut tested = false;
        let mut capture_at = None;
        let mut capturing = false;
        let started = Instant::now();
        eprintln!("Native editor ready");
        let exit=event_loop.run_return(|event,_,flow|{
            *flow=ControlFlow::WaitUntil(Instant::now()+POLL);
            match event {
                Event::UserEvent(UiEvent::Wake)=>{},
                Event::UserEvent(UiEvent::Snapshot)=>{
                    worker.shared.snapshot_queued.store(false,Ordering::Release);
                    if loaded {let script=worker.shared.latest.lock().unwrap().script.clone();let _=webview.evaluate_script(&script);}
                }
                Event::UserEvent(UiEvent::Reply{generation:reply_generation,script}) if reply_generation==generation=>{let _=webview.evaluate_script(&script);}
                Event::UserEvent(UiEvent::Loaded(current)) if current==generation=>{
                    loaded=true;worker.shared.snapshot_queued.store(false,Ordering::Release);
                    let script=worker.shared.latest.lock().unwrap().script.clone();let _=webview.evaluate_script(&script);
                }
                Event::UserEvent(UiEvent::Ready(current)) if current==generation&&!ready=>{
                    ready=true;
                    if let Some(script)=&test_script {
                        // Explicit local fixture code runs through native evaluation;
                        // the application CSP still forbids JavaScript eval.
                        let wrapper=format!("(async()=>{{\n{script}\n}})().then(result=>window.deckforge.request('_test_result',{{ok:true,result}}),error=>window.deckforge.request('_test_result',{{ok:false,error:String(error)}}));");
                        let _=webview.evaluate_script(&wrapper);
                    }else{tested=true;capture_at=Some(Instant::now()+Duration::from_millis(500));}
                }
                Event::UserEvent(UiEvent::TestDone{generation:current,result}) if current==generation=>{
                    tested=true;capture_at=Some(Instant::now()+Duration::from_millis(500));
                    if let Some(path)=test_result.clone(){std::thread::spawn(move||{if let Err(error)=fs::write(path,serde_json::to_vec_pretty(&result).unwrap()){eprintln!("Editor fixture result: {error}");}});}
                }
                Event::UserEvent(UiEvent::CaptureDone(result))=>{capture_result=Some(result);window.request(Command::Quit);}
                Event::WindowEvent{window_id,event:WindowEvent::CloseRequested,..} if window_id==native.id()=>{window.request(Command::Hide);}
                _=>{}
            }
            if capture.is_some()&&loaded&&ready&&tested&&!capturing&&capture_at.is_some_and(|at|Instant::now()>=at) {
                capturing=true;capture_webkit(&webview,capture.clone().unwrap(),proxy.clone());
            }
            if capture.is_some()&&started.elapsed()>Duration::from_secs(20)&&capture_result.is_none(){capture_result=Some(Err("Native WebKit screenshot timed out before the editor rendered".into()));window.request(Command::Quit);}
            let (keep_open,focus)=window.presentation();
            if !keep_open||!worker.shared.running.load(Ordering::Acquire){*flow=ControlFlow::Exit;}
            else if focus{native.set_minimized(false);native.set_focus();}
        });
        *worker.shared.proxy.lock().unwrap() = None;
        drop(webview);
        drop(native);
        window.detach();
        ensure!(
            exit == 0,
            "Native editor event loop exited with code {exit}"
        );
    }
    if capture.is_some() {
        capture_result
            .context("Editor closed before screenshot completed")?
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
fn capture_webkit(view: &wry::WebView, path: PathBuf, proxy: EventLoopProxy<UiEvent>) {
    use webkit2gtk::WebViewExt;
    use wry::WebViewExtUnix;
    view.webview().snapshot(
        webkit2gtk::SnapshotRegion::Visible,
        webkit2gtk::SnapshotOptions::empty(),
        None::<&webkit2gtk::gio::Cancellable>,
        move |result| {
            let result = result
                .map_err(|error| error.to_string())
                .and_then(|surface| {
                    let mut bytes = Vec::new();
                    surface
                        .write_to_png(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    Ok(bytes)
                });
            std::thread::spawn(move || {
                let result = result
                    .and_then(|bytes| fs::write(path, bytes).map_err(|error| error.to_string()));
                let _ = proxy.send_event(UiEvent::CaptureDone(result));
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_rejects_remote_documents_large_bodies_and_invalid_ids() {
        let body = r#"{"id":1,"method":"snapshot","payload":{}}"#;
        assert!(parse_request("about:blank", body).is_ok());
        assert!(parse_request("https://example.com", body).is_err());
        assert!(parse_request("about:blank.attacker", body).is_err());
        assert!(parse_request("about:blank", &"x".repeat(MAX_IPC + 1)).is_err());
        assert!(parse_request("about:blank", r#"{"id":{},"method":"snapshot"}"#).is_err());
    }
    #[test]
    fn external_links_are_not_shell_commands_or_local_files() {
        assert_eq!(
            external_url(&json!({"url":"https://docs.elgato.com/streamdeck/"})).unwrap(),
            "https://docs.elgato.com/streamdeck/"
        );
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,test",
            "https:\n",
        ] {
            assert!(external_url(&json!({"url":url})).is_err());
        }
    }
    #[test]
    fn replies_keep_source_looking_settings_as_data() {
        let value = json!({"id":7,"result":"\";window.close();\u{2028}x"});
        let encoded = js_json(&value);
        assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), value);
        assert!(encoded.contains("\\u2028"));
    }
    #[test]
    fn editor_policy_blocks_remote_code_frames_and_connections() {
        let html = protected_html("<!doctype html><html><head></head><body>Editor</body></html>");
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("frame-src 'none'"));
        assert!(html.find("Content-Security-Policy").unwrap() < html.find("</head>").unwrap());
    }
}
