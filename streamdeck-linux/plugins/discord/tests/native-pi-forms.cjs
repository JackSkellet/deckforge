// Runs owned copied HTML in the actual Rust WebKit helper, with a loopback mock host.
// No Discord socket/auth, user state, USB, external browser automation or defaults.
const fs = require("node:fs"),
  path = require("node:path"),
  os = require("node:os"),
  http = require("node:http"),
  crypto = require("node:crypto"),
  { spawn } = require("node:child_process");
const { WebSocketServer } = require("../../../frontend/node_modules/ws");
const args = process.argv.slice(2),
  opt = (key, fallback) => {
    const i = args.indexOf(key);
    return i < 0 ? fallback : args[i + 1];
  };
const binary = path.resolve(opt("--binary", "target/debug/deckforge"));
const source = path.resolve(opt("--ui", path.join(__dirname, "../ui")));
const output = path.resolve(
  opt("--output", "../evidence/discord-pi-audit/native"),
);
const negative = args.includes("--expect-blank");
const schema = JSON.parse(
  fs.readFileSync(path.join(source, "fields.json"), "utf8"),
);
const payload = {
  auth: {
    configured: true,
    authenticated: true,
    connected: true,
    message: "Fixture connected",
  },
  servers: [
    { id: "g1", name: "Alpha guild" },
    { id: "g2", name: "Beta guild" },
  ],
  channels: [
    { id: "c1", name: "General", type: 0 },
    { id: "c2", name: "Voice lounge", type: 2 },
  ],
  users: [
    { id: "u1", name: "Ada" },
    { id: "u2", name: "Lin" },
  ],
  inputs: [{ id: "mic1", name: "Fixture microphone" }],
  outputs: [{ id: "sp1", name: "Fixture speakers" }],
};
(async () => {
  fs.mkdirSync(output, { recursive: true });
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "deckforge-pi-audit-"));
  // Pin the tested executable while other agents may rebuild the workspace target.
  const executable = path.join(temp, "deckforge");
  fs.copyFileSync(binary, executable);
  fs.chmodSync(executable, 0o755);
  const pending = new Map(),
    reports = [],
    settingsEvents = [];
  let child;
  const server = http.createServer((req, res) => {
    let text = "";
    req.on("data", (chunk) => (text += chunk));
    req.on("end", () => {
      try {
        const proof = JSON.parse(text);
        pending.get(proof.action)?.(proof);
      } catch {}
      res.writeHead(204, { "Access-Control-Allow-Origin": "*" });
      res.end();
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const wss = new WebSocketServer({ server });
  wss.on("connection", (ws) => {
    ws.on("message", (text) => {
      const message = JSON.parse(text);
      if (message.event === "registerPropertyInspector") {
        for (const ms of [10, 200, 600])
          setTimeout(() => {
            if (ws.readyState === 1)
              ws.send(
                JSON.stringify({ event: "sendToPropertyInspector", payload }),
              );
          }, ms);
      }
      if (message.event === "setSettings") {
        settingsEvents.push(message);
        ws.send(
          JSON.stringify({
            event: "didReceiveSettings",
            payload: { settings: message.payload },
          }),
        );
        if (message.payload.server)
          setTimeout(() => {
            if (ws.readyState === 1)
              ws.send(
                JSON.stringify({
                  event: "sendToPropertyInspector",
                  payload: {
                    channels_server: message.payload.server,
                    channels:
                      message.payload.server === "g1"
                        ? payload.channels
                        : [{ id: "c3", name: "Beta text", type: 0 }],
                  },
                }),
              );
          }, 20);
      }
    });
  });
  try {
    for (const action of negative
      ? ["com.elgato.discord.serverstats"]
      : Object.keys(schema)) {
      const dir = path.join(temp, action);
      fs.cpSync(source, dir, { recursive: true });
      const endpoint = `http://127.0.0.1:${server.address().port}/proof`;
      const interactions = negative
        ? ""
        : `
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const change=async(key,value)=>{const input=document.getElementById('setting-'+key);if(!input||input.disabled)throw Error('Editable control missing: '+key);input.value=String(value);input.dispatchEvent(new Event('change',{bubbles:true}));await pause(70)};
if(document.getElementById('setting-server')){await change('server','g1');if(settings.serverName!=='Alpha guild')throw Error('Server name companion missing');if(action.endsWith('.channel.voice')&&Object.hasOwn(settings,'channel'))throw Error('Server selection implicitly configured Disconnect')}
if(document.getElementById('setting-channel')){const id=action.endsWith('.channel.voice')?'c2':'c1';await change('channel',id);if(settings.channel!==id)throw Error('Named channel did not save');if(action.endsWith('.channel.voice'))await change('channel','')}
if(document.getElementById('setting-guild_id')){await change('guild_id','g1');await change('sound_id','manual-sound');await change('sound_name','Fixture metadata')}
if(document.getElementById('setting-input')){await change('input','mic1');await change('output','sp1');if(settings.inputname!=='Fixture microphone'||settings.outputname!=='Fixture speakers')throw Error('Device companion names missing');await change('mode','input');if(!document.querySelector('[data-field=output]').hidden)throw Error('Audio mode did not hide output')}
if(document.getElementById('setting-user_mode'))await change('user_mode','specific');
if(document.getElementById('setting-user')){await change('user','u1');if(settings.usernick!=='Ada')throw Error('User companion name missing')}
if(document.getElementById('setting-volAdjustValue')){await change('mode','adjust');await change('volAdjustValue',25);if(settings.volAdjustValue!==24)throw Error('Positive percent encoding incorrect');await change('mode','set');await change('volSetValue',100)}
if(document.getElementById('setting-volValue'))await change('volValue',3);
if(document.getElementById('setting-volume'))await change('volume',3);
if(document.getElementById('setting-select-users'))await change('select-users','active');
if(document.getElementById('setting-select-press'))await change('select-press','clear');
if(!settings.future?.retained)throw Error('Unknown setting lost');
`;
      const fixture = `\nconst realFetch=window.fetch.bind(window);let fileFetchFailed=false;window.fetch=(url,...rest)=>realFetch(url,...rest).catch(error=>{if(String(url).includes('fields.json'))fileFetchFailed=true;throw error});\nconst connectFixture=window.connectElgatoStreamDeckSocket;window.connectElgatoStreamDeckSocket=(...args)=>{connectFixture(...args);setTimeout(async()=>{const action=JSON.parse(args[4]).action;try{${interactions}
const controlBounds=[...document.querySelectorAll('#fields input,#fields select')].filter(element=>element.getClientRects().length).map(element=>{const r=element.getBoundingClientRect();return{id:element.id,left:r.left,right:r.right,width:r.width,height:r.height}});
if(innerWidth<550||document.documentElement.scrollWidth>innerWidth+1||controlBounds.some(r=>r.width<=0||r.height<=0||r.left<0||r.right>innerWidth+1))throw Error('Inspector controls overflow horizontally: '+JSON.stringify(controlBounds));
const unavailable=[...document.querySelectorAll('#fields [data-field]')].filter(row=>row.querySelector('[data-key]')?.disabled||[...row.querySelectorAll('option')].some(option=>option.disabled)).map(row=>({field:row.dataset.field,notice:row.querySelector('.fieldnote')?.textContent}));
if(unavailable.some(item=>!item.notice?.includes('not available')))throw Error('Unavailable control has no explanation');
const fields=[...document.querySelectorAll('#fields [data-field]')].map(row=>({key:row.dataset.field,hidden:row.hidden,tag:row.querySelector('[data-key]')?.tagName,disabled:row.querySelector('[data-key]')?.disabled}));await realFetch(${JSON.stringify(endpoint)},{method:'POST',body:JSON.stringify({action,fields,fileFetchFailed,title:document.querySelector('#title').textContent,error:document.querySelector('#error').textContent,notes:document.querySelector('#fields').textContent,unknown:settings.future,settings,controlBounds,unavailable,viewport:[innerWidth,innerHeight],horizontal_overflow:false})})}catch(error){await realFetch(${JSON.stringify(endpoint)},{method:'POST',body:JSON.stringify({action,error:String(error)})})}},900)};`;
      fs.appendFileSync(path.join(dir, "app.js"), fixture);
      const log = fs.openSync(path.join(output, action + ".log"), "w");
      const proof = new Promise((resolve, reject) => {
        pending.set(action, resolve);
        setTimeout(
          () => reject(Error("Native inspector timeout: " + action)),
          12000,
        ).unref();
      });
      child = spawn(
        executable,
        [
          "webview",
          "--visible",
          "--path",
          path.join(dir, "index.html"),
          "--port",
          String(server.address().port),
          "--uuid",
          "fixture-token",
          "--register-event",
          "registerPropertyInspector",
          "--info",
          "{}",
          "--action-info",
          JSON.stringify({
            action,
            context: "fixture-action",
            payload: { settings: { future: { retained: true } } },
          }),
        ],
        {
          stdio: ["ignore", log, log],
          env: {
            ...process.env,
            XDG_CONFIG_HOME: path.join(temp, "config"),
            XDG_STATE_HOME: path.join(temp, "state"),
          },
          detached: true,
        },
      );
      let value;
      try {
        value = await proof;
      } finally {
        child.kill("SIGTERM");
        await new Promise((resolve) => child.once("exit", resolve));
        fs.closeSync(log);
        child = null;
      }
      reports.push(value);
      if (negative) {
        if (value.fields.length !== 0 || !value.fileFetchFailed)
          throw Error(
            "Expected original file-fetch blank form failure not reproduced",
          );
      } else {
        if (value.error)
          throw Error("Native interaction failed: " + JSON.stringify(value));
        const expected = Object.keys(schema[action].fields);
        if (
          JSON.stringify(value.fields.map((field) => field.key).sort()) !==
          JSON.stringify(expected.sort())
        )
          throw Error(
            "Missing form fields: " + action + " " + JSON.stringify(value),
          );
        if (!expected.length && !value.notes.includes("no additional settings"))
          throw Error("Missing explicit empty form note");
      }
    }
    fs.writeFileSync(
      path.join(output, "forms.json"),
      JSON.stringify(
        {
          ok: true,
          negative,
          binary_sha256: crypto
            .createHash("sha256")
            .update(fs.readFileSync(executable))
            .digest("hex"),
          source_sha256: Object.fromEntries(
            ["index.html", "app.js", "fields.json", "fields.js"]
              .filter((file) => fs.existsSync(path.join(source, file)))
              .map((file) => [
                file,
                crypto
                  .createHash("sha256")
                  .update(fs.readFileSync(path.join(source, file)))
                  .digest("hex"),
              ]),
          ),
          reports,
          settingsEvents,
          no_discord_auth: true,
          no_usb: true,
        },
        null,
        2,
      ) + "\n",
    );
    console.log(
      "PASS: native Discord property-inspector " +
        (negative
          ? "original blank form reproduced"
          : "all17 action field schemas rendered"),
    );
  } catch (error) {
    fs.writeFileSync(
      path.join(output, "forms.json"),
      JSON.stringify({ ok: false, error: String(error), reports }, null, 2) +
        "\n",
    );
    throw error;
  } finally {
    if (child) child.kill("SIGTERM");
    wss.clients.forEach((ws) => ws.terminate());
    await new Promise((resolve) => wss.close(resolve));
    await new Promise((resolve) => server.close(resolve));
    fs.rmSync(temp, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
