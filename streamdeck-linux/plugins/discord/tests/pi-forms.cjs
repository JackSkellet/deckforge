// UI contract tests against the real inspector scripts and a DOM/socket fixture.
const fs = require("node:fs"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const { JSDOM } = require("../../../frontend/node_modules/jsdom");
const ui = path.join(__dirname, "../ui"),
  schema = JSON.parse(fs.readFileSync(path.join(ui, "fields.json"), "utf8"));
require("node:child_process").execFileSync(process.execPath, [
  path.join(__dirname, "generate-fields.cjs"),
  "--check",
]);
const lists = {
  auth: { configured: true, authenticated: true, message: "Connected" },
  servers: [
    { id: "g1", name: "Guild One" },
    { id: "g2", name: "Guild Two" },
  ],
  channels_server: "g1",
  channels: [
    { id: "text", name: "Chat", type: 0 },
    { id: "voice", name: "Lounge", type: 2 },
    { id: "category", name: "Category", type: 4 },
  ],
  users: [{ id: "u1", name: "Ada" }],
  inputs: [{ id: "i1", name: "Microphone" }],
  outputs: [{ id: "o1", name: "Speakers" }],
};
function inspector(suffix, settings = {}) {
  const dom = new JSDOM(fs.readFileSync(path.join(ui, "index.html"), "utf8"), {
      url: "file:///fixture/index.html",
      runScripts: "outside-only",
    }),
    w = dom.window,
    sent = [];
  class Socket {
    static OPEN = 1;
    readyState = 1;
    constructor() {
      Socket.current = this;
    }
    send(value) {
      sent.push(JSON.parse(value));
    }
  }
  w.WebSocket = Socket;
  w.fetch = () => {
    throw Error("Local schema must not use fetch");
  };
  w.eval(fs.readFileSync(path.join(ui, "fields.js"), "utf8"));
  w.eval(fs.readFileSync(path.join(ui, "app.js"), "utf8"));
  const action = "com.elgato.discord." + suffix;
  w.connectElgatoStreamDeckSocket(
    1234,
    "inspector",
    "registerPropertyInspector",
    "{}",
    JSON.stringify({
      action,
      payload: { settings: { future: { keep: true }, ...settings } },
    }),
  );
  Socket.current.onopen();
  const receive = (payload) =>
    Socket.current.onmessage({
      data: JSON.stringify({ event: "sendToPropertyInspector", payload }),
    });
  receive(lists);
  const get = (key) => w.document.getElementById("setting-" + key),
    row = (key) => w.document.querySelector(`[data-field="${key}"]`);
  const change = (key, value) => {
    const input = get(key);
    input.value = String(value);
    input.dispatchEvent(new w.Event("change", { bubbles: true }));
  };
  const saved = () =>
    sent.filter((event) => event.event === "setSettings").at(-1)?.payload;
  return { w, dom, sent, receive, get, row, change, saved };
}
const coverage = [];
for (const [action, definition] of Object.entries(schema)) {
  const pi = inspector(action.slice("com.elgato.discord.".length));
  assert.deepEqual(
    [...pi.w.document.querySelectorAll("#fields [data-field]")].map(
      (row) => row.dataset.field,
    ),
    Object.keys(definition.fields),
  );
  if (!Object.keys(definition.fields).length)
    assert.match(
      pi.w.document.querySelector("#fields").textContent,
      /no additional settings/,
    );
  assert.equal(
    pi.sent.filter((message) => message.event === "setSettings").length,
    0,
    "Rendering must not alter settings",
  );
  coverage.push({
    action,
    fields: Object.keys(definition.fields),
    no_options: !Object.keys(definition.fields).length,
  });
  pi.dom.window.close();
}
{
  const pi = inspector("serverstats", {
    server: "imported",
    serverName: "Custom label",
    strange: "retain",
  });
  assert.equal(pi.get("server").tagName, "SELECT");
  assert.equal(pi.get("server").value, "imported");
  assert.match(
    pi.get("server").selectedOptions[0].textContent,
    /Saved \/ custom ID/,
  );
  pi.change("server", "g2");
  assert.equal(pi.saved().serverName, "Guild Two");
  assert.equal(pi.saved().strange, "retain");
  assert.deepEqual(pi.saved().future, { keep: true });
  assert.equal(pi.get("select-longpress").disabled, true);
  pi.change("server", "__deckforge_custom_id__");
  assert.equal(
    pi.saved().server,
    "g2",
    "Opening custom entry must not overwrite saved choice",
  );
  pi.change("server-custom", "manual-server");
  assert.equal(pi.saved().server, "manual-server");
  assert.equal(pi.saved().serverName, "");
  pi.dom.window.close();
}
{
  const pi = inspector("channel.voice", {
    server: "g1",
    channel: "voice",
    icon: "dynamic",
  });
  assert.equal(
    [...pi.get("channel").options].find((option) => option.value === "")
      .textContent,
    "Disconnect from voice",
  );
  assert.deepEqual(
    [...pi.get("channel").options]
      .filter((o) => ["voice", "text", "category"].includes(o.value))
      .map((o) => o.value),
    ["voice"],
  );
  assert.equal(pi.get("icon").value, "dynamic");
  assert.equal(pi.get("icon").selectedOptions[0].disabled, true);
  pi.change("server", "g2");
  assert.equal(Object.hasOwn(pi.saved(), "channel"), false);
  assert.equal(pi.get("channel").value, "__deckforge_unselected__");
  pi.receive({
    channels_server: "g1",
    channels: [{ id: "stale", name: "Old guild voice", type: 2 }],
  });
  assert(![...pi.get("channel").options].some((o) => o.value === "stale"));
  pi.receive({
    channels_server: "g2",
    channels: [{ id: "new", name: "New guild voice", type: 2 }],
  });
  pi.change("channel", "new");
  assert.equal(pi.saved().channel, "new");
  pi.change("channel", "");
  assert.equal(pi.saved().channel, "");
  pi.dom.window.close();
}
{
  const pi = inspector("channel.text", { server: "g1", channel: "text" });
  assert(![...pi.get("channel").options].some((o) => o.value === "voice"));
  pi.dom.window.close();
}
{
  const pi = inspector("setaudiodevice", {
    input: "old-mic",
    output: "old-speaker",
  });
  pi.change("input", "i1");
  pi.change("output", "o1");
  assert.equal(pi.saved().inputname, "Microphone");
  assert.equal(pi.saved().outputname, "Speakers");
  pi.change("mode", "input");
  assert.equal(pi.row("output").hidden, true);
  assert.equal(pi.row("input").hidden, false);
  assert.equal(pi.saved().output, "o1");
  pi.change("mode", "output");
  assert.equal(pi.row("input").hidden, true);
  pi.dom.window.close();
}
{
  const pi = inspector("uservolumecontrolbutton", {
    user: "u1",
    volSetValue: 180,
  });
  assert.equal(pi.row("muteType").hidden, false);
  assert.equal(pi.row("icon").hidden, true);
  pi.change("user", "u1");
  assert.equal(pi.saved().usernick, "Ada");
  pi.change("mode", "adjust");
  assert.equal(pi.row("muteType").hidden, true);
  assert.equal(pi.row("volAdjustValue").hidden, false);
  assert.equal(pi.get("volAdjustValue").value, "5");
  pi.change("volAdjustValue", 25);
  assert.equal(pi.saved().volAdjustValue, 24);
  pi.change("volAdjustValue", -25);
  assert.equal(pi.saved().volAdjustValue, -25);
  const count = pi.sent.length;
  pi.change("volAdjustValue", 0);
  assert.equal(pi.sent.length, count);
  pi.change("mode", "set");
  assert.equal(pi.row("volSetValue").hidden, false);
  assert.equal(pi.row("volAdjustValue").hidden, true);
  assert.equal(pi.get("volSetValue").max, "200");
  pi.dom.window.close();
}
{
  const pi = inspector("volumecontrolbutton", {
    device: "output",
    mode: "set",
    volSetValue: 175,
  });
  pi.change("device", "input");
  assert.equal(pi.get("volSetValue").max, "100");
  assert.equal(
    pi.saved().volSetValue,
    175,
    "Changing mode retains imported/out-of-range saved values",
  );
  const count = pi.sent.length;
  pi.change("volSetValue", 101);
  assert.equal(pi.sent.length, count);
  pi.change("volSetValue", 100);
  assert.equal(pi.saved().volSetValue, 100);
  pi.dom.window.close();
}
{
  const pi = inspector("uservolumecontroldial", {
    user: "saved-user",
    usernick: "Saved user",
  });
  assert.equal(pi.row("user").hidden, true);
  pi.change("user_mode", "specific");
  assert.equal(pi.row("user").hidden, false);
  assert.equal(pi.get("user").value, "saved-user");
  pi.change("user_mode", "any");
  assert.equal(pi.saved().user, "saved-user");
  pi.dom.window.close();
}
{
  const pi = inspector("soundboard", {
    guild_id: "g1",
    sound_id: "old-sound",
    sound_name: "Old name",
    emoji_path: "old.png",
  });
  assert.match(
    pi.w.document.querySelector("#fields").textContent,
    /does not provide a sound catalog/,
  );
  pi.change("guild_id", "g2");
  assert.equal(pi.saved().sound_id, "");
  assert.equal(pi.saved().sound_name, "");
  pi.dom.window.close();
}
console.log(
  JSON.stringify({
    ok: true,
    actions: coverage.length,
    coverage,
    checks: [
      "file schema without fetch",
      "custom IDs and unknown settings retained",
      "named lists save companion names",
      "dependent channel provenance",
      "voice disconnect",
      "all conditional modes",
      "real volume percentages and limits",
      "unsupported fields explicit",
    ],
  }),
);
