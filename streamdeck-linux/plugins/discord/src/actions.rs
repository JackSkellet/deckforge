//! Original action implementation. UUID/settings compatibility comes from the
//! inspected manifest; RPC shapes from Discord's official RPC documentation.
//! Notification/cache facts are bounded to the inspected Mac 2.4 artifact in
//! evidence/discord-notifications-stats.json; server short-press policy is explicit.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Effect {
    Rpc {
        cmd: String,
        args: Value,
        context: Option<String>,
    },
    Subscription {
        event: String,
        args: Value,
        context: Option<String>,
        subscribe: bool,
    },
    GuildWidget {
        guild_id: String,
    },
    Host(Value),
}
#[derive(Clone)]
struct Slot {
    action: String,
    settings: Value,
    encoder: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Held {
    Mute,
    TalkActivity,
    TalkPtt,
}
#[derive(Clone)]
struct Notice {
    channel: String,
}
#[derive(Default)]
struct GuildStats {
    messages: u64,
    last_channel: Option<String>,
    authors: BTreeMap<String, Instant>,
    online: Option<u64>,
}
#[derive(Default)]
pub struct ActionEngine {
    slots: BTreeMap<String, Slot>,
    connected: bool,
    voice: Value,
    selected: Value,
    guilds: Value,
    channels: BTreeMap<String, Value>,
    channel_servers: BTreeMap<String, String>,
    users: BTreeMap<String, Value>,
    speaker: Option<String>,
    errors: BTreeMap<String, String>,
    subscription_errors: BTreeSet<String>,
    held: BTreeMap<String, Held>,
    targets: BTreeMap<String, Value>,
    scopes: BTreeSet<String>,
    notifications: VecDeque<Notice>,
    latest_notification: Option<Notice>,
    stats: BTreeMap<String, GuildStats>,
    guild_channels: BTreeMap<String, Vec<(String, u64)>>,
    message_subscriptions: BTreeSet<String>,
    widget_poll: BTreeMap<String, Instant>,
    pressed_stats: BTreeSet<String>,
}
impl ActionEngine {
    pub fn new() -> Self {
        Self::default()
    }
    /// Requirements are proposed to the user; this never starts authorization.
    pub fn required_scopes(&self) -> Vec<String> {
        let mut scopes: BTreeSet<String> = ["rpc", "identify", "rpc.voice.read", "rpc.voice.write"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for slot in self.slots.values() {
            scopes.extend(feature_scopes(&slot.action).iter().map(|s| (*s).to_owned()));
        }
        scopes.into_iter().collect()
    }
    /// Check actual AUTHENTICATE grants, not merely the saved requested scopes.
    pub fn missing_scopes(&self, context: &str) -> Vec<String> {
        self.slots
            .get(context)
            .into_iter()
            .flat_map(|slot| feature_scopes(&slot.action))
            .filter(|scope| !self.scopes.contains(**scope))
            .map(|scope| (*scope).to_owned())
            .collect()
    }
    pub fn connection(&mut self, connected: bool) -> Vec<Effect> {
        let mut cleanup = vec![];
        for context in self.held.keys().cloned().collect::<Vec<_>>() {
            cleanup.extend(self.release(&context));
        }
        self.connected = connected;
        self.voice = Value::Null;
        self.selected = Value::Null;
        self.guilds = Value::Null;
        self.channels.clear();
        self.channel_servers.clear();
        self.users.clear();
        self.speaker = None;
        self.errors.clear();
        self.subscription_errors.clear();
        self.targets.clear();
        self.scopes.clear();
        self.notifications.clear();
        self.latest_notification = None;
        self.stats.clear();
        self.guild_channels.clear();
        self.message_subscriptions.clear();
        self.widget_poll.clear();
        self.pressed_stats.clear();
        let mut effects = cleanup;
        effects.extend(self.refresh());
        if connected {
            for cmd in [
                "GET_VOICE_SETTINGS",
                "GET_SELECTED_VOICE_CHANNEL",
                "GET_GUILDS",
            ] {
                effects.push(rpc(cmd, json!({}), None));
            }
            for (context, slot) in &self.slots {
                effects.extend(self.queries(context, slot));
            }
        }
        effects
    }
    pub fn host_event(&mut self, event: &Value) -> Vec<Effect> {
        let name = string(event, "event", "");
        let context = string(event, "context", "");
        if context.is_empty() {
            return vec![];
        }
        let payload = &event["payload"];
        if name == "willDisappear" {
            let mut cleanup = self.release(context);
            self.slots.remove(context);
            self.channels.remove(context);
            self.channel_servers.remove(context);
            self.errors.remove(context);
            self.subscription_errors.remove(context);
            self.pressed_stats.remove(context);
            cleanup.extend(self.sync_subscriptions());
            return cleanup;
        }
        if matches!(
            name,
            "willAppear" | "didReceiveSettings" | "propertyInspectorDidAppear"
        ) {
            if let Some(action) = event["action"].as_str() {
                let previous = self.slots.get(context);
                let settings = payload
                    .get("settings")
                    .cloned()
                    .or_else(|| previous.map(|s| s.settings.clone()))
                    .unwrap_or(json!({}));
                let encoder = payload["controller"].as_str() == Some("Encoder")
                    || previous.is_some_and(|s| s.encoder);
                if name == "didReceiveSettings"
                    && previous.is_some_and(|old| old.settings != settings)
                    && !self.subscription_errors.contains(context)
                {
                    self.errors.remove(context);
                }
                if previous.is_some_and(|old| old.settings["server"] != settings["server"]) {
                    self.channels.remove(context);
                    self.channel_servers.remove(context);
                }
                self.slots.insert(
                    context.into(),
                    Slot {
                        action: action.into(),
                        settings,
                        encoder,
                    },
                );
            }
            let mut out = self.render(context);
            if self.connected
                && let Some(slot) = self.slots.get(context)
            {
                out.extend(self.queries(context, slot));
            }
            out.extend(self.sync_subscriptions());
            out.extend(self.tick());
            return out;
        }
        if name == "sendToPlugin" {
            return self.render(context);
        }
        if !matches!(
            name,
            "keyDown" | "keyUp" | "dialDown" | "dialUp" | "dialRotate" | "touchTap"
        ) {
            return vec![];
        }
        if let Some(slot) = self.slots.get(context).cloned()
            && matches!(
                slot.action.as_str(),
                "com.elgato.discord.notifications" | "com.elgato.discord.serverstats"
            )
        {
            if name == "keyDown" {
                self.pressed_stats.insert(context.into());
                return vec![];
            }
            if name == "keyUp" && self.pressed_stats.remove(context) {
                return self.stats_press(context, &slot);
            }
            return vec![];
        }
        // Regular buttons activate once on down; held operations have separate handling.
        if matches!(name, "keyUp" | "dialUp") {
            return self.release(context);
        }
        let Some(slot) = self.slots.get(context).cloned() else {
            return vec![];
        };
        if !self.connected {
            return self.fail(
                context,
                "Connect and authorize Discord in this action's settings.",
            );
        }
        if !self.missing_scopes(context).is_empty() {
            return self.fail(context, "Additional Discord access is required. Choose Authorize required access in this action's settings.");
        }
        self.errors.remove(context);
        if matches!(
            slot.action.as_str(),
            "com.elgato.discord.pushto.talk" | "com.elgato.discord.pushto.mute"
        ) {
            return self.press_held(context, &slot);
        }
        match self.activate(context, &slot, name, payload) {
            Ok(Some(effect)) => vec![effect],
            Ok(None) => vec![],
            Err(error) => self.fail(context, &error),
        }
    }
    /// Read-only cache maintenance; no work is scheduled more frequently than needed.
    pub fn tick(&mut self) -> Vec<Effect> {
        self.tick_at(Instant::now())
    }
    fn tick_at(&mut self, now: Instant) -> Vec<Effect> {
        if !self.connected {
            return vec![];
        }
        let mut changed = false;
        for stats in self.stats.values_mut() {
            let before = stats.authors.len();
            stats
                .authors
                .retain(|_, at| now.saturating_duration_since(*at) <= Duration::from_secs(1200));
            changed |= before != stats.authors.len();
        }
        let mut out = if changed { self.refresh() } else { vec![] };
        let guilds = self.stats_guilds();
        self.widget_poll.retain(|guild, _| guilds.contains(guild));
        for guild in guilds {
            if self
                .widget_poll
                .get(&guild)
                .is_none_or(|at| now.saturating_duration_since(*at) >= Duration::from_secs(300))
            {
                self.widget_poll.insert(guild.clone(), now);
                out.push(Effect::GuildWidget { guild_id: guild });
            }
        }
        out
    }
    fn stats_guilds(&self) -> BTreeSet<String> {
        self.slots
            .values()
            .filter(|s| s.action == "com.elgato.discord.serverstats")
            .filter_map(|s| {
                s.settings["server"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            })
            .collect()
    }
    fn sync_subscriptions(&mut self) -> Vec<Effect> {
        let guilds = self.stats_guilds();
        let wanted: BTreeSet<String> = if self.connected && self.scopes.contains("messages.read") {
            self.guild_channels
                .iter()
                .filter(|(guild, _)| guilds.contains(*guild))
                .flat_map(|(_, channels)| {
                    channels
                        .iter()
                        .filter(|(_, kind)| matches!(kind, 0 | 11 | 12))
                        .map(|(id, _)| id.clone())
                })
                .collect()
        } else {
            BTreeSet::new()
        };
        let mut out = vec![];
        for channel in self.message_subscriptions.difference(&wanted) {
            out.push(Effect::Subscription {
                event: "MESSAGE_CREATE".into(),
                args: json!({"channel_id":channel}),
                context: None,
                subscribe: false,
            });
        }
        for channel in wanted.difference(&self.message_subscriptions) {
            let context = self.slots.iter().find_map(|(context, slot)| {
                let guild = slot.settings["server"].as_str()?;
                (slot.action == "com.elgato.discord.serverstats"
                    && self
                        .guild_channels
                        .get(guild)
                        .is_some_and(|channels| channels.iter().any(|(id, _)| id == channel)))
                .then(|| context.clone())
            });
            out.push(Effect::Subscription {
                event: "MESSAGE_CREATE".into(),
                args: json!({"channel_id":channel}),
                context,
                subscribe: true,
            });
        }
        self.message_subscriptions = wanted;
        out
    }
    fn notification(&mut self, data: &Value) {
        let Some(channel) = data["channel_id"].as_str().filter(|s| !s.is_empty()) else {
            return;
        };
        // Store only routing metadata: no notification body or message text is needed.
        let record = Notice {
            channel: channel.into(),
        };
        self.latest_notification = Some(record.clone());
        self.notifications.push_back(record);
        if self.notifications.len() > 1000 {
            self.notifications.pop_front();
        }
    }
    fn message(&mut self, data: &Value, now: Instant) {
        let Some(channel) = data["channel_id"].as_str() else {
            return;
        };
        let Some(guild) = self.guild_channels.iter().find_map(|(guild, channels)| {
            channels
                .iter()
                .any(|(id, _)| id == channel)
                .then_some(guild.clone())
        }) else {
            return;
        };
        if !self.stats_guilds().contains(&guild) {
            return;
        }
        let stats = self.stats.entry(guild).or_default();
        stats.messages = stats.messages.saturating_add(1);
        stats.last_channel = Some(channel.into());
        if let Some(author) = data["message"]["author"]["id"].as_str() {
            if stats.authors.len() >= 10_000
                && !stats.authors.contains_key(author)
                && let Some(oldest) = stats
                    .authors
                    .iter()
                    .min_by_key(|(_, at)| *at)
                    .map(|(id, _)| id.clone())
            {
                stats.authors.remove(&oldest);
            }
            stats.authors.insert(author.into(), now);
        }
    }
    fn stats_press(&mut self, context: &str, slot: &Slot) -> Vec<Effect> {
        if !self.connected {
            return self.fail(context, "Connect and authorize Discord first.");
        }
        if !self.missing_scopes(context).is_empty() {
            return self.fail(context, "Additional Discord access is required. Choose Authorize required access in this action's settings.");
        }
        if !self.subscription_errors.contains(context) {
            self.errors.remove(context);
        }
        let mut out = vec![];
        let channel = if slot.action == "com.elgato.discord.notifications" {
            if !self.scopes.contains("rpc.notifications.read") {
                return self.fail(
                    context,
                    "Authorize the rpc.notifications.read scope to receive notifications.",
                );
            }
            match string(&slot.settings, "select-press", "open") {
                "nothing" => return vec![],
                "clear" => {
                    self.notifications.clear();
                    self.latest_notification = None;
                    out.push(host("showOk", context, json!({})));
                    None
                }
                "cycle-recent" => self.notifications.pop_back().map(|n| n.channel),
                "cycle-old" => self.notifications.pop_front().map(|n| n.channel),
                _ => {
                    let latest = self.latest_notification.take().map(|n| n.channel);
                    self.notifications.clear();
                    latest
                }
            }
        } else {
            let Ok(guild) = required(&slot.settings, "server") else {
                return self.fail(context, "Choose a server in this action's settings.");
            };
            // Linux policy honors the stored enum. The inspected Mac binary
            // ignores it; Windows parity for that quirk has not been established.
            let stats = self.stats.entry(guild.into()).or_default();
            match string(&slot.settings, "select-press", "nothing") {
                "nothing" => return vec![],
                "clear" => {
                    stats.messages = 0;
                    out.push(host("showOk", context, json!({})));
                    None
                }
                "open" => {
                    stats.messages = 0;
                    stats.last_channel.clone()
                }
                _ => return self.fail(context, "Unknown server press operation."),
            }
        };
        if let Some(channel) = channel {
            out.push(rpc(
                "SELECT_TEXT_CHANNEL",
                json!({"channel_id":channel}),
                Some(context),
            ));
        }
        out.extend(self.refresh());
        out
    }
    fn press_held(&mut self, context: &str, slot: &Slot) -> Vec<Effect> {
        if self.held.contains_key(context) {
            return vec![];
        }
        let mode = if slot.action.ends_with("pushto.mute") {
            Held::Mute
        } else {
            match self.voice["mode"]["type"].as_str() {
                Some("PUSH_TO_TALK") => Held::TalkPtt,
                Some("VOICE_ACTIVITY") => Held::TalkActivity,
                _ => return self.fail(context, "Current voice input mode is unavailable."),
            }
        };
        if mode == Held::TalkPtt && self.selected["id"].as_str().is_none() {
            return self.fail(context, "Join a voice channel before using Push to Talk.");
        }
        // Linux ownership policy: equal held operations share ownership. Conflicting
        // held controls are rejected instead of letting one release override another.
        if self.held.values().any(|m| *m != mode) {
            return self.fail(context, "Release the other held voice control first.");
        }
        let first = self.held.is_empty();
        self.held.insert(context.into(), mode);
        if !first {
            return self.render(context);
        }
        let mut out = vec![rpc(
            "SET_VOICE_SETTINGS",
            json!({"mute":mode==Held::Mute}),
            Some(context),
        )];
        if mode == Held::TalkPtt {
            out.push(rpc("PUSH_TO_TALK", json!({"active":true}), Some(context)));
        }
        out.extend(self.render(context));
        out
    }
    fn release(&mut self, context: &str) -> Vec<Effect> {
        let Some(mode) = self.held.remove(context) else {
            return vec![];
        };
        if self.held.values().any(|m| *m == mode) {
            return self.render(context);
        }
        let mut out = vec![rpc(
            "SET_VOICE_SETTINGS",
            json!({"mute":mode==Held::TalkActivity}),
            Some(context),
        )];
        if mode == Held::TalkPtt {
            out.push(rpc("PUSH_TO_TALK", json!({"active":false}), Some(context)));
        }
        out.extend(self.render(context));
        out
    }
    fn queries(&self, context: &str, slot: &Slot) -> Vec<Effect> {
        let mut out = vec![];
        if let Some(server) = slot.settings["server"].as_str().filter(|s| !s.is_empty()) {
            out.push(rpc(
                "GET_CHANNELS",
                json!({"guild_id":server}),
                Some(context),
            ));
        }
        out
    }
    fn activate(
        &mut self,
        context: &str,
        slot: &Slot,
        event: &str,
        payload: &Value,
    ) -> Result<Option<Effect>, String> {
        let suffix = slot
            .action
            .strip_prefix("com.elgato.discord.")
            .unwrap_or(&slot.action);
        let s = &slot.settings;
        let mut args = json!({});
        let mut cmd = "SET_VOICE_SETTINGS";
        match suffix {
            "mute" | "deafen" => {
                let key = if suffix == "mute" { "mute" } else { "deaf" };
                let old = self
                    .target(key, &self.voice[key])
                    .as_bool()
                    .ok_or("Discord has not supplied the current voice settings yet.")?;
                args[key] = json!(desired_bool(payload, old));
            }
            "pushtotalktoggle" => {
                let mode = self
                    .target("mode.type", &self.voice["mode"]["type"])
                    .as_str()
                    .ok_or("Discord has not supplied the current input mode yet.")?;
                args["mode"] = json!({"type": if mode == "PUSH_TO_TALK" {"VOICE_ACTIVITY"} else {"PUSH_TO_TALK"}});
            }
            "channel.voice" | "channel.text" => {
                let channel = s["channel"]
                    .as_str()
                    .ok_or("Choose a channel in this action's settings.")?;
                if suffix == "channel.text" && channel.is_empty() {
                    return Err("Choose a text channel.".into());
                }
                cmd = if suffix == "channel.voice" {
                    "SELECT_VOICE_CHANNEL"
                } else {
                    "SELECT_TEXT_CHANNEL"
                };
                args["channel_id"] = if channel.is_empty() {
                    Value::Null
                } else {
                    json!(channel)
                };
            }
            "setaudiodevice" => {
                let mode = string(s, "mode", "both");
                if !["input", "output", "both"].contains(&mode) {
                    return Err("Unknown audio device mode.".into());
                }
                for device in ["input", "output"] {
                    if mode == "both" || mode == device {
                        args[device] = json!({"device_id": required(s, device)?});
                    }
                }
            }
            "volumecontrol" | "volumecontrolbutton" => {
                let device = string(s, "device", "output");
                if !["input", "output"].contains(&device) {
                    return Err("Unknown volume device.".into());
                }
                if suffix == "volumecontrol" && event != "dialRotate" {
                    let key = if device == "input" { "mute" } else { "deaf" };
                    args[key] = json!(
                        !self
                            .target(key, &self.voice[key])
                            .as_bool()
                            .ok_or("Current mute state is unavailable.")?
                    );
                } else {
                    let limit = if device == "input" { 100.0 } else { 200.0 };
                    let value = volume(
                        s,
                        self.target(&format!("{device}.volume"), &self.voice[device]["volume"])
                            .as_f64(),
                        if suffix == "volumecontrol" {
                            Some((payload, "volValue"))
                        } else {
                            None
                        },
                        limit,
                    )?;
                    args[device] = json!({"volume":value});
                }
            }
            "uservolumecontrolbutton" | "uservolumecontroldial" => {
                let user = if suffix == "uservolumecontroldial"
                    && string(s, "user_mode", "any") == "any"
                {
                    self.speaker
                        .as_deref()
                        .ok_or("No recent speaker is available in the current voice channel.")?
                } else {
                    required(s, "user")?
                };
                let current = self.users.get(user).ok_or(
                    "The selected user's voice state is not available in the current channel.",
                )?;
                cmd = "SET_USER_VOICE_SETTINGS";
                args["user_id"] = json!(user);
                let dial = suffix == "uservolumecontroldial";
                if (dial && event != "dialRotate") || (!dial && string(s, "mode", "mute") == "mute")
                {
                    let mute = match string(s, "muteType", "toggle") {
                        "mute" => true,
                        "unmute" => false,
                        "toggle" => desired_bool(
                            payload,
                            self.target(&format!("user:{user}.mute"), &current["mute"])
                                .as_bool()
                                .ok_or("Current user mute state is unavailable.")?,
                        ),
                        _ => return Err("Unknown mute operation.".into()),
                    };
                    args["mute"] = json!(mute);
                } else {
                    args["volume"] = json!(volume(
                        s,
                        self.target(&format!("user:{user}.volume"), &current["volume"])
                            .as_f64(),
                        if dial {
                            Some((payload, "volume"))
                        } else {
                            None
                        },
                        200.0
                    )?);
                }
            }

            "pushto.talk" | "pushto.mute" => {
                return Err(
                    "Held voice control is awaiting the verified Discord RPC contract.".into(),
                );
            }
            "soundboard" => {
                cmd = "PLAY_SOUNDBOARD_SOUND";
                args =
                    json!({"guild_id":required(s,"guild_id")?,"sound_id":required(s,"sound_id")?});
            }
            "videotoggle" => {
                cmd = "TOGGLE_VIDEO";
                args = json!({"active":true});
            }
            "streamtoggle" => {
                cmd = "TOGGLE_SCREENSHARE";
            }
            _ => return Err("Unknown Discord action.".into()),
        }
        // Pending targets accumulate rapid dial turns without displaying unconfirmed
        // values. A matching response clears the target; errors clear all targets.
        for (key, value) in target_fields(cmd, &args) {
            self.targets.insert(key, value);
        }
        Ok(Some(rpc(cmd, args, Some(context))))
    }
    pub fn rpc_event(&mut self, event: &Value) -> Vec<Effect> {
        let cmd = string(event, "cmd", "");
        let evt = string(event, "evt", "");
        let data = &event["data"];
        if evt == "ERROR" {
            if cmd.starts_with("SET_") {
                self.targets.clear();
            }
            if cmd == "SUBSCRIBE" {
                // The original slot can disappear or switch guild while a reply
                // is in flight. The subscribed channel is the stable ownership key.
                let guild = event["_args"]["channel_id"]
                    .as_str()
                    .and_then(|channel| {
                        self.guild_channels.iter().find_map(|(guild, channels)| {
                            channels
                                .iter()
                                .any(|(id, _)| id == channel)
                                .then_some(guild.as_str())
                        })
                    })
                    .or_else(|| {
                        event["_context"]
                            .as_str()
                            .and_then(|context| self.slots.get(context))
                            .filter(|slot| slot.action == "com.elgato.discord.serverstats")
                            .and_then(|slot| slot.settings["server"].as_str())
                    });
                let notices = event["_subscription_event"].as_str() == Some("NOTIFICATION_CREATE");
                let contexts: Vec<String> = self
                    .slots
                    .iter()
                    .filter(|(_, slot)| {
                        (notices && slot.action == "com.elgato.discord.notifications")
                            || (guild.is_some()
                                && slot.action == "com.elgato.discord.serverstats"
                                && slot.settings["server"].as_str() == guild)
                    })
                    .map(|(context, _)| context.clone())
                    .collect();
                if !contexts.is_empty() {
                    let message = format!(
                        "Discord rejected SUBSCRIBE (code {}). Review application access and reconnect.",
                        data["code"]
                    );
                    let mut effects = vec![];
                    for context in contexts {
                        self.subscription_errors.insert(context.clone());
                        effects.extend(self.fail(&context, &message));
                    }
                    return effects;
                }
            }
            if let Some(context) = event["_context"].as_str() {
                if cmd == "SUBSCRIBE" {
                    self.subscription_errors.insert(context.into());
                }
                // Do not print response bodies: third-party errors can contain private data.
                return self.fail(context, &format!("Discord rejected {cmd} (code {}). Check authorization scopes and action settings.", data["code"]));
            }
            return vec![];
        }
        for (key, value) in target_fields(cmd, data) {
            if self.targets.get(&key).is_some_and(|v| {
                v == &value || (v.is_number() && value.is_number() && v.as_f64() == value.as_f64())
            }) {
                self.targets.remove(&key);
            }
        }
        let mut extra = vec![];
        match (cmd, evt) {
            ("AUTHENTICATE", _) => {
                self.scopes = data["scopes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                extra.extend(self.sync_subscriptions());
                extra.extend(self.tick());
            }
            ("DECKFORGE_GUILD_WIDGET", _) => {
                if let Some(id) = data["guild_id"].as_str() {
                    self.stats.entry(id.into()).or_default().online =
                        data["presence_count"].as_u64();
                }
            }
            (_, "NOTIFICATION_CREATE") => self.notification(data),
            (_, "MESSAGE_CREATE") => self.message(data, Instant::now()),

            ("GET_VOICE_SETTINGS" | "SET_VOICE_SETTINGS", _) | (_, "VOICE_SETTINGS_UPDATE") => {
                merge(&mut self.voice, data)
            }
            ("GET_GUILDS", _) => self.guilds = data["guilds"].clone(),
            (_, "GUILD_CREATE") => {
                if let Some(id) = data["id"].as_str() {
                    if !self.guilds.is_array() {
                        self.guilds = json!([]);
                    }
                    let guilds = self.guilds.as_array_mut().unwrap();
                    if let Some(old) = guilds.iter_mut().find(|g| g["id"].as_str() == Some(id)) {
                        *old = data.clone();
                    } else {
                        guilds.push(data.clone());
                    }
                }
            }
            (_, "CHANNEL_CREATE") => {
                for (context, slot) in &self.slots {
                    extra.extend(self.queries(context, slot));
                }
            }

            ("GET_CHANNELS", _) => {
                if let Some(context) = event["_context"].as_str() {
                    let requested = event["_args"]["guild_id"].as_str();
                    let current = self
                        .slots
                        .get(context)
                        .and_then(|s| s.settings["server"].as_str());
                    if let Some(requested) = requested
                        && Some(requested) == current
                    {
                        self.channels
                            .insert(context.into(), data["channels"].clone());
                        self.channel_servers
                            .insert(context.into(), requested.into());
                        let channels = data["channels"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| {
                                Some((v["id"].as_str()?.to_owned(), v["type"].as_u64()?))
                            })
                            .collect();
                        self.guild_channels.insert(requested.into(), channels);
                        extra.extend(self.sync_subscriptions());
                    }
                }
            }
            ("GET_SELECTED_VOICE_CHANNEL" | "SELECT_VOICE_CHANNEL", _) => self.set_channel(data),
            (_, "VOICE_CHANNEL_SELECT") => {
                self.selected = data.clone();
                self.users.clear();
                self.speaker = None;
                if let Some(id) = data["channel_id"].as_str() {
                    extra.push(rpc("GET_CHANNEL", json!({"channel_id":id}), None));
                }
            }
            ("GET_CHANNEL", _) => {
                // A delayed details response must not select a channel that a
                // newer VOICE_CHANNEL_SELECT event has already replaced.
                if data["id"].as_str().is_some_and(|id| {
                    self.selected["channel_id"]
                        .as_str()
                        .or(self.selected["id"].as_str())
                        == Some(id)
                }) {
                    self.set_channel(data);
                }
            }
            ("SET_USER_VOICE_SETTINGS", _) => {
                if let Some(id) = data["user_id"].as_str() {
                    merge(self.users.entry(id.into()).or_default(), data);
                }
            }
            (_, "VOICE_STATE_CREATE" | "VOICE_STATE_UPDATE") => {
                if let Some(id) = data["user"]["id"].as_str() {
                    merge(self.users.entry(id.into()).or_default(), data);
                }
            }
            (_, "VOICE_STATE_DELETE") => {
                if let Some(id) = data["user"]["id"].as_str() {
                    self.users.remove(id);
                    if self.speaker.as_deref() == Some(id) {
                        self.speaker = None;
                    }
                }
            }
            (_, "SPEAKING_START") => self.speaker = data["user_id"].as_str().map(str::to_owned),
            _ => {}
        }
        if !cmd.starts_with("GET_")
            && let Some(context) = event["_context"].as_str()
            && !self.subscription_errors.contains(context)
        {
            self.errors.remove(context);
        }
        extra.extend(self.refresh());
        extra
    }
    fn target<'a>(&'a self, key: &str, fallback: &'a Value) -> &'a Value {
        self.targets.get(key).unwrap_or(fallback)
    }
    fn set_channel(&mut self, data: &Value) {
        self.selected = data.clone();
        self.users.clear();
        self.speaker = None;
        if let Some(states) = data["voice_states"].as_array() {
            for state in states {
                if let Some(id) = state["user"]["id"].as_str() {
                    self.users.insert(id.into(), state.clone());
                }
            }
        }
    }
    fn fail(&mut self, context: &str, error: &str) -> Vec<Effect> {
        self.errors.insert(context.into(), error.into());
        let mut out = vec![host("showAlert", context, json!({}))];
        out.extend(self.render(context));
        out
    }
    fn refresh(&self) -> Vec<Effect> {
        self.slots.keys().flat_map(|c| self.render(c)).collect()
    }
    fn render(&self, context: &str) -> Vec<Effect> {
        let Some(slot) = self.slots.get(context) else {
            return vec![];
        };
        let s = &slot.settings;
        let suffix = slot
            .action
            .strip_prefix("com.elgato.discord.")
            .unwrap_or("");
        let state = match suffix {
            "mute" => self.voice["mute"].as_bool(),
            "deafen" => self.voice["deaf"].as_bool(),
            "uservolumecontrolbutton" if string(s, "mode", "mute") == "mute" => self
                .users
                .get(string(s, "user", ""))
                .and_then(|u| u["mute"].as_bool()),
            "volumecontrolbutton" => self.voice[if string(s, "device", "output") == "input" {
                "mute"
            } else {
                "deaf"
            }]
            .as_bool(),
            "pushtotalktoggle" => self.voice["mode"]["type"]
                .as_str()
                .map(|m| m == "PUSH_TO_TALK"),
            // This is a local held-gesture indicator, not an acknowledgement
            // that Discord has applied the requested remote mute/PTT operation.
            "pushto.talk" | "pushto.mute" => {
                Some(self.connected && self.held.contains_key(context))
            }
            "channel.voice" => s["channel"].as_str().filter(|id| !id.is_empty()).map(|id| {
                self.connected
                    && self.selected["id"]
                        .as_str()
                        .or(self.selected["channel_id"].as_str())
                        == Some(id)
            }),
            _ => None,
        };
        let users:Vec<Value>=self.users.iter().map(|(id,u)|json!({"id":id,"name":u["nick"].as_str().or(u["user"]["username"].as_str()).unwrap_or(id)})).collect();
        let missing_scopes = self.missing_scopes(context);
        let scope_warning = if self.connected && !missing_scopes.is_empty() {
            Some("Authorize the additional access required by this action.")
        } else {
            None
        };
        let payload = json!({"warning":scope_warning,"missing_scopes":missing_scopes,"required_scopes":self.required_scopes(),"notificationCount":self.notifications.len(),"disabled":!self.connected,"unauthorized":!self.connected,"error":self.errors.get(context),"servers":self.guilds,"channels":self.channels.get(context),"channels_server":self.channel_servers.get(context),"users":users,"inputs":self.voice["input"]["available_devices"],"outputs":self.voice["output"]["available_devices"],"_currentVoice":self.selected,"voice":self.voice});
        let mut out = vec![Effect::Host(
            json!({"event":"sendToPropertyInspector","context":context,"action":slot.action,"payload":payload}),
        )];
        // State zero is the baseline artwork when observation is unavailable.
        // Reset it after disconnect/settings changes rather than retaining a
        // stale active icon. No video/share/text-selection state is inferred.
        out.push(host(
            "setState",
            context,
            json!({"state":u8::from(state.unwrap_or(false))}),
        ));
        if suffix == "notifications" {
            let title = if !self.connected {
                "Offline".into()
            } else if scope_warning.is_some() {
                "Authorize".into()
            } else if self.errors.contains_key(context) {
                "Unavailable".into()
            } else {
                self.notifications.len().to_string()
            };
            out.push(host("setTitle", context, json!({"title":title,"target":0})));
        } else if suffix == "serverstats" {
            let stats = self.stats.get(string(s, "server", ""));
            let mut lines = vec![];
            if string(s, "select-messages", "nothing") == "display" {
                lines.push(format!("{} messages", stats.map_or(0, |s| s.messages)));
            }
            match string(s, "select-users", "nothing") {
                "active" => lines.push(format!("{} active", stats.map_or(0, |s| s.authors.len()))),
                "online" => lines.push(
                    stats
                        .and_then(|s| s.online)
                        .map(|n| format!("{n} online"))
                        .unwrap_or("Online unavailable".into()),
                ),
                _ => {}
            }
            out.push(host("setTitle",context,json!({"title":if !self.connected {"Offline".into()} else if scope_warning.is_some() {"Authorize".into()} else if self.errors.contains_key(context) {"Unavailable".into()} else {lines.join("\n")},"target":0})));
        }
        if !slot.encoder && matches!(suffix, "volumecontrolbutton" | "uservolumecontrolbutton") {
            let style = string(s, "icon", "static");
            let user = suffix == "uservolumecontrolbutton";
            let dynamic = matches!(style, "dynamic_h" | "dynamic_v")
                && self.connected
                && (!user || string(s, "mode", "mute") != "mute");
            let image = if dynamic {
                let device = string(s, "device", "output");
                let current = if user {
                    self.users
                        .get(string(s, "user", ""))
                        .unwrap_or(&Value::Null)
                } else {
                    &self.voice[device]
                };
                let value = current["volume"].as_f64();
                let muted = if user {
                    current["mute"].as_bool()
                } else {
                    self.voice[if device == "input" { "mute" } else { "deaf" }].as_bool()
                }
                .unwrap_or(false);
                let label = if user {
                    string(s, "usernick", "User volume")
                } else if device == "input" {
                    "Microphone"
                } else {
                    "Discord output"
                };
                crate::graphics::fader(
                    value,
                    if !user && device == "input" {
                        100.0
                    } else {
                        200.0
                    },
                    style == "dynamic_v",
                    muted,
                    label,
                )
            } else {
                String::new()
            };
            out.push(host("setImage", context, json!({"image":image,"target":0})));
        }
        if slot.encoder {
            let device = string(s, "device", "output");
            let value = if suffix == "uservolumecontroldial" {
                let id = if string(s, "user_mode", "any") == "any" {
                    self.speaker.as_deref().unwrap_or("")
                } else {
                    string(s, "user", "")
                };
                self.users.get(id).and_then(|v| v["volume"].as_f64())
            } else {
                self.voice[device]["volume"].as_f64()
            };
            let label = if !self.connected {
                "Disconnected".into()
            } else {
                value
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or("Unavailable".into())
            };
            out.push(host("setFeedback",context,json!({"title":if suffix=="uservolumecontroldial" {"User volume"}else if device=="input" {"Microphone"}else{"Discord output"},"value":label,"indicator":value.unwrap_or(0.0)})));
        }
        out
    }
}
fn feature_scopes(action: &str) -> &'static [&'static str] {
    match action {
        "com.elgato.discord.notifications" => &["rpc.notifications.read"],
        "com.elgato.discord.serverstats" => &["messages.read"],
        "com.elgato.discord.videotoggle" => &["rpc.video.read", "rpc.video.write"],
        "com.elgato.discord.streamtoggle" => &["rpc.screenshare.read", "rpc.screenshare.write"],
        _ => &[],
    }
}
fn target_fields(cmd: &str, args: &Value) -> Vec<(String, Value)> {
    let mut out = vec![];
    if cmd == "SET_VOICE_SETTINGS" {
        if let Some(value) = args["mode"].get("type") {
            out.push(("mode.type".into(), value.clone()));
        }
        for key in ["mute", "deaf"] {
            if let Some(value) = args.get(key) {
                out.push((key.into(), value.clone()));
            }
        }
        for device in ["input", "output"] {
            if let Some(value) = args[device].get("volume") {
                out.push((format!("{device}.volume"), value.clone()));
            }
        }
    } else if cmd == "SET_USER_VOICE_SETTINGS"
        && let Some(id) = args["user_id"].as_str()
    {
        for key in ["mute", "volume"] {
            if let Some(value) = args.get(key) {
                out.push((format!("user:{id}.{key}"), value.clone()));
            }
        }
    }
    out
}
fn rpc(cmd: &str, args: Value, context: Option<&str>) -> Effect {
    Effect::Rpc {
        cmd: cmd.into(),
        args,
        context: context.map(str::to_owned),
    }
}
fn host(event: &str, context: &str, payload: Value) -> Effect {
    Effect::Host(json!({"event":event,"context":context,"payload":payload}))
}
fn string<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v[key].as_str().unwrap_or(default)
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Choose {key} in this action's settings."))
}
fn integer(v: &Value, key: &str, default: i64, min: i64, max: i64) -> Result<i64, String> {
    let n = match v.get(key) {
        None => default,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| format!("{key} must be an integer."))?,
    };
    if (min..=max).contains(&n) {
        Ok(n)
    } else {
        Err(format!("{key} must be between {min} and {max}."))
    }
}
fn desired_bool(payload: &Value, current: bool) -> bool {
    payload["userDesiredState"]
        .as_u64()
        .filter(|s| *s <= 1)
        .map(|s| s == 1)
        .unwrap_or(!current)
}
fn volume(
    s: &Value,
    current: Option<f64>,
    dial: Option<(&Value, &str)>,
    limit: f64,
) -> Result<f64, String> {
    if dial.is_none() && string(s, "mode", "adjust") == "set" {
        return Ok((integer(s, "volSetValue", 100, 0, 200)? as f64).min(limit));
    }
    if dial.is_none() && string(s, "mode", "adjust") != "adjust" {
        return Err("Unknown volume operation.".into());
    }
    let delta = if let Some((payload, key)) = dial {
        let ticks = payload["ticks"]
            .as_i64()
            .filter(|t| (-127..=127).contains(t))
            .ok_or("Invalid dial tick count.")?;
        ticks * integer(s, key, 5, 1, 5)?
    } else {
        let n = integer(s, "volAdjustValue", 4, -25, 24)?;
        if n >= 0 { n + 1 } else { n }
    };
    let old = current
        .filter(|v| v.is_finite())
        .ok_or("Current volume is unavailable. Wait for Discord's voice settings.")?;
    Ok((old + delta as f64).clamp(0.0, limit))
}
fn merge(target: &mut Value, source: &Value) {
    if let Some(object) = source.as_object() {
        if !target.is_object() {
            *target = json!({});
        }
        for (k, v) in object {
            if v.is_object() {
                merge(&mut target[k], v)
            } else {
                target[k] = v.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pickers_clear_on_server_change_and_session_reset_and_keep_response_provenance() {
        let mut engine = ready("channel.voice", json!({"server":"first"}));
        engine.rpc_event(
            &json!({"cmd":"GET_GUILDS","data":{"guilds":[{"id":"first","name":"Old server"}]}}),
        );
        engine.rpc_event(&json!({"cmd":"GET_CHANNELS","_context":"a","_args":{"guild_id":"first"},"data":{"channels":[{"id":"old","type":2}]}}));
        assert_eq!(engine.channel_servers["a"], "first");
        engine.host_event(&json!({"event":"didReceiveSettings","action":"com.elgato.discord.channel.voice","context":"a","payload":{"settings":{"server":"second"}}}));
        assert!(!engine.channels.contains_key("a"));
        assert!(!engine.channel_servers.contains_key("a"));
        engine.rpc_event(&json!({"cmd":"GET_CHANNELS","_context":"a","_args":{"guild_id":"first"},"data":{"channels":[{"id":"late-old","type":2}]}}));
        assert!(!engine.channels.contains_key("a"));
        let effects = engine.rpc_event(&json!({"cmd":"GET_CHANNELS","_context":"a","_args":{"guild_id":"second"},"data":{"channels":[{"id":"new","type":2}]}}));
        let payload = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Host(value) if value["event"] == "sendToPropertyInspector" => {
                    Some(&value["payload"])
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(payload["channels_server"], "second");
        assert_eq!(payload["channels"][0]["id"], "new");
        engine.connection(false);
        assert!(engine.guilds.is_null());
        assert!(engine.channels.is_empty() && engine.channel_servers.is_empty());
        engine.connection(true);
        assert!(engine.guilds.is_null());
        assert!(engine.channels.is_empty() && engine.channel_servers.is_empty());
    }
    #[test]
    fn editing_settings_clears_obsolete_action_errors_but_keeps_subscription_failures() {
        let mut engine = ready("serverstats", json!({}));
        engine.errors.insert(
            "a".into(),
            "Choose a server in this action's settings.".into(),
        );
        let update = |server: &str| json!({"event":"didReceiveSettings","action":"com.elgato.discord.serverstats","context":"a","payload":{"settings":{"server":server}}});
        engine.host_event(&update("configured"));
        assert!(!engine.errors.contains_key("a"));
        engine.subscription_errors.insert("a".into());
        engine
            .errors
            .insert("a".into(), "Discord denied subscription".into());
        engine.host_event(&update("other"));
        assert!(engine.errors.contains_key("a"));
    }
    #[test]
    fn requirements_follow_active_actions_without_starting_authorization() {
        let mut e = ActionEngine::new();
        let base = e.required_scopes();
        assert!(!base.contains(&"messages.read".to_owned()));
        let effects=e.host_event(&json!({"event":"willAppear","context":"stats","action":"com.elgato.discord.serverstats","payload":{"settings":{"server":"guild"}}}));
        assert!(effects.iter().all(|e| !matches!(e, Effect::Rpc { .. })));
        assert!(e.required_scopes().contains(&"messages.read".to_owned()));
        assert_eq!(e.missing_scopes("stats"), vec!["messages.read"]);
        e.host_event(&json!({"event":"willDisappear","context":"stats"}));
        assert_eq!(e.required_scopes(), base);
    }
    #[test]
    fn message_subscriptions_wait_for_actual_granted_permission() {
        let mut e = ready(
            "serverstats",
            json!({"server":"guild","select-messages":"display","select-press":"open"}),
        );
        e.rpc_event(&json!({"cmd":"AUTHENTICATE","data":{"scopes":["rpc"]}}));
        assert!(channel_list(&mut e).iter().all(|e| !matches!(
            e,
            Effect::Subscription {
                subscribe: true,
                ..
            }
        )));
        assert_eq!(e.missing_scopes("a"), vec!["messages.read"]);
        assert!(commands(click(&mut e)).is_empty());
        let effects =
            e.rpc_event(&json!({"cmd":"AUTHENTICATE","data":{"scopes":["rpc","messages.read"]}}));
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(
                    e,
                    Effect::Subscription {
                        subscribe: true,
                        ..
                    }
                ))
                .count(),
            3
        );
        assert!(e.missing_scopes("a").is_empty());
    }
    #[test]
    fn denied_subscription_stays_unavailable_after_other_channel_success() {
        let mut e = ready(
            "serverstats",
            json!({"server":"guild","select-messages":"display","select-press":"clear"}),
        );
        channel_list(&mut e);
        e.rpc_event(&json!({"cmd":"SUBSCRIBE","evt":"ERROR","_context":"a","data":{"code":4006}}));
        e.rpc_event(&json!({"cmd":"SUBSCRIBE","_context":"a","data":{}}));
        assert!(e.subscription_errors.contains("a"));
        click(&mut e);
        assert!(format!("{:?}", e.render("a")).contains("Unavailable"));
        e.connection(false);
        assert!(e.subscription_errors.is_empty());
    }
    #[test]
    fn subscription_denial_marks_every_affected_action() {
        let mut e = ready("serverstats", json!({"server":"guild"}));
        e.host_event(&json!({"event":"willAppear","context":"b","action":"com.elgato.discord.serverstats","payload":{"settings":{"server":"guild"}}}));
        e.rpc_event(&json!({"cmd":"SUBSCRIBE","evt":"ERROR","_context":"a","data":{"code":4006}}));
        assert!(e.subscription_errors.contains("a") && e.subscription_errors.contains("b"));
        for context in ["n1", "n2"] {
            e.host_event(&json!({"event":"willAppear","context":context,"action":"com.elgato.discord.notifications","payload":{"settings":{}}}));
        }
        e.rpc_event(&json!({"cmd":"SUBSCRIBE","evt":"ERROR","_subscription_event":"NOTIFICATION_CREATE","data":{"code":4006}}));
        assert!(e.subscription_errors.contains("n1") && e.subscription_errors.contains("n2"));
    }
    #[test]
    fn subscription_denial_follows_channel_when_original_slot_changed() {
        let mut e = ready("serverstats", json!({"server":"guild"}));
        channel_list(&mut e);
        e.host_event(&json!({"event":"willAppear","context":"b","action":"com.elgato.discord.serverstats","payload":{"settings":{"server":"guild"}}}));
        e.host_event(&json!({"event":"didReceiveSettings","context":"a","action":"com.elgato.discord.serverstats","payload":{"settings":{"server":"other"}}}));
        e.rpc_event(&json!({"cmd":"SUBSCRIBE","evt":"ERROR","_context":"a","_subscription_event":"MESSAGE_CREATE","_args":{"channel_id":"text"},"data":{"code":4006}}));
        assert!(e.subscription_errors.contains("b"));
        assert!(!e.subscription_errors.contains("a"));
    }
    fn ready(action: &str, settings: Value) -> ActionEngine {
        let mut e = ActionEngine::new();
        e.connection(true);
        e.host_event(&json!({"event":"willAppear","context":"a","action":format!("com.elgato.discord.{action}"),"payload":{"settings":settings}}));
        e.rpc_event(&json!({"cmd":"AUTHENTICATE","data":{"scopes":e.required_scopes()}}));
        e.rpc_event(&json!({"cmd":"GET_VOICE_SETTINGS","data":{"mute":false,"deaf":true,"input":{"volume":98},"output":{"volume":100},"mode":{"type":"VOICE_ACTIVITY"}}}));
        e
    }
    fn args(e: &mut ActionEngine, event: &str, payload: Value) -> Value {
        e.host_event(&json!({"event":event,"context":"a","payload":payload}))
            .into_iter()
            .find_map(|v| {
                if let Effect::Rpc { args, .. } = v {
                    Some(args)
                } else {
                    None
                }
            })
            .unwrap()
    }
    fn icon_state(effects: &[Effect], context: &str) -> Option<u64> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Host(value)
                    if value["event"] == "setState" && value["context"] == context =>
                {
                    value["payload"]["state"].as_u64()
                }
                _ => None,
            })
            .next_back()
    }
    #[test]
    fn every_manifest_action_disables_automatic_state_alternation() {
        let manifest: Value = serde_json::from_str(include_str!("../manifest.json")).unwrap();
        let actions = manifest["Actions"].as_array().unwrap();
        assert_eq!(actions.len(), 17);
        for action in actions {
            assert_eq!(action["DisableAutomaticStates"], true, "{}", action["UUID"]);
            assert_eq!(action["States"].as_array().unwrap().len(), 2);
        }
    }
    #[test]
    fn reported_voice_states_drive_icons_and_disconnect_clears_stale_active() {
        for (action, settings, field) in [
            ("mute", json!({}), "mute"),
            ("deafen", json!({}), "deaf"),
            ("volumecontrolbutton", json!({"device":"input"}), "mute"),
            ("volumecontrolbutton", json!({"device":"output"}), "deaf"),
        ] {
            let mut e = ready(action, settings);
            let effects = e.rpc_event(&json!({"evt":"VOICE_SETTINGS_UPDATE","data":{field:true}}));
            assert_eq!(icon_state(&effects, "a"), Some(1));
            let down = e.host_event(&json!({"event":"keyDown","context":"a"}));
            assert_eq!(icon_state(&down, "a"), None); // Request is not confirmation.
            assert_eq!(icon_state(&e.render("a"), "a"), Some(1));
            let effects = e.rpc_event(&json!({"evt":"VOICE_SETTINGS_UPDATE","data":{field:false}}));
            assert_eq!(icon_state(&effects, "a"), Some(0));
            e.rpc_event(&json!({"evt":"VOICE_SETTINGS_UPDATE","data":{field:true}}));
            assert_eq!(icon_state(&e.connection(false), "a"), Some(0));
        }
        let mut e = ready("pushtotalktoggle", json!({}));
        assert_eq!(
            icon_state(
                &e.rpc_event(
                    &json!({"evt":"VOICE_SETTINGS_UPDATE","data":{"mode":{"type":"PUSH_TO_TALK"}}})
                ),
                "a"
            ),
            Some(1)
        );
        assert_eq!(icon_state(&e.rpc_event(&json!({"evt":"VOICE_SETTINGS_UPDATE","data":{"mode":{"type":"VOICE_ACTIVITY"}}})),"a"),Some(0));
        let mut e = ready(
            "uservolumecontrolbutton",
            json!({"user":"42","mode":"mute"}),
        );
        assert_eq!(icon_state(&e.rpc_event(&json!({"cmd":"GET_SELECTED_VOICE_CHANNEL","data":{"id":"v","voice_states":[{"user":{"id":"42"},"mute":true}]}})),"a"),Some(1));
        assert_eq!(
            icon_state(
                &e.rpc_event(
                    &json!({"cmd":"SET_USER_VOICE_SETTINGS","data":{"user_id":"42","mute":false}})
                ),
                "a"
            ),
            Some(0)
        );
    }
    #[test]
    fn held_icons_follow_each_local_gesture_and_cleanup_not_blind_toggle() {
        for action in ["pushto.talk", "pushto.mute"] {
            let mut e = ready(action, json!({}));
            e.host_event(&json!({"event":"willAppear","context":"b","action":format!("com.elgato.discord.{action}"),"payload":{"settings":{}}}));
            for context in ["a", "b"] {
                assert_eq!(
                    icon_state(
                        &e.host_event(&json!({"event":"keyDown","context":context})),
                        context
                    ),
                    Some(1)
                );
                assert!(
                    e.host_event(&json!({"event":"keyDown","context":context}))
                        .is_empty()
                );
            }
            assert_eq!(
                icon_state(&e.host_event(&json!({"event":"keyUp","context":"a"})), "a"),
                Some(0)
            );
            assert_eq!(icon_state(&e.render("b"), "b"), Some(1));
            assert_eq!(
                icon_state(
                    &e.host_event(&json!({"event":"willDisappear","context":"b"})),
                    "b"
                ),
                Some(0)
            );
            e.host_event(&json!({"event":"keyDown","context":"a"}));
            assert_eq!(icon_state(&e.connection(false), "a"), Some(0));
        }
    }
    #[test]
    fn voice_selection_uses_observed_channel_and_ignores_late_details() {
        let mut e = ready("channel.voice", json!({"channel":"chosen"}));
        assert_eq!(icon_state(&e.render("a"), "a"), Some(0));
        assert_eq!(
            icon_state(
                &e.host_event(&json!({"event":"keyDown","context":"a"})),
                "a"
            ),
            None
        );
        assert_eq!(
            icon_state(
                &e.rpc_event(&json!({"evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"chosen"}})),
                "a"
            ),
            Some(1)
        );
        assert_eq!(
            icon_state(
                &e.rpc_event(&json!({"evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":"other"}})),
                "a"
            ),
            Some(0)
        );
        assert_eq!(
            icon_state(
                &e.rpc_event(&json!({"cmd":"GET_CHANNEL","data":{"id":"chosen"}})),
                "a"
            ),
            Some(0)
        );
        assert_eq!(e.selected["channel_id"], "other");
        assert_eq!(
            icon_state(
                &e.rpc_event(&json!({"cmd":"GET_SELECTED_VOICE_CHANNEL","data":{"id":"chosen"}})),
                "a"
            ),
            Some(1)
        );
        assert_eq!(
            icon_state(
                &e.rpc_event(&json!({"evt":"VOICE_CHANNEL_SELECT","data":{"channel_id":null}})),
                "a"
            ),
            Some(0)
        );
    }
    #[test]
    fn actions_without_readback_never_claim_active_on_request_or_reply() {
        for (action, settings) in [
            ("videotoggle", json!({})),
            ("streamtoggle", json!({})),
            ("channel.text", json!({"channel":"text"})),
        ] {
            let mut e = ready(action, settings);
            for _ in 0..2 {
                let effects = e.host_event(&json!({"event":"keyDown","context":"a"}));
                assert!(effects.iter().any(|e| matches!(e, Effect::Rpc { .. })));
                assert_eq!(icon_state(&effects, "a"), None);
                assert_eq!(
                    icon_state(&e.rpc_event(&json!({"cmd":"TOGGLE_VIDEO","data":{}})), "a"),
                    Some(0)
                );
            }
        }
    }
    #[test]
    fn volume_encoding_and_caps() {
        assert_eq!(
            volume(&json!({"volAdjustValue":0}), Some(40.0), None, 200.0).unwrap(),
            41.0
        );
        assert_eq!(
            volume(&json!({"volAdjustValue":-25}), Some(20.0), None, 200.0).unwrap(),
            0.0
        );
        assert_eq!(volume(&json!({}), Some(98.0), None, 100.0).unwrap(), 100.0);
        assert!(volume(&json!({"volAdjustValue":25}), Some(50.0), None, 200.0).is_err());
    }
    #[test]
    fn mute_uses_observed_state_and_multi_override() {
        let mut e = ready("mute", json!({}));
        assert_eq!(args(&mut e, "keyDown", json!({})), json!({"mute":true}));
        assert_eq!(
            args(&mut e, "keyDown", json!({"userDesiredState":0})),
            json!({"mute":false})
        );
        assert!(
            e.host_event(&json!({"event":"keyUp","context":"a"}))
                .is_empty()
        );
    }
    #[test]
    fn unknown_state_never_toggles_assumed_false() {
        let mut e = ready("mute", json!({}));
        e.connection(true);
        assert!(
            e.host_event(&json!({"event":"keyDown","context":"a"}))
                .iter()
                .all(|e| !matches!(e, Effect::Rpc { .. }))
        );
    }
    #[test]
    fn preserves_unknown_settings_and_user_volume() {
        let mut e = ready(
            "uservolumecontrolbutton",
            json!({"user":"42","mode":"adjust","future":{"x":true}}),
        );
        e.rpc_event(&json!({"cmd":"GET_SELECTED_VOICE_CHANNEL","data":{"id":"v","voice_states":[{"user":{"id":"42"},"volume":195,"mute":false}]}}));
        assert_eq!(
            args(&mut e, "keyDown", json!({})),
            json!({"user_id":"42","volume":200.0})
        );
        assert_eq!(e.slots["a"].settings["future"], json!({"x":true}));
    }
    #[test]
    fn recent_speaker_cleared_on_channel_change() {
        let mut e = ready("uservolumecontroldial", json!({}));
        e.rpc_event(&json!({"evt":"SPEAKING_START","data":{"user_id":"42"}}));
        assert_eq!(e.speaker.as_deref(), Some("42"));
        e.rpc_event(&json!({"cmd":"GET_SELECTED_VOICE_CHANNEL","data":null}));
        assert!(e.speaker.is_none());
    }
    #[test]
    fn audio_device_only_changes_requested_fields() {
        let mut e = ready(
            "setaudiodevice",
            json!({"mode":"input","input":"usb","output":"leave-alone"}),
        );
        assert_eq!(
            args(&mut e, "keyDown", json!({})),
            json!({"input":{"device_id":"usb"}})
        );
    }
    #[test]
    fn errors_do_not_report_success_or_leak_remote_body() {
        let mut e = ready("mute", json!({}));
        let out=e.rpc_event(&json!({"cmd":"SET_VOICE_SETTINGS","evt":"ERROR","_context":"a","data":{"code":4006,"message":"private token"}}));
        assert!(format!("{out:?}").contains("showAlert"));
        assert!(!format!("{out:?}").contains("private token"));
    }
    fn commands(effects: Vec<Effect>) -> Vec<(String, Value)> {
        effects
            .into_iter()
            .filter_map(|e| {
                if let Effect::Rpc { cmd, args, .. } = e {
                    Some((cmd, args))
                } else {
                    None
                }
            })
            .collect()
    }
    #[test]
    fn native_ptt_voice_activity_press_release() {
        let mut e = ready("pushto.talk", json!({}));
        assert_eq!(
            commands(e.host_event(&json!({"event":"keyDown","context":"a"}))),
            vec![("SET_VOICE_SETTINGS".into(), json!({"mute":false}))]
        );
        assert_eq!(
            commands(e.host_event(&json!({"event":"keyUp","context":"a"}))),
            vec![("SET_VOICE_SETTINGS".into(), json!({"mute":true}))]
        );
        assert!(
            e.host_event(&json!({"event":"keyUp","context":"a"}))
                .is_empty()
        );
    }
    #[test]
    fn native_ptt_mode_has_active_and_balanced_cleanup() {
        let mut e = ready("pushto.talk", json!({}));
        e.rpc_event(&json!({"cmd":"GET_VOICE_SETTINGS","data":{"mode":{"type":"PUSH_TO_TALK"}}}));
        e.rpc_event(&json!({"cmd":"GET_SELECTED_VOICE_CHANNEL","data":{"id":"voice"}}));
        let down = commands(e.host_event(&json!({"event":"keyDown","context":"a"})));
        assert_eq!(
            down,
            vec![
                ("SET_VOICE_SETTINGS".into(), json!({"mute":false})),
                ("PUSH_TO_TALK".into(), json!({"active":true}))
            ]
        );
        let up = commands(e.host_event(&json!({"event":"willDisappear","context":"a"})));
        assert_eq!(
            up.last(),
            Some(&("PUSH_TO_TALK".into(), json!({"active":false})))
        );
        assert!(commands(e.connection(false)).is_empty());
    }
    #[test]
    fn shared_held_mute_releases_only_last_owner() {
        let mut e = ready("pushto.mute", json!({}));
        e.host_event(&json!({"event":"willAppear","context":"b","action":"com.elgato.discord.pushto.mute","payload":{"settings":{}}}));
        assert_eq!(
            commands(e.host_event(&json!({"event":"keyDown","context":"a"}))).len(),
            1
        );
        assert!(commands(e.host_event(&json!({"event":"keyDown","context":"b"}))).is_empty());
        assert!(commands(e.host_event(&json!({"event":"keyUp","context":"a"}))).is_empty());
        assert_eq!(
            commands(e.connection(false)),
            vec![("SET_VOICE_SETTINGS".into(), json!({"mute":false}))]
        );
    }
    #[test]
    fn recovered_extension_arguments() {
        for (action, settings, command, expected) in [
            (
                "soundboard",
                json!({"guild_id":"guild","sound_id":"sound"}),
                "PLAY_SOUNDBOARD_SOUND",
                json!({"guild_id":"guild","sound_id":"sound"}),
            ),
            (
                "videotoggle",
                json!({}),
                "TOGGLE_VIDEO",
                json!({"active":true}),
            ),
            ("streamtoggle", json!({}), "TOGGLE_SCREENSHARE", json!({})),
        ] {
            let mut e = ready(action, settings);
            assert_eq!(
                commands(e.host_event(&json!({"event":"keyDown","context":"a"}))),
                vec![(command.into(), expected)]
            );
        }
    }
    #[test]
    fn rapid_dial_turns_accumulate_until_matching_response() {
        let mut e = ready("volumecontrol", json!({"volValue":5}));
        assert_eq!(
            args(&mut e, "dialRotate", json!({"ticks":2})),
            json!({"output":{"volume":110.0}})
        );
        assert_eq!(
            args(&mut e, "dialRotate", json!({"ticks":2})),
            json!({"output":{"volume":120.0}})
        );
        e.rpc_event(&json!({"cmd":"SET_VOICE_SETTINGS","data":{"output":{"volume":110.0}}}));
        assert_eq!(
            args(&mut e, "dialRotate", json!({"ticks":1})),
            json!({"output":{"volume":125.0}})
        );
        e.rpc_event(&json!({"cmd":"SET_VOICE_SETTINGS","data":{"output":{"volume":125.0}}}));
        assert!(e.targets.is_empty());
    }
    fn click(e: &mut ActionEngine) -> Vec<Effect> {
        assert!(
            e.host_event(&json!({"event":"keyDown","context":"a"}))
                .is_empty()
        );
        e.host_event(&json!({"event":"keyUp","context":"a"}))
    }
    fn notices(mode: &str) -> ActionEngine {
        let mut e = ready(
            "notifications",
            json!({"select-press":mode,"select-longpress":"nothing","unknown":7}),
        );
        e.rpc_event(&json!({"cmd":"AUTHENTICATE","data":{"scopes":["rpc.notifications.read"]}}));
        for c in ["first", "second", "third"] {
            e.rpc_event(
                &json!({"evt":"NOTIFICATION_CREATE","data":{"channel_id":c,"body":"private body"}}),
            );
        }
        e
    }
    #[test]
    fn notification_cycles_open_clear_and_missing_scope() {
        let mut e = notices("cycle-old");
        assert_eq!(
            commands(click(&mut e)),
            vec![("SELECT_TEXT_CHANNEL".into(), json!({"channel_id":"first"}))]
        );
        assert_eq!(e.notifications.len(), 2);
        e.slots.get_mut("a").unwrap().settings["select-press"] = json!("cycle-recent");
        assert_eq!(commands(click(&mut e))[0].1, json!({"channel_id":"third"}));
        assert_eq!(e.notifications.len(), 1);
        e.slots.get_mut("a").unwrap().settings["select-press"] = json!("open");
        assert_eq!(commands(click(&mut e))[0].1, json!({"channel_id":"third"}));
        assert!(e.notifications.is_empty());
        assert!(e.latest_notification.is_none());
        assert_eq!(e.slots["a"].settings["unknown"], 7);
        e.scopes.clear();
        assert!(format!("{:?}", click(&mut e)).contains("rpc.notifications.read"));
    }
    #[test]
    fn notifications_are_bounded_and_not_message_history() {
        let mut e = notices("clear");
        for i in 0..1100 {
            e.notification(&json!({"channel_id":i.to_string(),"body":"not retained"}));
        }
        assert_eq!(e.notifications.len(), 1000);
        assert_eq!(e.notifications.front().unwrap().channel, "100");
        let out = click(&mut e);
        assert!(format!("{out:?}").contains("showOk"));
        assert!(!format!("{out:?}").contains("not retained"));
        assert!(e.notifications.is_empty());
    }
    fn channel_list(e: &mut ActionEngine) -> Vec<Effect> {
        e.rpc_event(&json!({"cmd":"GET_CHANNELS","_context":"a","_args":{"guild_id":"guild"},"data":{"channels":[{"id":"text","type":0},{"id":"voice","type":2},{"id":"thread","type":11},{"id":"private-thread","type":12},{"id":"forum","type":15}]}}))
    }
    #[test]
    fn stats_subscriptions_filter_and_deduplicate() {
        let mut e = ready("serverstats", json!({"server":"guild"}));
        let out = channel_list(&mut e);
        assert_eq!(
            out.iter()
                .filter(|e| matches!(
                    e,
                    Effect::Subscription {
                        subscribe: true,
                        ..
                    }
                ))
                .count(),
            3
        );
        assert!(
            channel_list(&mut e)
                .iter()
                .all(|e| !matches!(e, Effect::Subscription { .. }))
        );
        let out=e.host_event(&json!({"event":"didReceiveSettings","context":"a","action":"com.elgato.discord.serverstats","payload":{"settings":{"server":"other"}}}));
        assert_eq!(
            out.iter()
                .filter(|e| matches!(
                    e,
                    Effect::Subscription {
                        subscribe: false,
                        ..
                    }
                ))
                .count(),
            3
        );
        assert!(
            channel_list(&mut e)
                .iter()
                .all(|e| !matches!(e, Effect::Subscription { .. }))
        );
    }
    #[test]
    fn server_press_clears_only_messages_and_preserves_last_channel() {
        let mut e = ready(
            "serverstats",
            json!({"server":"guild","select-press":"open"}),
        );
        channel_list(&mut e);
        let now = Instant::now();
        for user in ["one", "one", "two"] {
            e.message(
                &json!({"channel_id":"text","message":{"author":{"id":user}}}),
                now,
            );
        }
        assert_eq!(e.stats["guild"].messages, 3);
        assert_eq!(e.stats["guild"].authors.len(), 2);
        assert_eq!(
            commands(click(&mut e)),
            vec![("SELECT_TEXT_CHANNEL".into(), json!({"channel_id":"text"}))]
        );
        assert_eq!(e.stats["guild"].messages, 0);
        assert_eq!(e.stats["guild"].authors.len(), 2);
        assert_eq!(e.stats["guild"].last_channel.as_deref(), Some("text"));
        e.tick_at(now + Duration::from_secs(1200));
        assert_eq!(e.stats["guild"].authors.len(), 2);
        e.tick_at(now + Duration::from_secs(1200) + Duration::from_nanos(1));
        assert!(e.stats["guild"].authors.is_empty());
    }
    #[test]
    fn widget_unknown_and_zero_are_distinct_and_poll_is_300_seconds() {
        let mut e = ready(
            "serverstats",
            json!({"server":"guild","select-users":"online"}),
        );
        e.widget_poll.clear();
        let now = Instant::now();
        assert_eq!(
            e.tick_at(now)
                .iter()
                .filter(|e| matches!(e, Effect::GuildWidget { .. }))
                .count(),
            1
        );
        assert!(e.tick_at(now + Duration::from_secs(299)).is_empty());
        assert_eq!(e.tick_at(now + Duration::from_secs(300)).len(), 1);
        assert!(format!("{:?}", e.render("a")).contains("Online unavailable"));
        e.rpc_event(
            &json!({"cmd":"DECKFORGE_GUILD_WIDGET","data":{"guild_id":"guild","presence_count":0}}),
        );
        assert!(format!("{:?}", e.render("a")).contains("0 online"));
        e.rpc_event(&json!({"cmd":"DECKFORGE_GUILD_WIDGET","data":{"guild_id":"guild","presence_count":null}}));
        assert!(e.stats["guild"].online.is_none());
    }
    #[test]
    fn stats_linux_press_policy_honors_nothing_and_clear() {
        let mut e = ready(
            "serverstats",
            json!({"server":"guild","select-press":"nothing"}),
        );
        channel_list(&mut e);
        e.message(
            &json!({"channel_id":"text","message":{"author":{"id":"one"}}}),
            Instant::now(),
        );
        assert!(click(&mut e).is_empty());
        assert_eq!(e.stats["guild"].messages, 1);
        e.slots.get_mut("a").unwrap().settings["select-press"] = json!("clear");
        let out = click(&mut e);
        assert!(commands(out.clone()).is_empty());
        assert!(format!("{out:?}").contains("showOk"));
        assert_eq!(e.stats["guild"].messages, 0);
    }
}
