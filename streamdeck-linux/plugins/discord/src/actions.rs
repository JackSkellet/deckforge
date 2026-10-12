//! Original action implementation. UUID/settings compatibility comes from the
//! inspected manifest; RPC shapes from Discord's official RPC documentation.
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Effect {
    Rpc {
        cmd: String,
        args: Value,
        context: Option<String>,
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
#[derive(Default)]
pub struct ActionEngine {
    slots: BTreeMap<String, Slot>,
    connected: bool,
    voice: Value,
    selected: Value,
    guilds: Value,
    channels: BTreeMap<String, Value>,
    users: BTreeMap<String, Value>,
    speaker: Option<String>,
    errors: BTreeMap<String, String>,
    held: BTreeMap<String, Held>,
    targets: BTreeMap<String, Value>,
}
impl ActionEngine {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn connection(&mut self, connected: bool) -> Vec<Effect> {
        let mut cleanup = vec![];
        for context in self.held.keys().cloned().collect::<Vec<_>>() {
            cleanup.extend(self.release(&context));
        }
        self.connected = connected;
        self.voice = Value::Null;
        self.selected = Value::Null;
        self.users.clear();
        self.speaker = None;
        self.errors.clear();
        self.targets.clear();
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
            let cleanup = self.release(context);
            self.slots.remove(context);
            self.errors.remove(context);
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
            return vec![];
        }
        let mut out = vec![rpc(
            "SET_VOICE_SETTINGS",
            json!({"mute":mode==Held::Mute}),
            Some(context),
        )];
        if mode == Held::TalkPtt {
            out.push(rpc("PUSH_TO_TALK", json!({"active":true}), Some(context)));
        }
        out
    }
    fn release(&mut self, context: &str) -> Vec<Effect> {
        let Some(mode) = self.held.remove(context) else {
            return vec![];
        };
        if self.held.values().any(|m| *m == mode) {
            return vec![];
        }
        let mut out = vec![rpc(
            "SET_VOICE_SETTINGS",
            json!({"mute":mode==Held::TalkActivity}),
            Some(context),
        )];
        if mode == Held::TalkPtt {
            out.push(rpc("PUSH_TO_TALK", json!({"active":false}), Some(context)));
        }
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
                let key = if suffix == "mute" {"mute"} else {"deaf"};
                let old = self.target(key, &self.voice[key]).as_bool().ok_or("Discord has not supplied the current voice settings yet.")?;
                args[key] = json!(desired_bool(payload, old));
            }
            "pushtotalktoggle" => {
                let mode = self.target("mode.type", &self.voice["mode"]["type"]).as_str().ok_or("Discord has not supplied the current input mode yet.")?;
                args["mode"] = json!({"type": if mode == "PUSH_TO_TALK" {"VOICE_ACTIVITY"} else {"PUSH_TO_TALK"}});
            }
            "channel.voice" | "channel.text" => {
                let channel = s["channel"].as_str().ok_or("Choose a channel in this action's settings.")?;
                if suffix == "channel.text" && channel.is_empty() { return Err("Choose a text channel.".into()); }
                cmd = if suffix == "channel.voice" {"SELECT_VOICE_CHANNEL"} else {"SELECT_TEXT_CHANNEL"};
                args["channel_id"] = if channel.is_empty() { Value::Null } else { json!(channel) };
            }
            "setaudiodevice" => {
                let mode = string(s, "mode", "both");
                if !["input", "output", "both"].contains(&mode) { return Err("Unknown audio device mode.".into()); }
                for device in ["input", "output"] {
                    if mode == "both" || mode == device { args[device] = json!({"device_id": required(s, device)?}); }
                }
            }
            "volumecontrol" | "volumecontrolbutton" => {
                let device = string(s, "device", "output");
                if !["input", "output"].contains(&device) { return Err("Unknown volume device.".into()); }
                if suffix == "volumecontrol" && event != "dialRotate" {
                    let key = if device == "input" {"mute"} else {"deaf"};
                    args[key] = json!(!self.target(key, &self.voice[key]).as_bool().ok_or("Current mute state is unavailable.")?);
                } else {
                    let limit = if device == "input" {100.0} else {200.0};
                    let value = volume(s, self.target(&format!("{device}.volume"), &self.voice[device]["volume"]).as_f64(), if suffix == "volumecontrol" {Some((payload, "volValue"))} else {None}, limit)?;
                    args[device] = json!({"volume":value});
                }
            }
            "uservolumecontrolbutton" | "uservolumecontroldial" => {
                let user = if suffix == "uservolumecontroldial" && string(s,"user_mode","any") == "any" {
                    self.speaker.as_deref().ok_or("No recent speaker is available in the current voice channel.")?
                } else { required(s, "user")? };
                let current = self.users.get(user).ok_or("The selected user's voice state is not available in the current channel.")?;
                cmd = "SET_USER_VOICE_SETTINGS";
                args["user_id"] = json!(user);
                let dial = suffix == "uservolumecontroldial";
                if (dial && event != "dialRotate") || (!dial && string(s,"mode","mute") == "mute") {
                    let mute = match string(s,"muteType","toggle") {
                        "mute" => true, "unmute" => false,
                        "toggle" => desired_bool(payload, self.target(&format!("user:{user}.mute"), &current["mute"]).as_bool().ok_or("Current user mute state is unavailable.")?),
                        _ => return Err("Unknown mute operation.".into()),
                    };
                    args["mute"] = json!(mute);
                } else {
                    args["volume"] = json!(volume(s, self.target(&format!("user:{user}.volume"), &current["volume"]).as_f64(), if dial {Some((payload,"volume"))} else {None}, 200.0)?);
                }
            }
            "serverstats" | "notifications" => return Err("Notification history and server statistics are not available in this native adapter yet.".into()),
            "pushto.talk" | "pushto.mute" => return Err("Held voice control is awaiting the verified Discord RPC contract.".into()),
            "soundboard" => { cmd="PLAY_SOUNDBOARD_SOUND"; args=json!({"guild_id":required(s,"guild_id")?,"sound_id":required(s,"sound_id")?}); }
            "videotoggle" => {cmd="TOGGLE_VIDEO";args=json!({"active":true});}
            "streamtoggle" => {cmd="TOGGLE_SCREENSHARE";}
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
            self.targets.clear();
            if let Some(context) = event["_context"].as_str() {
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
        match (cmd, evt) {
            ("GET_VOICE_SETTINGS" | "SET_VOICE_SETTINGS", _) | (_, "VOICE_SETTINGS_UPDATE") => {
                merge(&mut self.voice, data)
            }
            ("GET_GUILDS", _) => self.guilds = data["guilds"].clone(),
            ("GET_CHANNELS", _) => {
                if let Some(context) = event["_context"].as_str() {
                    self.channels
                        .insert(context.into(), data["channels"].clone());
                }
            }
            ("GET_SELECTED_VOICE_CHANNEL" | "SELECT_VOICE_CHANNEL", _) => self.set_channel(data),
            (_, "VOICE_CHANNEL_SELECT") => {
                self.selected = data.clone();
                self.users.clear();
                self.speaker = None;
                if let Some(id) = data["channel_id"].as_str() {
                    return vec![rpc("GET_CHANNEL", json!({"channel_id":id}), None)];
                }
            }
            ("GET_CHANNEL", _) => self.set_channel(data),
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
        {
            self.errors.remove(context);
        }
        self.refresh()
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
            "pushtotalktoggle" => self.voice["mode"]["type"]
                .as_str()
                .map(|m| m == "PUSH_TO_TALK"),
            _ => None,
        };
        let users:Vec<Value>=self.users.iter().map(|(id,u)|json!({"id":id,"name":u["nick"].as_str().or(u["user"]["username"].as_str()).unwrap_or(id)})).collect();
        let payload = json!({"disabled":!self.connected,"unauthorized":!self.connected,"error":self.errors.get(context),"servers":self.guilds,"channels":self.channels.get(context),"users":users,"inputs":self.voice["input"]["available_devices"],"outputs":self.voice["output"]["available_devices"],"_currentVoice":self.selected,"voice":self.voice});
        let mut out = vec![Effect::Host(
            json!({"event":"sendToPropertyInspector","context":context,"action":slot.action,"payload":payload}),
        )];
        if let Some(state) = state {
            out.push(host("setState", context, json!({"state":u8::from(state)})));
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
    fn ready(action: &str, settings: Value) -> ActionEngine {
        let mut e = ActionEngine::new();
        e.connection(true);
        e.host_event(&json!({"event":"willAppear","context":"a","action":format!("com.elgato.discord.{action}"),"payload":{"settings":settings}}));
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
    fn unsupported_actions_fail_explicitly() {
        let mut e = ready("notifications", json!({"select-longpress":"cycle-old"}));
        let out = e.host_event(&json!({"event":"keyDown","context":"a"}));
        assert!(commands(out.clone()).is_empty());
        assert!(format!("{out:?}").contains("not available"));
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
}
