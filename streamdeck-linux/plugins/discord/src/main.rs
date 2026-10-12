//! Independent native Discord plugin. No Discord connection exists before Connect.
mod actions;
mod auth;
mod auth_error;
mod config;
mod graphics;
mod ipc;
mod widget;

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
    widget: String,
}
struct Pending {
    cmd: String,
    context: Option<String>,
    since: Instant,
    args: Option<Value>,
    subscription_event: Option<String>,
}
struct OAuthResult {
    generation: u64,
    result: Result<config::Token>,
}
struct WidgetResult {
    generation: u64,
    guild_id: String,
    presence: Option<u64>,
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
    widget_endpoint: String,
    allow_authorize: bool,
    force_authorize: bool,
    candidate_token: Option<config::Token>,
    auth_notice: Option<String>,
    auth_nonce: Option<String>,
    http: reqwest::Client,
    outgoing: mpsc::Sender<Value>,
    rpc: Option<mpsc::Sender<ipc::Command>>,
    events: mpsc::Receiver<ipc::Event>,
    task: Option<AbortTask>,
    oauth_results: mpsc::Receiver<OAuthResult>,
    oauth_sender: mpsc::Sender<OAuthResult>,
    oauth_task: Option<AbortTask>,
    generation: u64,
    widget_sender: mpsc::Sender<WidgetResult>,
    widget_results: mpsc::Receiver<WidgetResult>,
    widget_tasks: HashMap<String, AbortTask>,
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
        let (widget_sender, widget_results) = mpsc::channel(32);
        Ok(Self {
            engine: ActionEngine::new(),
            saved,
            config_path: options.config.clone(),
            paths: options.ipc.clone(),
            oauth_endpoint: options.oauth.clone(),
            widget_endpoint: options.widget.clone(),
            allow_authorize: false,
            force_authorize: false,
            candidate_token: None,
            auth_notice: None,
            auth_nonce: None,
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
            widget_sender,
            widget_results,
            widget_tasks: HashMap::new(),
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
                Effect::Subscription {
                    event,
                    args,
                    context,
                    subscribe,
                } => {
                    if self.authenticated {
                        self.request(
                            if subscribe {
                                "SUBSCRIBE"
                            } else {
                                "UNSUBSCRIBE"
                            },
                            args,
                            context,
                            Some(&event),
                        )
                        .await?;
                    }
                }
                Effect::GuildWidget { guild_id } => {
                    if self.authenticated && !self.widget_tasks.contains_key(&guild_id) {
                        ensure!(
                            self.widget_tasks.len() < 32,
                            "Too many pending guild widget requests"
                        );
                        let client = self.http.clone();
                        let endpoint = self.widget_endpoint.clone();
                        let tx = self.widget_sender.clone();
                        let generation = self.generation;
                        let id = guild_id.clone();
                        let task = tokio::spawn(async move {
                            let presence = widget::presence(&client, &endpoint, &id).await.ok();
                            let _ = tx
                                .send(WidgetResult {
                                    generation,
                                    guild_id: id,
                                    presence,
                                })
                                .await;
                        });
                        self.widget_tasks.insert(guild_id, AbortTask(task));
                    }
                }
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
        let echoed_args =
            matches!(cmd, "GET_CHANNELS" | "SUBSCRIBE" | "UNSUBSCRIBE").then(|| args.clone());
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
        if matches!(cmd, "AUTHORIZE" | "AUTHENTICATE") {
            self.auth_nonce = Some(nonce.clone());
        }
        self.pending.insert(
            nonce,
            Pending {
                cmd: cmd.into(),
                context,
                since: Instant::now(),
                args: echoed_args,
                subscription_event: evt.map(str::to_owned),
            },
        );
        Ok(())
    }
    async fn stop(&mut self, message: &str) -> Result<()> {
        let effects = self.engine.connection(false);
        let mut releases = Vec::new();
        let mut updates = Vec::new();
        for effect in effects {
            match effect {
                Effect::Rpc { .. } => releases.push(effect),
                Effect::Host(value) => updates.push(value),
                _ => {} // Closing the IPC session removes its subscriptions.
            }
        }
        // One deadline fits inside the host's 750 ms SIGTERM grace. UI failures
        // must never interrupt physical held-control releases or their flush.
        let cleanup_ok = matches!(
            tokio::time::timeout(Duration::from_millis(500), async {
                self.effects(releases).await?;
                if let Some(sender) = self.rpc.as_ref() {
                    let (done, receipt) = tokio::sync::oneshot::channel();
                    sender
                        .send(ipc::Command::Flush(done))
                        .await
                        .context("Discord transport stopped")?;
                    receipt.await.context("Discord release flush failed")?;
                }
                Ok::<(), anyhow::Error>(())
            })
            .await,
            Ok(Ok(()))
        );
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
        self.oauth_task = None;
        self.widget_tasks.clear();
        self.rpc = None;
        self.pending.clear();
        self.candidate_token = None;
        self.auth_nonce = None;
        self.connected = false;
        self.authenticated = false;
        self.voice_channel = None;
        self.message = if cleanup_ok {
            message.into()
        } else {
            format!("{message} Discord did not accept all held-control release requests.")
        };
        for value in updates {
            let _ = self.outgoing.try_send(value);
        }
        for (context, action) in &self.inspectors {
            let _ = self.outgoing.try_send(json!({"event":"sendToPropertyInspector","context":context,"action":action,"payload":{"auth":{"configured":self.saved.is_some(),"connected":false,"authenticated":false,"message":self.message}}}));
        }
        Ok(())
    }
    fn connect_config(&self, payload: &Value, explicit: bool) -> Result<config::Config> {
        let get = |key: &str| {
            payload
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        let mut candidate = self.saved.clone().unwrap_or(config::Config {
            enabled: false,
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
        if explicit && payload["permissions_for_actions"] == true {
            for scope in self.engine.required_scopes() {
                if !candidate.scopes.contains(&scope) {
                    candidate.scopes.push(scope);
                }
            }
        }
        candidate.scopes.sort();
        candidate.scopes.dedup();
        config::validate(&candidate)?;
        // A scope/secret/redirect edit does not revoke an existing same-app token.
        if self
            .saved
            .as_ref()
            .is_none_or(|old| old.client_id != candidate.client_id)
        {
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
                    "connect" => self.connect(event["payload"].clone(), true).await,
                    "disconnect" => {
                        let persisted = if let Some(config) = self.saved.as_mut() {
                            config.enabled = false;
                            config::save(&self.config_path, config)
                        } else {
                            Ok(())
                        };
                        self.stop("Disconnected. Choose Connect to reconnect.")
                            .await?;
                        persisted
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
    async fn connect(&mut self, payload: Value, explicit: bool) -> Result<()> {
        let mut config = self.connect_config(&payload, explicit)?;
        let scopes_changed = self.saved.as_ref().is_some_and(|old| {
            let mut scopes = old.scopes.clone();
            scopes.sort();
            scopes.dedup();
            scopes != config.scopes
        });
        let force_authorize = explicit && (payload["reauthorize"] == true || scopes_changed);
        config.enabled = true;
        config::save(&self.config_path, &config)?;
        self.stop("Connecting to Discord desktop…").await?;
        self.saved = Some(config.clone());
        self.allow_authorize = explicit;
        self.force_authorize = force_authorize;
        self.auth_notice = None;
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
        if self.force_authorize {
            self.authorize().await?;
        } else if let Some(token) = saved.token {
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
        if !self.allow_authorize {
            self.message =
                "Discord needs new authorization. Choose Connect to approve access.".into();
            return Ok(());
        }
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
                self.candidate_token = None;
                self.auth_nonce = None;
                self.force_authorize = false;
                self.widget_tasks.clear();
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
                if let Some(object) = frame.as_object_mut() {
                    for internal in ["_context", "_args", "_subscription_event"] {
                        object.remove(internal);
                    }
                }
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
                    if matches!(cmd.as_str(), "AUTHORIZE" | "AUTHENTICATE")
                        && self.auth_nonce.as_deref() != Some(nonce)
                    {
                        return Ok(());
                    }
                    if let Some(event) = pending.subscription_event {
                        frame["_subscription_event"] = json!(event);
                    }
                    if let Some(args) = pending.args {
                        frame["_args"] = args;
                    }
                    if let Some(context) = pending.context {
                        frame["_context"] = json!(context);
                    }
                } else if cmd != "DISPATCH" {
                    return Ok(());
                }
                if evt == "ERROR" && ["AUTHORIZE", "AUTHENTICATE"].contains(&cmd.as_str()) {
                    let replacement_failed = cmd == "AUTHORIZE" || self.candidate_token.is_some();
                    let message = auth_error::describe(&cmd, &frame["data"]);
                    self.authenticated = false;
                    let effects = self.engine.connection(false);
                    self.effects(effects).await?;
                    if !replacement_failed || !self.restore_previous_authorization(&message).await?
                    {
                        self.candidate_token = None;
                        self.force_authorize = false;
                        self.message = message;
                    }
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
                    if let Some(token) = self.candidate_token.take() {
                        let config = self.saved.as_mut().context("Discord config missing")?;
                        config.token = Some(token);
                        config::save(&self.config_path, config)?;
                    }
                    self.force_authorize = false;
                    self.authenticated = true;
                    self.message = self
                        .auth_notice
                        .take()
                        .unwrap_or_else(|| "Connected to Discord.".into());
                    for event in [
                        "VOICE_SETTINGS_UPDATE",
                        "VOICE_CHANNEL_SELECT",
                        "GUILD_CREATE",
                        "CHANNEL_CREATE",
                    ] {
                        self.request("SUBSCRIBE", json!({}), None, Some(event))
                            .await?;
                    }
                    if frame["data"]["scopes"]
                        .as_array()
                        .is_some_and(|scopes| scopes.iter().any(|s| s == "rpc.notifications.read"))
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
    async fn restore_previous_authorization(&mut self, reason: &str) -> Result<bool> {
        self.candidate_token = None;
        self.force_authorize = false;
        let token = self
            .saved
            .as_ref()
            .and_then(|config| config.token.as_ref())
            .filter(|token| token.expires_at > config::now() + 60)
            .cloned();
        let Some(token) = token else {
            return Ok(false);
        };
        self.auth_notice = Some(format!(
            "{reason} Restored the previous authorized permissions."
        ));
        self.message = "Restoring the previous authorized Discord session…".into();
        self.request(
            "AUTHENTICATE",
            json!({"access_token":token.access_token}),
            None,
            None,
        )
        .await?;
        Ok(true)
    }
    async fn on_oauth(&mut self, result: OAuthResult) -> Result<()> {
        if result.generation != self.generation || self.task.is_none() {
            return Ok(());
        }
        self.oauth_task = None;
        match result.result {
            Ok(token) => {
                // Commit the replacement only after Discord accepts AUTHENTICATE.
                // A refused upgrade must not destroy the working token on disk.
                self.candidate_token = Some(token.clone());
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
                let message = error.to_string();
                self.authenticated = false;
                let effects = self.engine.connection(false);
                self.effects(effects).await?;
                if !self.restore_previous_authorization(&message).await? {
                    self.message = message;
                }
            }
        }
        self.status().await
    }
}

async fn run(options: Options) -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    run_until(options, async move {
        tokio::select! {_=terminate.recv()=>{},_=interrupt.recv()=>{}}
    })
    .await
}
async fn run_until(
    options: Options,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    tokio::pin!(shutdown);
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
    if runtime.saved.as_ref().is_some_and(|config| config.enabled)
        && let Err(error) = runtime.connect(json!({}), false).await
    {
        runtime.message = error.to_string();
    }

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let result = tokio::select! {
            _=&mut shutdown=>break,
            message=reader.next()=>{
                match message {Some(Ok(Message::Text(text)))=>{match serde_json::from_str::<Value>(&text){Ok(event)=>runtime.on_host(event).await,Err(_)=>Ok(())}},Some(Ok(Message::Close(_)))|None=>break,Some(Err(_))=>break,_=>Ok(())}
            }
            event=runtime.events.recv(), if runtime.task.is_some()=>{match event{Some(event)=>runtime.on_rpc(event).await,None=>runtime.stop("Discord transport stopped. Choose Connect.").await}}
            result=runtime.oauth_results.recv()=>{match result{Some(result)=>runtime.on_oauth(result).await,None=>Ok(())}}
            result=runtime.widget_results.recv()=>{
                if let Some(result)=result && result.generation==runtime.generation && runtime.authenticated {
                    runtime.widget_tasks.remove(&result.guild_id);
                    let effects=runtime.engine.rpc_event(&json!({"cmd":"DECKFORGE_GUILD_WIDGET","data":{"guild_id":result.guild_id,"presence_count":result.presence}}));
                    runtime.effects(effects).await
                } else { Ok(()) }
            }
            _=tick.tick()=>{let effects=runtime.engine.tick();runtime.effects(effects).await}
        };
        if let Err(error) = result {
            runtime.message = error.to_string();
            if runtime.status().await.is_err() {
                break;
            }
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
        widget: "https://discord.com/api/guilds".into(),
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
            config::save(&config_path,&config::Config{enabled:false,client_id:"123456789012345678".into(),client_secret:"fixture-secret".into(),redirect_uri:String::new(),scopes:vec!["rpc".into()],token:Some(config::Token{access_token:"fixture-access".into(),refresh_token:"fixture-refresh".into(),expires_at:config::now()+3600})}).unwrap();
            let server=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let options=Options{host:format!("ws://{}",server.local_addr().unwrap()),uuid:"fixture-plugin".into(),registration:"registerPlugin".into(),config:config_path.clone(),ipc:vec![socket_path],oauth:"http://127.0.0.1:1/unused-fixture-endpoint".into(),widget:"http://127.0.0.1:1/unused-widget".into()};
            let plugin=tokio::spawn(run(options));
            let (tcp,_)=server.accept().await.unwrap();let mut host=tokio_tungstenite::accept_async(tcp).await.unwrap();
            let registration=host.next().await.unwrap().unwrap();assert_eq!(serde_json::from_str::<Value>(registration.to_text().unwrap()).unwrap(),json!({"event":"registerPlugin","uuid":"fixture-plugin"}));
            let appear=json!({"event":"willAppear","action":"com.elgato.discord.mute","context":"mute-slot","payload":{"controller":"Keypad","settings":{}}});
            send(&mut host,appear).await;
            assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err(),"No Discord connection before explicit Connect");
            send(&mut host,json!({"event":"sendToPlugin","action":"com.elgato.discord.mute","context":"mute-slot","payload":{"command":"connect"}})).await;
            let (mut peer,_)=listener.accept().await.unwrap();let (op,bytes)=ipc::read_frame(&mut peer).await.unwrap();assert_eq!(op,0);assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["client_id"],"123456789012345678");
            ipc::write_frame(&mut peer,1,br#"{"cmd":"DISPATCH","evt":"READY","data":{}}"#).await.unwrap();
            let auth=next_command(&mut peer,"AUTHENTICATE").await;assert_eq!(auth["args"]["access_token"],"fixture-access");reply(&mut peer,&auth,json!({"user":{"id":"fixture-user"},"scopes":["rpc","rpc.notifications.read","messages.read"]})).await;
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
            send(&mut host,json!({"event":"willAppear","context":"notification-slot","action":"com.elgato.discord.notifications","payload":{"controller":"Keypad","settings":{"select-press":"open"}}})).await;
            ipc::write_frame(&mut peer,1,br#"{"cmd":"DISPATCH","evt":"NOTIFICATION_CREATE","data":{"channel_id":"notice-channel","message":{"id":"notice"}}}"#).await.unwrap();
            loop {let message=host.next().await.unwrap().unwrap();let event:Value=serde_json::from_str(message.to_text().unwrap()).unwrap();if event["event"]=="setTitle"&&event["context"]=="notification-slot"&&event["payload"]["title"]=="1"{break;}}
            send(&mut host,json!({"event":"keyDown","context":"notification-slot","action":"com.elgato.discord.notifications","payload":{}})).await;
            send(&mut host,json!({"event":"keyUp","context":"notification-slot","action":"com.elgato.discord.notifications","payload":{}})).await;
            let open=next_command(&mut peer,"SELECT_TEXT_CHANNEL").await;assert_eq!(open["args"]["channel_id"],"notice-channel");reply(&mut peer,&open,json!({})).await;
            send(&mut host,json!({"event":"willAppear","context":"stats-slot","action":"com.elgato.discord.serverstats","payload":{"controller":"Keypad","settings":{"server":"123","select-press":"open","select-messages":"display","select-users":"active"}}})).await;
            let channels=next_command(&mut peer,"GET_CHANNELS").await;assert_eq!(channels["args"]["guild_id"],"123");reply(&mut peer,&channels,json!({"channels":[{"id":"text-channel","type":0},{"id":"voice-channel","type":2}]})).await;
            let subscription=next_command(&mut peer,"SUBSCRIBE").await;assert_eq!(subscription["evt"],"MESSAGE_CREATE");assert_eq!(subscription["args"]["channel_id"],"text-channel");reply(&mut peer,&subscription,json!({})).await;
            ipc::write_frame(&mut peer,1,br#"{"cmd":"DISPATCH","evt":"MESSAGE_CREATE","data":{"channel_id":"text-channel","message":{"author":{"id":"fixture-author"}}}}"#).await.unwrap();
            loop {let message=host.next().await.unwrap().unwrap();let event:Value=serde_json::from_str(message.to_text().unwrap()).unwrap();if event["event"]=="setTitle"&&event["context"]=="stats-slot"&&event["payload"]["title"]=="1 messages\n1 active"{break;}}
            send(&mut host,json!({"event":"keyDown","context":"stats-slot","action":"com.elgato.discord.serverstats","payload":{}})).await;
            send(&mut host,json!({"event":"keyUp","context":"stats-slot","action":"com.elgato.discord.serverstats","payload":{}})).await;
            let open=next_command(&mut peer,"SELECT_TEXT_CHANNEL").await;assert_eq!(open["args"]["channel_id"],"text-channel");reply(&mut peer,&open,json!({})).await;
            send(&mut host,json!({"event":"willAppear","context":"held-slot","action":"com.elgato.discord.pushto.mute","payload":{"controller":"Keypad","settings":{}}})).await;
            send(&mut host,json!({"event":"keyDown","context":"held-slot","action":"com.elgato.discord.pushto.mute","payload":{}})).await;
            let held=next_command(&mut peer,"SET_VOICE_SETTINGS").await;assert_eq!(held["args"]["mute"],true);reply(&mut peer,&held,json!({"mute":true})).await;
            send(&mut host,json!({"event":"sendToPlugin","context":"mute-slot","action":"com.elgato.discord.mute","payload":{"command":"disconnect"}})).await;
            let release=next_command(&mut peer,"SET_VOICE_SETTINGS").await;assert_eq!(release["args"]["mute"],false);
            // Disconnect can also flush subscription removals before closing.
            while let Ok((_, bytes)) = ipc::read_frame(&mut peer).await {
                let frame:Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(frame["cmd"],"UNSUBSCRIBE");
            }
            assert!(!config::load(&config_path).unwrap().unwrap().enabled);
            assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err(),"Disconnect must stop automatic reconnect");
            host.close(None).await.unwrap();assert!(plugin.await.unwrap().is_ok());
        }).await.unwrap();
    }
    #[tokio::test]
    async fn enabled_sessions_resume_but_never_open_new_consent_on_startup() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let dir = tempfile::tempdir().unwrap();
            let config_path = dir.path().join("private/auth.json");
            for saved_token in [true, false] {
                config::save(
                    &config_path,
                    &config::Config {
                        enabled: true,
                        client_id: "123456789012345678".into(),
                        client_secret: "fixture-secret".into(),
                        redirect_uri: String::new(),
                        scopes: vec!["rpc".into()],
                        token: saved_token.then(|| config::Token {
                            access_token: "fixture-access".into(),
                            refresh_token: "fixture-refresh".into(),
                            expires_at: config::now() + 3600,
                        }),
                    },
                )
                .unwrap();
                let path = dir.path().join(if saved_token {
                    "resume.ipc"
                } else {
                    "consent.ipc"
                });
                let ipc = UnixListener::bind(&path).unwrap();
                let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let options = Options {
                    host: format!("ws://{}", server.local_addr().unwrap()),
                    uuid: "fixture-plugin".into(),
                    registration: "registerPlugin".into(),
                    config: config_path.clone(),
                    ipc: vec![path],
                    oauth: "http://127.0.0.1:1/unused-token".into(),
                    widget: "http://127.0.0.1:1/unused-widget".into(),
                };
                let plugin = tokio::spawn(run(options));
                let (tcp, _) = server.accept().await.unwrap();
                let mut host = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let _ = host.next().await.unwrap().unwrap();
                let (mut peer, _) = ipc.accept().await.unwrap();
                assert_eq!(ipc::read_frame(&mut peer).await.unwrap().0, 0);
                ipc::write_frame(
                    &mut peer,
                    1,
                    br#"{"cmd":"DISPATCH","evt":"READY","data":{}}"#,
                )
                .await
                .unwrap();
                if saved_token {
                    let frame = rpc_read(&mut peer).await;
                    assert_eq!(frame["cmd"], "AUTHENTICATE");
                    assert_eq!(frame["args"]["access_token"], "fixture-access");
                } else {
                    assert!(
                        tokio::time::timeout(
                            Duration::from_millis(100),
                            ipc::read_frame(&mut peer)
                        )
                        .await
                        .is_err(),
                        "Restart must not show a new AUTHORIZE prompt"
                    );
                }
                host.close(None).await.unwrap();
                assert!(plugin.await.unwrap().is_ok());
                assert!(
                    config::load(&config_path).unwrap().unwrap().enabled,
                    "Host restart must retain prior Connect authorization"
                );
            }
        })
        .await
        .unwrap();
    }
    fn runtime_fixture() -> (
        Runtime,
        mpsc::Receiver<ipc::Command>,
        mpsc::Receiver<Value>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private/auth.json");
        config::save(
            &path,
            &config::Config {
                enabled: true,
                client_id: "123456789012345678".into(),
                client_secret: "fixture-secret".into(),
                redirect_uri: String::new(),
                scopes: vec!["rpc".into()],
                token: Some(config::Token {
                    access_token: "old-valid-token".into(),
                    refresh_token: "old-refresh".into(),
                    expires_at: config::now() + 3600,
                }),
            },
        )
        .unwrap();
        let (outgoing, host) = mpsc::channel(128);
        let options = Options {
            host: String::new(),
            uuid: String::new(),
            registration: String::new(),
            config: path,
            ipc: vec![],
            oauth: "http://127.0.0.1:1/unused".into(),
            widget: "http://127.0.0.1:1/unused".into(),
        };
        let mut runtime = Runtime::new(&options, outgoing).unwrap();
        let (sender, rpc) = mpsc::channel(64);
        runtime.rpc = Some(sender);
        runtime.connected = true;
        (runtime, rpc, host, dir)
    }
    async fn queued_frame(receiver: &mut mpsc::Receiver<ipc::Command>) -> Value {
        match receiver.recv().await.unwrap() {
            ipc::Command::Frame(value) => value,
            ipc::Command::Flush(done) => {
                let _ = done.send(());
                panic!("unexpected fixture flush")
            }
        }
    }
    #[tokio::test]
    async fn repeated_auth_errors_keep_safe_details_without_changing_saved_credentials() {
        let (mut runtime, mut rpc, _host, _dir) = runtime_fixture();
        let before = std::fs::read(&runtime.config_path).unwrap();
        for _ in 0..2 {
            runtime
                .request(
                    "AUTHENTICATE",
                    json!({"access_token":"fixture-token"}),
                    None,
                    None,
                )
                .await
                .unwrap();
            let request = queued_frame(&mut rpc).await;
            runtime
                .on_rpc(ipc::Event::Frame(json!({
                    "cmd":"AUTHENTICATE", "evt":"ERROR", "nonce":request["nonce"],
                    "data":{"code":5000,"error":{"message":"invalid_scope: NEVER_ECHO_SECRET"}}
                })))
                .await
                .unwrap();
            assert!(
                runtime
                    .message
                    .contains("AUTHENTICATE failed (RPC code 5000)")
            );
            assert!(runtime.message.contains("invalid_scope"));
            assert!(!runtime.message.contains("NEVER_ECHO_SECRET"));
            assert!(!runtime.authenticated);
        }
        assert_eq!(std::fs::read(&runtime.config_path).unwrap(), before);
    }
    #[tokio::test]
    async fn explicit_permission_upgrade_preserves_previous_token_until_acceptance() {
        let (mut runtime, _, _host, _dir) = runtime_fixture();
        runtime.engine.host_event(&json!({"event":"willAppear","context":"notice","action":"com.elgato.discord.notifications","payload":{"settings":{}}}));
        let flags = json!({"permissions_for_actions":true,"reauthorize":true});
        let automatic = runtime.connect_config(&flags, false).unwrap();
        assert!(!automatic.scopes.contains(&"rpc.notifications.read".into()));
        runtime.rpc = None; // No transport exists yet; explicit Connect starts no socket for empty fixture paths.
        runtime.connect(flags, true).await.unwrap();
        assert!(
            runtime
                .saved
                .as_ref()
                .unwrap()
                .scopes
                .contains(&"rpc.notifications.read".into())
        );
        assert_eq!(
            config::load(&runtime.config_path)
                .unwrap()
                .unwrap()
                .token
                .unwrap()
                .access_token,
            "old-valid-token"
        );
        let (sender, mut receiver) = mpsc::channel(64);
        runtime.rpc = Some(sender);
        runtime.task = Some(AbortTask(tokio::spawn(std::future::pending())));
        runtime.ready().await.unwrap();
        let authorize = queued_frame(&mut receiver).await;
        assert_eq!(authorize["cmd"], "AUTHORIZE");
        runtime.on_rpc(ipc::Event::Frame(json!({"cmd":"AUTHORIZE","evt":"ERROR","nonce":authorize["nonce"],"data":{"code":4008}}))).await.unwrap();
        let restored = queued_frame(&mut receiver).await;
        assert_eq!(restored["cmd"], "AUTHENTICATE");
        assert_eq!(restored["args"]["access_token"], "old-valid-token");
        assert_eq!(
            config::load(&runtime.config_path)
                .unwrap()
                .unwrap()
                .token
                .unwrap()
                .refresh_token,
            "old-refresh"
        );
        // A successfully exchanged candidate remains staged until RPC accepts it.
        let generation = runtime.generation;
        runtime
            .on_oauth(OAuthResult {
                generation,
                result: Ok(config::Token {
                    access_token: "candidate-token".into(),
                    refresh_token: "candidate-refresh".into(),
                    expires_at: config::now() + 3600,
                }),
            })
            .await
            .unwrap();
        let candidate = queued_frame(&mut receiver).await;
        assert_eq!(candidate["args"]["access_token"], "candidate-token");
        // A late reply for a prior auth request cannot commit the staged token.
        runtime
            .on_rpc(ipc::Event::Frame(
                json!({"cmd":"AUTHENTICATE","nonce":restored["nonce"],"data":{"scopes":["rpc"]}}),
            ))
            .await
            .unwrap();
        assert!(!runtime.authenticated);

        assert_eq!(
            config::load(&runtime.config_path)
                .unwrap()
                .unwrap()
                .token
                .unwrap()
                .access_token,
            "old-valid-token"
        );
        runtime.on_rpc(ipc::Event::Frame(json!({"cmd":"AUTHENTICATE","evt":"ERROR","nonce":candidate["nonce"],"data":{"code":4009}}))).await.unwrap();
        let restored = queued_frame(&mut receiver).await;
        assert_eq!(restored["args"]["access_token"], "old-valid-token");
        runtime
            .on_oauth(OAuthResult {
                generation,
                result: Ok(config::Token {
                    access_token: "accepted-token".into(),
                    refresh_token: "accepted-refresh".into(),
                    expires_at: config::now() + 3600,
                }),
            })
            .await
            .unwrap();
        let accepted = queued_frame(&mut receiver).await;
        runtime.on_rpc(ipc::Event::Frame(json!({"cmd":"AUTHENTICATE","nonce":accepted["nonce"],"data":{"scopes":["rpc","rpc.notifications.read"]}}))).await.unwrap();
        assert_eq!(
            config::load(&runtime.config_path)
                .unwrap()
                .unwrap()
                .token
                .unwrap()
                .access_token,
            "accepted-token"
        );
        let changed = runtime
            .connect_config(&json!({"client_id":"987654321098765432"}), true)
            .unwrap();
        assert!(changed.token.is_none());
        assert!(
            runtime
                .connect_config(&json!({"reauthorize":true}), true)
                .unwrap()
                .token
                .is_some()
        );
    }
    #[tokio::test]
    async fn subscription_metadata_reaches_all_affected_notification_actions() {
        let (mut runtime, mut receiver, mut host, _dir) = runtime_fixture();
        runtime.authenticated = true;
        runtime.engine.connection(true);
        for context in ["n1", "n2"] {
            runtime.engine.host_event(&json!({"event":"willAppear","context":context,"action":"com.elgato.discord.notifications","payload":{"settings":{}}}));
        }
        runtime
            .request("SUBSCRIBE", json!({}), None, Some("NOTIFICATION_CREATE"))
            .await
            .unwrap();
        let request = queued_frame(&mut receiver).await;
        runtime.on_rpc(ipc::Event::Frame(json!({"cmd":"SUBSCRIBE","evt":"ERROR","nonce":request["nonce"],"data":{"code":4006}}))).await.unwrap();
        let mut alerted = vec![];
        while let Ok(event) = host.try_recv() {
            if event["event"] == "showAlert" {
                alerted.push(event["context"].as_str().unwrap().to_owned());
            }
        }
        alerted.sort();
        assert_eq!(alerted, vec!["n1", "n2"]);
        runtime
            .request(
                "AUTHENTICATE",
                json!({"access_token":"not-for-engine"}),
                None,
                None,
            )
            .await
            .unwrap();
        let request = queued_frame(&mut receiver).await;
        let pending = &runtime.pending[request["nonce"].as_str().unwrap()];
        assert!(pending.args.is_none());
        assert!(pending.subscription_event.is_none());
    }
    #[tokio::test]
    async fn shutdown_flushes_held_release_even_when_host_ui_is_closed() {
        let (mut runtime, mut receiver, host, _dir) = runtime_fixture();
        runtime.authenticated = true;
        runtime.engine.connection(true);
        runtime.engine.host_event(&json!({"event":"willAppear","context":"held","action":"com.elgato.discord.pushto.mute","payload":{"settings":{}}}));
        let effects=runtime.engine.host_event(&json!({"event":"keyDown","context":"held","action":"com.elgato.discord.pushto.mute","payload":{}}));
        runtime.effects(effects).await.unwrap();
        assert_eq!(queued_frame(&mut receiver).await["args"]["mute"], true);
        drop(host);
        let peer = tokio::spawn(async move {
            let release = queued_frame(&mut receiver).await;
            assert_eq!(release["args"]["mute"], false);
            if let ipc::Command::Flush(done) = receiver.recv().await.unwrap() {
                done.send(()).unwrap();
            } else {
                panic!("release flush missing");
            }
        });
        tokio::time::timeout(Duration::from_millis(700), runtime.stop("Host shutdown"))
            .await
            .unwrap()
            .unwrap();
        peer.await.unwrap();
        assert!(config::load(&runtime.config_path).unwrap().unwrap().enabled);
        assert!(runtime.rpc.is_none());
    }
}
