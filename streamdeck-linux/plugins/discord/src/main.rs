//! Independent native Discord plugin. No Discord connection exists before Connect.
mod actions;
mod auth;
mod config;
mod ipc;

use actions::{ActionEngine, Effect};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message;

struct Options {
    host: String,
    uuid: String,
    registration: String,
    config: PathBuf,
    ipc: Vec<PathBuf>,
    oauth: String,
}
struct Pending {
    cmd: String,
    context: Option<String>,
    since: Instant,
}
struct OAuthResult {
    generation: u64,
    result: Result<config::Token>,
}
struct AbortTask(JoinHandle<()>);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Runtime {
    engine: ActionEngine,
    saved: Option<config::Config>,
    config_path: PathBuf,
    paths: Vec<PathBuf>,
    oauth_endpoint: String,
    http: reqwest::Client,
    outgoing: mpsc::Sender<Value>,
    rpc: Option<mpsc::Sender<ipc::Command>>,
    events: mpsc::Receiver<ipc::Event>,
    task: Option<AbortTask>,
    oauth_results: mpsc::Receiver<OAuthResult>,
    oauth_sender: mpsc::Sender<OAuthResult>,
    oauth_task: Option<AbortTask>,
    generation: u64,
    pending: HashMap<String, Pending>,
    inspectors: HashMap<String, String>,
    actions: HashMap<String, String>,
    connected: bool,
    authenticated: bool,
    message: String,
    voice_channel: Option<String>,
}
impl Runtime {
    fn new(options: &Options, outgoing: mpsc::Sender<Value>) -> Result<Self> {
        let (saved, message)=match config::load(&options.config) {Ok(c)=>(c,"Not connected. Configure your Discord application, then choose Connect.".to_owned()),Err(_)=>(None,"Private Discord configuration could not be read. Check its ownership and permissions.".to_owned())};
        let (_, events) = mpsc::channel(1);
        let (oauth_sender, oauth_results) = mpsc::channel(8);
        Ok(Self {
            engine: ActionEngine::new(),
            saved,
            config_path: options.config.clone(),
            paths: options.ipc.clone(),
            oauth_endpoint: options.oauth.clone(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            outgoing,
            rpc: None,
            events,
            task: None,
            oauth_results,
            oauth_sender,
            oauth_task: None,
            generation: 0,
            pending: HashMap::new(),
            inspectors: HashMap::new(),
            actions: HashMap::new(),
            connected: false,
            authenticated: false,
            message,
            voice_channel: None,
        })
    }
    async fn host(&self, value: Value) -> Result<()> {
        self.outgoing
            .send(value)
            .await
            .context("Host connection closed")
    }
    async fn status(&self) -> Result<()> {
        for (context, action) in &self.inspectors {
            self.host(json!({"event":"sendToPropertyInspector","context":context,"action":action,"payload":{"auth":{"configured":self.saved.is_some(),"connected":self.connected,"authenticated":self.authenticated,"message":self.message}}})).await?;
        }
        Ok(())
    }
    async fn effects(&mut self, effects: Vec<Effect>) -> Result<()> {
        for effect in effects {
            match effect {
                Effect::Host(value) => self.host(value).await?,
                Effect::Rpc { cmd, args, context } => {
                    if self.authenticated {
                        self.request(&cmd, args, context, None).await?;
                    }
                }
            }
        }
        Ok(())
    }
    async fn request(
        &mut self,
        cmd: &str,
        args: Value,
        context: Option<String>,
        evt: Option<&str>,
    ) -> Result<()> {
        let Some(sender) = self.rpc.as_ref() else {
            return Ok(());
        };
        self.pending
            .retain(|_, p| p.since.elapsed() < Duration::from_secs(600));
        ensure!(
            self.pending.len() < 512,
            "Too many outstanding Discord RPC requests"
        );
        let nonce = uuid::Uuid::new_v4().to_string();
        let mut frame = json!({"cmd":cmd,"args":args,"nonce":nonce});
        if let Some(evt) = evt {
            frame["evt"] = json!(evt);
        }
        tokio::time::timeout(
            Duration::from_secs(1),
            sender.send(ipc::Command::Frame(frame)),
        )
        .await
        .context("Discord command queue is busy")?
        .context("Discord transport stopped")?;
        self.pending.insert(
            nonce,
            Pending {
                cmd: cmd.into(),
                context,
                since: Instant::now(),
            },
        );
        Ok(())
    }
    async fn stop(&mut self, message: &str) -> Result<()> {
        let effects = self.engine.connection(false);
        let cleanup_ok = matches!(
            tokio::time::timeout(Duration::from_millis(750), self.effects(effects)).await,
            Ok(Ok(()))
        );
        if let Some(sender) = self.rpc.as_ref() {
            let (done, receipt) = tokio::sync::oneshot::channel();
            let _ = tokio::time::timeout(Duration::from_millis(750), async {
                if sender.send(ipc::Command::Flush(done)).await.is_ok() {
                    let _ = receipt.await;
                }
            })
            .await;
        }
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
        self.oauth_task = None;
        self.rpc = None;
        self.pending.clear();
        self.connected = false;
        self.authenticated = false;
        self.voice_channel = None;
        self.message = if cleanup_ok {
            message.into()
        } else {
            format!("{message} Discord did not accept all held-control release requests.")
        };
        self.status().await
    }
    fn connect_config(&self, payload: &Value) -> Result<config::Config> {
        let get = |key: &str| {
            payload
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        let mut candidate = self.saved.clone().unwrap_or(config::Config {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: String::new(),
            scopes: vec![
                "rpc".into(),
                "identify".into(),
                "rpc.voice.read".into(),
                "rpc.voice.write".into(),
            ],
            token: None,
        });
        for key in ["client_id", "client_secret", "redirect_uri"] {
            let value = get(key);
            if !value.is_empty() {
                match key {
                    "client_id" => candidate.client_id = value,
                    "client_secret" => candidate.client_secret = value,
                    _ => candidate.redirect_uri = value,
                }
            }
        }
        if let Some(scopes) = payload.get("scopes") {
            candidate.scopes = if let Some(array) = scopes.as_array() {
                array
                    .iter()
                    .map(|s| s.as_str().unwrap_or("").to_owned())
                    .collect()
            } else if let Some(text) = scopes.as_str() {
                text.split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            } else {
                bail!("Scopes must be text or an array")
            };
        }
        config::validate(&candidate)?;
        if self.saved.as_ref().is_none_or(|old| {
            old.client_id != candidate.client_id
                || old.client_secret != candidate.client_secret
                || old.scopes != candidate.scopes
                || old.redirect_uri != candidate.redirect_uri
        }) {
            candidate.token = None;
        }
        Ok(candidate)
    }
    async fn on_host(&mut self, event: Value) -> Result<()> {
        let kind = event["event"].as_str().unwrap_or("");
        let context = event["context"].as_str().unwrap_or("");
        let action = event["action"].as_str().unwrap_or("");
        if kind == "willAppear" {
            self.actions.insert(context.into(), action.into());
        }
        if kind == "willDisappear" {
            self.actions.remove(context);
            self.inspectors.remove(context);
        }
        if kind == "propertyInspectorDidAppear" {
            self.inspectors.insert(context.into(), action.into());
        }
        if kind == "propertyInspectorDidDisappear" {
            self.inspectors.remove(context);
        }
        if kind == "sendToPlugin" {
            let command = event["payload"]["command"].as_str().unwrap_or("");
            if command == "status" {
                self.inspectors.insert(context.into(), action.into());
                let effects = self.engine.host_event(&event);
                self.effects(effects).await?;
                return self.status().await;
            }
            if ["connect", "disconnect", "forget"].contains(&command) {
                ensure!(
                    self.actions
                        .get(context)
                        .is_some_and(|a| a.starts_with("com.elgato.discord.")),
                    "Unknown Discord action context"
                );
                let result = match command {
                    "connect" => self.connect(event["payload"].clone()).await,
                    "disconnect" => {
                        self.stop("Disconnected. Choose Connect to reconnect.")
                            .await
                    }
                    _ => {
                        self.stop("Disconnected; removing saved credentials.")
                            .await?;
                        let result = config::forget(&self.config_path);
                        if result.is_ok() {
                            self.saved = None;
                            self.message = "Saved Discord credentials removed.".into();
                        }
                        result
                    }
                };
                if let Err(error) = result {
                    self.message = error.to_string();
                }
                return self.status().await;
            }
        }
        let effects = self.engine.host_event(&event);
        self.effects(effects).await?;
        if kind == "propertyInspectorDidAppear" {
            self.status().await?;
        }
        Ok(())
    }
    async fn connect(&mut self, payload: Value) -> Result<()> {
        let config = self.connect_config(&payload)?;
        config::save(&self.config_path, &config)?;
        self.stop("Connecting to Discord desktop…").await?;
        self.saved = Some(config.clone());
        let (sender, commands) = mpsc::channel(64);
        let (events, received) = mpsc::channel(64);
        self.events = received;
        self.rpc = Some(sender);
        self.task = Some(AbortTask(tokio::spawn(ipc::supervise(
            self.paths.clone(),
            config.client_id,
            commands,
            events,
        ))));
        self.status().await
    }
    fn exchange(&mut self, code: String, refresh: bool) -> Result<()> {
        let config = self
            .saved
            .clone()
            .context("Discord application not configured")?;
        let endpoint = self.oauth_endpoint.clone();
        let client = self.http.clone();
        let tx = self.oauth_sender.clone();
        let generation = self.generation;
        self.oauth_task = Some(AbortTask(tokio::spawn(async move {
            let result = auth::exchange(&client, &endpoint, &config, &code, refresh).await;
            let _ = tx.send(OAuthResult { generation, result }).await;
        })));
        Ok(())
    }
    async fn ready(&mut self) -> Result<()> {
        let saved = self
            .saved
            .clone()
            .context("Discord application not configured")?;
        if let Some(token) = saved.token {
            if token.expires_at > config::now() + 60 {
                self.message = "Authenticating saved Discord session…".into();
                self.request(
                    "AUTHENTICATE",
                    json!({"access_token":token.access_token}),
                    None,
                    None,
                )
                .await?;
            } else if !token.refresh_token.is_empty() {
                self.message = "Refreshing Discord authorization…".into();
                self.exchange(token.refresh_token, true)?;
            } else {
                self.authorize().await?;
            }
        } else {
            self.authorize().await?;
        }
        self.status().await
    }
    async fn authorize(&mut self) -> Result<()> {
        let saved = self
            .saved
            .as_ref()
            .context("Discord application not configured")?;
        let args = json!({"client_id":saved.client_id,"scopes":saved.scopes});
        self.message =
            "Approve this application in Discord. RPC scopes may require Discord approval.".into();
        self.request("AUTHORIZE", args, None, None).await
    }
    async fn subscriptions(&mut self, channel: Option<String>) -> Result<()> {
        if self.voice_channel == channel {
            return Ok(());
        }
        let old = self.voice_channel.take();
        for (command, id) in [("UNSUBSCRIBE", old), ("SUBSCRIBE", channel.clone())] {
            if let Some(id) = id {
                for event in [
                    "VOICE_STATE_CREATE",
                    "VOICE_STATE_UPDATE",
                    "VOICE_STATE_DELETE",
                    "SPEAKING_START",
                    "SPEAKING_STOP",
                ] {
                    self.request(command, json!({"channel_id":id}), None, Some(event))
                        .await?;
                }
            }
        }
        self.voice_channel = channel;
        Ok(())
    }
    async fn on_rpc(&mut self, event: ipc::Event) -> Result<()> {
        match event {
            ipc::Event::Connected => {
                self.connected = true;
                self.message = "Discord IPC connected; awaiting ready event…".into();
                self.status().await?;
            }
            ipc::Event::Offline(message) => {
                self.generation = self.generation.wrapping_add(1);
                self.oauth_task = None;
                self.pending.clear();
                self.connected = false;
                self.authenticated = false;
                self.voice_channel = None;
                self.message = message;
                let effects = self.engine.connection(false);
                self.effects(effects).await?;
                self.status().await?;
            }
            ipc::Event::Frame(mut frame) => {
                let cmd = frame["cmd"].as_str().unwrap_or("").to_owned();
                let evt = frame["evt"].as_str().unwrap_or("").to_owned();
                if evt == "READY" {
                    self.ready().await?;
                    return Ok(());
                }
                if let Some(nonce) = frame["nonce"].as_str() {
                    let Some(pending) = self.pending.remove(nonce) else {
                        return Ok(());
                    };
                    ensure!(cmd == pending.cmd, "Discord RPC response command mismatch");
                    if let Some(context) = pending.context {
                        frame["_context"] = json!(context);
                    }
                } else if cmd != "DISPATCH" {
                    return Ok(());
                }
                if evt == "ERROR" && ["AUTHORIZE", "AUTHENTICATE"].contains(&cmd.as_str()) {
                    self.authenticated = false;
                    self.message = format!(
                        "Discord authorization failed (code {}). Check application approval/scopes, then Connect again.",
                        frame["data"]["code"].as_i64().unwrap_or(0)
                    );
                    if cmd == "AUTHENTICATE"
                        && let Some(config) = self.saved.as_mut()
                    {
                        config.token = None;
                        config::save(&self.config_path, config)?;
                    }
                    let effects = self.engine.connection(false);
                    self.effects(effects).await?;
                    self.status().await?;
                    return Ok(());
                }
                if cmd == "AUTHORIZE" {
                    let code = frame["data"]["code"]
                        .as_str()
                        .context("Discord did not return an authorization code")?;
                    self.message = "Exchanging user-approved Discord authorization…".into();
                    self.exchange(code.into(), false)?;
                    self.status().await?;
                    return Ok(());
                }
                if cmd == "AUTHENTICATE" {
                    self.authenticated = true;
                    self.message = "Connected to Discord.".into();
                    for event in [
                        "VOICE_SETTINGS_UPDATE",
                        "VOICE_CHANNEL_SELECT",
                        "GUILD_CREATE",
                        "CHANNEL_CREATE",
                    ] {
                        self.request("SUBSCRIBE", json!({}), None, Some(event))
                            .await?;
                    }
                    if self
                        .saved
                        .as_ref()
                        .is_some_and(|c| c.scopes.iter().any(|s| s == "rpc.notifications.read"))
                    {
                        self.request("SUBSCRIBE", json!({}), None, Some("NOTIFICATION_CREATE"))
                            .await?;
                    }
                    let effects = self.engine.connection(true);
                    self.effects(effects).await?;
                    self.status().await?;
                }
                if self.authenticated {
                    let channel = if cmd == "GET_SELECTED_VOICE_CHANNEL" {
                        Some(frame["data"]["id"].as_str().map(str::to_owned))
                    } else if evt == "VOICE_CHANNEL_SELECT" {
                        Some(frame["data"]["channel_id"].as_str().map(str::to_owned))
                    } else {
                        None
                    };
                    if let Some(channel) = channel {
                        self.subscriptions(channel).await?;
                    }
                    let effects = self.engine.rpc_event(&frame);
                    self.effects(effects).await?;
                }
            }
        }
        Ok(())
    }
    async fn on_oauth(&mut self, result: OAuthResult) -> Result<()> {
        if result.generation != self.generation || self.task.is_none() {
            return Ok(());
        }
        self.oauth_task = None;
        match result.result {
            Ok(token) => {
                let config = self.saved.as_mut().context("Discord config missing")?;
                config.token = Some(token.clone());
                config::save(&self.config_path, config)?;
                self.request(
                    "AUTHENTICATE",
                    json!({"access_token":token.access_token}),
                    None,
                    None,
                )
                .await?;
                self.message = "Authenticating Discord RPC…".into();
            }
            Err(error) => {
                self.message = error.to_string();
                self.authenticated = false;
                let effects = self.engine.connection(false);
                self.effects(effects).await?;
            }
        }
        self.status().await
    }
}

async fn run(options: Options) -> Result<()> {
    let (socket, _) = tokio_tungstenite::connect_async(&options.host)
        .await
        .context("Connect to Deckforge loopback WebSocket")?;
    let (mut writer, mut reader) = socket.split();
    writer
        .send(Message::Text(
            json!({"event":options.registration,"uuid":options.uuid})
                .to_string()
                .into(),
        ))
        .await?;
    let (outgoing, mut received) = mpsc::channel::<Value>(128);
    let _writer = AbortTask(tokio::spawn(async move {
        while let Some(value) = received.recv().await {
            if writer
                .send(Message::Text(value.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    }));
    let mut runtime = Runtime::new(&options, outgoing)?;
    loop {
        let result = tokio::select! {
            message=reader.next()=>{
                match message {Some(Ok(Message::Text(text)))=>{match serde_json::from_str::<Value>(&text){Ok(event)=>runtime.on_host(event).await,Err(_)=>Ok(())}},Some(Ok(Message::Close(_)))|None=>break,Some(Err(error))=>return Err(error.into()),_=>Ok(())}
            }
            event=runtime.events.recv(), if runtime.task.is_some()=>{match event{Some(event)=>runtime.on_rpc(event).await,None=>runtime.stop("Discord transport stopped. Choose Connect.").await}}
            result=runtime.oauth_results.recv()=>{match result{Some(result)=>runtime.on_oauth(result).await,None=>Ok(())}}
        };
        if let Err(error) = result {
            runtime.message = error.to_string();
            runtime.status().await?;
        }
    }
    runtime.stop("Host disconnected.").await.ok();
    Ok(())
}
fn options() -> Result<Options> {
    let mut arguments = std::env::args().skip(1);
    let mut values = HashMap::new();
    while let Some(key) = arguments.next() {
        let value = arguments
            .next()
            .context("Missing plugin launch argument value")?;
        values.insert(key.trim_start_matches('-').to_owned(), value);
    }
    let port: u16 = values.get("port").context("Missing -port")?.parse()?;
    Ok(Options {
        host: format!("ws://127.0.0.1:{port}"),
        uuid: values.remove("pluginUUID").context("Missing -pluginUUID")?,
        registration: values
            .remove("registerEvent")
            .context("Missing -registerEvent")?,
        config: config::path()?,
        ipc: ipc::paths(),
        oauth: "https://discord.com/api/oauth2/token".into(),
    })
}
#[tokio::main]
async fn main() {
    if let Err(error) = async { run(options()?).await }.await {
        eprintln!("Discord plugin stopped: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    use tokio::net::{TcpListener, UnixListener, UnixStream};
    type HostSocket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;
    async fn send(host: &mut HostSocket, event: Value) {
        host.send(Message::Text(event.to_string().into()))
            .await
            .unwrap();
    }
    async fn rpc_read(peer: &mut UnixStream) -> Value {
        let (op, bytes) = ipc::read_frame(peer).await.unwrap();
        assert_eq!(op, 1);
        serde_json::from_slice(&bytes).unwrap()
    }
    async fn reply(peer: &mut UnixStream, request: &Value, data: Value) {
        ipc::write_frame(
            peer,
            1,
            &serde_json::to_vec(
                &json!({"cmd":request["cmd"],"nonce":request["nonce"],"data":data}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    }
    async fn next_command(peer: &mut UnixStream, command: &str) -> Value {
        for _ in 0..32 {
            let request = rpc_read(peer).await;
            if request["cmd"] == command {
                return request;
            }
            reply(peer, &request, json!({})).await;
        }
        panic!("Expected fixture RPC command was not sent")
    }
    #[tokio::test]
    async fn real_host_websocket_requires_connect_routes_nonce_and_flushes_held_release() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let dir=tempfile::tempdir().unwrap();let socket_path=dir.path().join("fixture.ipc");let listener=UnixListener::bind(&socket_path).unwrap();
            let config_path=dir.path().join("private/auth.json");
            config::save(&config_path,&config::Config{client_id:"123456789012345678".into(),client_secret:"fixture-secret".into(),redirect_uri:String::new(),scopes:vec!["rpc".into()],token:Some(config::Token{access_token:"fixture-access".into(),refresh_token:"fixture-refresh".into(),expires_at:config::now()+3600})}).unwrap();
            let server=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let options=Options{host:format!("ws://{}",server.local_addr().unwrap()),uuid:"fixture-plugin".into(),registration:"registerPlugin".into(),config:config_path,ipc:vec![socket_path],oauth:"http://127.0.0.1:1/unused-fixture-endpoint".into()};
            let plugin=tokio::spawn(run(options));
            let (tcp,_)=server.accept().await.unwrap();let mut host=tokio_tungstenite::accept_async(tcp).await.unwrap();
            let registration=host.next().await.unwrap().unwrap();assert_eq!(serde_json::from_str::<Value>(registration.to_text().unwrap()).unwrap(),json!({"event":"registerPlugin","uuid":"fixture-plugin"}));
            let appear=json!({"event":"willAppear","action":"com.elgato.discord.mute","context":"mute-slot","payload":{"controller":"Keypad","settings":{}}});
            send(&mut host,appear).await;
            assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err(),"No Discord connection before explicit Connect");
            send(&mut host,json!({"event":"sendToPlugin","action":"com.elgato.discord.mute","context":"mute-slot","payload":{"command":"connect"}})).await;
            let (mut peer,_)=listener.accept().await.unwrap();let (op,bytes)=ipc::read_frame(&mut peer).await.unwrap();assert_eq!(op,0);assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["client_id"],"123456789012345678");
            ipc::write_frame(&mut peer,1,br#"{"cmd":"DISPATCH","evt":"READY","data":{}}"#).await.unwrap();
            let auth=next_command(&mut peer,"AUTHENTICATE").await;assert_eq!(auth["args"]["access_token"],"fixture-access");reply(&mut peer,&auth,json!({"user":{"id":"fixture-user"},"scopes":["rpc"]})).await;
            let voice=next_command(&mut peer,"GET_VOICE_SETTINGS").await;reply(&mut peer,&voice,json!({"mute":false,"deaf":false,"mode":{"type":"VOICE_ACTIVITY"},"input":{"volume":100},"output":{"volume":100}})).await;
            loop {
                let message = host.next().await.unwrap().unwrap();
                let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if event["event"] == "sendToPropertyInspector" && event["payload"]["voice"]["mute"] == false { break; }
            }
            send(&mut host,json!({"event":"keyDown","context":"mute-slot","action":"com.elgato.discord.mute","payload":{}})).await;
            let mute=next_command(&mut peer,"SET_VOICE_SETTINGS").await;assert_eq!(mute["args"]["mute"],true);
            ipc::write_frame(&mut peer,1,&serde_json::to_vec(&json!({"cmd":"SET_VOICE_SETTINGS","evt":"ERROR","nonce":mute["nonce"],"data":{"code":5000,"message":"fixture-secret-must-not-be-echoed"}})).unwrap()).await.unwrap();
            loop {let message=host.next().await.unwrap().unwrap();let text=message.to_text().unwrap();assert!(!text.contains("fixture-secret"));let event:Value=serde_json::from_str(text).unwrap();if event["event"]=="showAlert" && event["context"]=="mute-slot"{break;}}
            send(&mut host,json!({"event":"willAppear","context":"held-slot","action":"com.elgato.discord.pushto.mute","payload":{"controller":"Keypad","settings":{}}})).await;
            send(&mut host,json!({"event":"keyDown","context":"held-slot","action":"com.elgato.discord.pushto.mute","payload":{}})).await;
            let held=next_command(&mut peer,"SET_VOICE_SETTINGS").await;assert_eq!(held["args"]["mute"],true);reply(&mut peer,&held,json!({"mute":true})).await;
            send(&mut host,json!({"event":"sendToPlugin","context":"mute-slot","action":"com.elgato.discord.mute","payload":{"command":"disconnect"}})).await;
            let release=next_command(&mut peer,"SET_VOICE_SETTINGS").await;assert_eq!(release["args"]["mute"],false);
            assert!(ipc::read_frame(&mut peer).await.is_err());
            assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err(),"Disconnect must stop automatic reconnect");
            host.close(None).await.unwrap();assert!(plugin.await.unwrap().is_ok());
        }).await.unwrap();
    }
}
