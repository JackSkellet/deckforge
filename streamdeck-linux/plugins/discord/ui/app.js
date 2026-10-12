"use strict";
// Settings retain unknown imported fields; credentials never enter action settings.
let socket,
  uuid,
  action,
  settings = {},
  definitions = window.DeckforgeDiscordFields || {},
  state = {},
  auth = {};
const $ = (id) => document.getElementById(id);
const scopeNames = {
  rpc: "control local Discord",
  identify: "account identity",
  "rpc.voice.read": "read voice settings",
  "rpc.voice.write": "change voice settings",
  "rpc.notifications.read": "receive notifications",
  "messages.read": "read channel messages for activity counts",
  "rpc.video.read": "video status",
  "rpc.video.write": "video control",
  "rpc.screenshare.read": "screen sharing status",
  "rpc.screenshare.write": "screen sharing control",
};
function accessNames(scopes) {
  return [...new Set(scopes.map((scope) => scopeNames[scope] || scope))].join(
    ", ",
  );
}
const names = {
  server: "Server",
  serverName: "Saved server name",
  channel: "Channel",
  guild_id: "Sound server",
  sound_id: "Sound ID",
  sound_name: "Saved sound name",
  emoji_path: "Saved emoji path",
  input: "Input device",
  output: "Output device",
  inputname: "Saved input name",
  outputname: "Saved output name",
  user: "User",
  usernick: "User display name",
  user_mode: "Choose user",
  volValue: "Percent per dial tick",
  volume: "Percent per dial tick",
  volAdjustValue: "Volume change (%)",
  volSetValue: "Target volume (%)",
  muteType: "Mute operation",
  icon: "Icon style",
  device: "Audio device",
  mode: "Operation",
  "select-press": "On press",
  "select-longpress": "On long press",
  "select-messages": "Message count",
  "select-users": "User count",
};
const selectors = {
  server: { source: "servers", name: "serverName" },
  guild_id: { source: "servers" },
  channel: { source: "channels" },
  user: { source: "users", name: "usernick" },
  input: { source: "inputs", name: "inputname" },
  output: { source: "outputs", name: "outputname" },
};
const customValue = "__deckforge_custom_id__";
const unselectedValue = "__deckforge_unselected__";
function send(event, payload) {
  if (socket?.readyState === WebSocket.OPEN)
    socket.send(JSON.stringify({ event, context: uuid, action, payload }));
}
function plugin(payload) {
  send("sendToPlugin", payload);
}
window.connectElgatoStreamDeckSocket = (
  port,
  id,
  registration,
  info,
  actionInfo,
) => {
  uuid = id;
  const parsed = JSON.parse(actionInfo);
  action = parsed.action;
  settings = parsed.payload?.settings ?? {};
  renderFields();
  socket = new WebSocket(`ws://127.0.0.1:${port}`);
  socket.onopen = () => {
    socket.send(JSON.stringify({ event: registration, uuid }));
    plugin({ command: "status" });
  };
  socket.onclose = () => {
    $("status").textContent = "Deckforge connection closed.";
  };
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.event === "didReceiveSettings") {
      settings = message.payload?.settings ?? {};
      renderFields();
    }
    if (message.event === "sendToPropertyInspector") {
      Object.assign(state, message.payload ?? {});
      if (message.payload?.auth) Object.assign(auth, message.payload.auth);
      renderState();
      refreshLists();
    }
  };
};
function renderState() {
  $("status").textContent =
    auth.message ||
    (auth.authenticated
      ? "Discord authorized."
      : state.unauthorized
        ? "Connect an authorized Discord application."
        : "Waiting for Discord status.");
  $("error").textContent = state.error || state.warning || "";
  $("connect-saved").disabled = auth.configured === false;
  const missing = Array.isArray(state.missing_scopes)
    ? state.missing_scopes
    : [];
  const denied =
    typeof state.error === "string" &&
    state.error.startsWith("Discord rejected SUBSCRIBE");
  $("permissions").hidden = missing.length === 0 && !denied;
  $("missing-access").textContent = missing.length
    ? `This action needs permission to ${accessNames(missing)}.`
    : denied
      ? "Discord denied event access for this action. You can request access again; application approval may still be required."
      : "";
  $("requested-access").textContent =
    `Active actions request: ${accessNames(state.required_scopes ?? missing)}.`;
  $("authorize-required").disabled = auth.configured !== true;
  if (missing.length && auth.configured === false)
    $("missing-access").textContent +=
      " Enter your application credentials below first.";
}
function note(parent, text) {
  const p = document.createElement("p");
  p.className = "fieldnote";
  p.textContent = text;
  parent.append(p);
}
function option(select, value, text) {
  const node = document.createElement("option");
  node.value = value;
  node.textContent = text;
  select.append(node);
  return node;
}
function entries(key) {
  let items = state[selectors[key].source];
  if (items && !Array.isArray(items))
    items = Object.entries(items).map(([id, value]) =>
      typeof value === "string" ? { id, name: value } : { id, ...value },
    );
  let result = (Array.isArray(items) ? items : [])
    .filter((item) => item?.id !== undefined && String(item.id) !== "")
    .map((item) => ({
      ...item,
      id: String(item.id),
      name: String(item.name || item.id),
    }));
  if (key === "channel") {
    if (
      state.channels_server !== undefined &&
      state.channels_server !== settings.server
    )
      return [];
    const accepted = action.endsWith(".channel.voice") ? [2] : [0, 5];
    result = result.filter(
      (item) => item.type === undefined || accepted.includes(Number(item.type)),
    );
  }
  return result;
}
function persist(key, value) {
  const previous = settings[key];
  settings = { ...settings };
  if (value === undefined) delete settings[key];
  else settings[key] = value;
  const selector = selectors[key],
    selected = selector && entries(key).find((item) => item.id === value);
  if (selector?.name && (selected || value !== previous))
    settings[selector.name] = selected?.name || "";
  if (previous !== value && key === "server") {
    if (Object.hasOwn(definitions[action].fields, "channel"))
      delete settings.channel;
    state.channels = [];
    state.channels_server = value;
  }
  if (previous !== value && key === "guild_id") {
    settings.sound_id = "";
    settings.sound_name = "";
    settings.emoji_path = "";
  }
  send("setSettings", settings);
  $("save-status").textContent =
    socket?.readyState === WebSocket.OPEN
      ? "Settings saved."
      : "Not saved: Deckforge connection is closed.";
  visibility();
  refreshLists();
  for (const name of [
    "serverName",
    "usernick",
    "inputname",
    "outputname",
    "sound_id",
    "sound_name",
    "emoji_path",
  ])
    if ($(`setting-${name}`)) $(`setting-${name}`).value = settings[name] ?? "";
}
function addSelector(row, key) {
  const select = document.createElement("select");
  select.id = `setting-${key}`;
  select.dataset.key = key;
  row.append(select);
  const custom = document.createElement("div");
  custom.id = `custom-${key}`;
  custom.className = "custom-id";
  const label = document.createElement("label");
  label.htmlFor = `setting-${key}-custom`;
  label.textContent = `Custom ${names[key].toLowerCase()} ID`;
  custom.append(label);
  const input = document.createElement("input");
  input.id = `setting-${key}-custom`;
  input.type = "text";
  input.autocomplete = "off";
  input.spellcheck = false;
  custom.append(input);
  row.append(custom);
  select.onchange = () => {
    if (select.value === customValue) {
      custom.hidden = false;
      input.value = settings[key] ?? "";
      input.focus();
    } else
      persist(key, select.value === unselectedValue ? undefined : select.value);
  };
  input.onchange = () => {
    const value = input.value.trim();
    persist(
      key,
      key === "channel" && action.endsWith(".channel.voice") && !value
        ? undefined
        : value,
    );
  };
  const status = document.createElement("p");
  status.id = `list-status-${key}`;
  status.className = "fieldnote";
  row.append(status);
}
function enumLabel(key, value) {
  if (key === "user_mode")
    return value === "any"
      ? "Most recent speaker"
      : value === "specific"
        ? "Specific user"
        : value;
  if (key === "mode" && action.endsWith(".setaudiodevice"))
    return (
      { input: "Input only", output: "Output only", both: "Input and output" }[
        value
      ] || value
    );
  return (
    {
      dynamic_h: "Horizontal meter",
      dynamic_v: "Vertical meter",
      static: "Static icon",
      "cycle-recent": "Cycle newest notification",
      "cycle-old": "Cycle oldest notification",
      nothing: "Do nothing",
      display: "Display count",
      online: "Online users",
      active: "Recent message authors",
    }[value] || value.replaceAll("_", " ").replaceAll("-", " ")
  );
}
function numericBounds(key, schema) {
  return {
    min: key === "volAdjustValue" ? -25 : schema.minimum,
    max:
      key === "volAdjustValue"
        ? 25
        : key === "volSetValue" &&
            action.endsWith(".volumecontrolbutton") &&
            (settings.device ?? "output") === "input"
          ? 100
          : schema.maximum,
  };
}
function renderFields() {
  const fields = $("fields");
  fields.replaceChildren();
  const definition = definitions[action];
  if (!definition) {
    const p = document.createElement("p");
    p.className = "error";
    p.setAttribute("role", "alert");
    p.textContent =
      "Action setting definitions are unavailable. Reinstall or update the native Discord plugin.";
    fields.append(p);
    return;
  }
  $("title").textContent = definition.name;
  if (action.endsWith(".notifications") || action.endsWith(".serverstats"))
    note(
      fields,
      action.endsWith(".notifications")
        ? "Counts notifications received while connected. Clear resets this plugin counter; it does not mark Discord messages as read."
        : "Counts messages received while connected. Active counts distinct authors in the last 20 minutes. Online requires an enabled public server widget.",
    );
  if (action.endsWith(".soundboard"))
    note(
      fields,
      "Discord does not provide a sound catalog here. Enter a sound ID. Saved sound name and emoji path are compatibility metadata; playback uses the server and sound IDs.",
    );
  for (const [key, schema] of Object.entries(definition.fields)) {
    const row = document.createElement("div");
    row.dataset.field = key;
    const label = document.createElement("label");
    label.htmlFor = `setting-${key}`;
    label.textContent = names[key] || key;
    row.append(label);
    if (selectors[key]) {
      addSelector(row, key);
      fields.append(row);
      continue;
    }
    const input = document.createElement(schema.enum ? "select" : "input");
    input.id = `setting-${key}`;
    input.dataset.key = key;
    let value = settings[key] ?? schema.default ?? "";
    if (schema.enum) {
      for (const item of schema.enum) option(input, item, enumLabel(key, item));
      if (!schema.enum.includes(value) && value !== "")
        option(input, value, `Imported value: ${value}`);
    } else if (schema.type === "integer") {
      input.type = "number";
      const bounds = numericBounds(key, schema);
      input.min = bounds.min;
      input.max = bounds.max;
      input.step = "1";
      if (key === "volAdjustValue" && Number.isInteger(value) && value >= 0)
        value += 1;
    } else {
      input.type = "text";
      input.autocomplete = "off";
      input.spellcheck = false;
    }
    input.value = value;
    input.addEventListener("change", () => {
      let value = input.value;
      if (schema.type === "integer") {
        const bounds = numericBounds(key, schema);
        value = Number(value);
        if (
          input.value === "" ||
          !Number.isInteger(value) ||
          value < bounds.min ||
          value > bounds.max ||
          (key === "volAdjustValue" && value === 0)
        ) {
          $("save-status").textContent =
            key === "volAdjustValue"
              ? "Choose −25…−1% or +1…+25%."
              : `Enter a whole number from ${bounds.min} to ${bounds.max}.`;
          return;
        }
        if (key === "volAdjustValue" && value > 0) value -= 1;
      }
      persist(key, value);
    });
    row.append(input);
    if (key === "select-longpress") {
      input.disabled = true;
      note(
        row,
        "Long-press actions are not available yet. The saved value is retained.",
      );
    }
    if (key === "icon") {
      if (
        action.endsWith(".volumecontrolbutton") ||
        action.endsWith(".uservolumecontrolbutton")
      )
        note(
          row,
          "Dynamic styles show a live volume meter in Adjust and Set modes.",
        );
      else {
        for (const choice of input.options)
          if (choice.value !== "static") {
            choice.disabled = true;
            choice.textContent += " (not available yet)";
          }
        note(
          row,
          "Dynamic server avatars are not available yet. An imported dynamic choice is retained until you select Static.",
        );
      }
    }
    if (key === "volAdjustValue")
      note(
        row,
        "Negative values lower volume; positive values raise it. Zero is not a volume adjustment.",
      );
    if (key === "volSetValue" && action.endsWith(".volumecontrolbutton"))
      note(
        row,
        "Input volume supports 0–100%; output volume supports 0–200%. Imported values are retained until edited.",
      );
    fields.append(row);
  }
  if (!Object.keys(definition.fields).length) {
    const p = document.createElement("p");
    p.className = "empty";
    p.textContent = "This action has no additional settings.";
    fields.append(p);
  }
  visibility();
  refreshLists();
  renderState();
}
function visibility() {
  const mode = settings.mode ?? definitions[action]?.fields.mode?.default;
  for (const row of $("fields").children) {
    const key = row.dataset.field;
    let show = true;
    if (["input", "inputname"].includes(key) && mode === "output") show = false;
    if (["output", "outputname"].includes(key) && mode === "input")
      show = false;
    if (
      ["user", "usernick"].includes(key) &&
      action?.endsWith("uservolumecontroldial") &&
      (settings.user_mode ?? "any") === "any"
    )
      show = false;
    if (key === "volAdjustValue" && mode !== "adjust") show = false;
    if (key === "volSetValue" && mode !== "set") show = false;
    if (key === "muteType" && mode !== "mute") show = false;
    if (
      key === "icon" &&
      action?.endsWith(".uservolumecontrolbutton") &&
      mode === "mute"
    )
      show = false;
    row.hidden = !show;
  }
  const volume = $("setting-volSetValue");
  if (volume)
    volume.max = numericBounds(
      "volSetValue",
      definitions[action].fields.volSetValue,
    ).max;
}
function refreshLists() {
  for (const key of Object.keys(selectors)) {
    const select = $(`setting-${key}`);
    if (!select) continue;
    const custom = $(`custom-${key}`),
      input = $(`setting-${key}-custom`);
    const editing = select.value === customValue && !custom.hidden;
    const voiceChannel = key === "channel" && action.endsWith(".channel.voice");
    const current =
      voiceChannel && typeof settings[key] !== "string"
        ? unselectedValue
        : String(settings[key] ?? "");
    const items = entries(key);
    select.replaceChildren();
    option(
      select,
      voiceChannel ? unselectedValue : "",
      `Choose ${names[key].toLowerCase()}…`,
    );
    if (voiceChannel) option(select, "", "Disconnect from voice");
    for (const item of items)
      option(select, item.id, `${item.name} · ${item.id}`);
    if (
      current &&
      current !== unselectedValue &&
      !items.some((item) => item.id === current)
    )
      option(select, current, `Saved / custom ID: ${current}`);
    option(select, customValue, "Enter custom ID…");
    select.value = editing ? customValue : current;
    custom.hidden =
      !editing &&
      (!current ||
        current === unselectedValue ||
        items.some((item) => item.id === current));
    if (document.activeElement !== input)
      input.value = current === unselectedValue ? "" : current;
    $(`list-status-${key}`).textContent = items.length
      ? `${items.length} available ${key === "user" ? "users in the current voice channel" : "choices"}.`
      : "No list is available yet. Connect Discord or enter a custom ID.";
  }
}
$("connect-saved").onclick = () => plugin({ command: "connect" });
$("connect-new").onclick = () => {
  const client_id = $("client-id").value.trim(),
    client_secret = $("client-secret").value;
  if (!client_id || !client_secret) {
    $("error").textContent = "Enter the client ID and client secret.";
    return;
  }
  const payload = {
    command: "connect",
    permissions_for_actions: true,
    client_id,
    client_secret,
    scopes: $("scopes")
      .value.split(/[\s,]+/)
      .filter(Boolean),
  };
  if ($("redirect").value.trim())
    payload.redirect_uri = $("redirect").value.trim();
  plugin(payload);
  $("client-secret").value = "";
};
$("authorize-required").onclick = () =>
  plugin({
    command: "connect",
    permissions_for_actions: true,
    reauthorize: true,
  });
$("disconnect").onclick = () => plugin({ command: "disconnect" });
$("forget").onclick = () => {
  if (
    window.confirm(
      "Forget the application credentials and authorization saved privately by this plugin?",
    )
  )
    plugin({ command: "forget" });
};
