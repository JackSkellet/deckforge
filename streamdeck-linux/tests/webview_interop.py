#!/usr/bin/env python3
"""Exercise actual native HTML/plugin-inspector helpers without touching USB.

The contract fixture is original test code. The Counter test changes only a
temporary manifest to wrap its byte-identical official property inspector HTML
in an instrumented iframe. It sets the real sdpi-range control programmatically;
this is not a claim of manual clicking or an unmodified-manifest UI workflow.
The shipped tutorial test proves HTML registration only, not popup interaction.
Run in a graphical Linux session (or an Xvfb session with WebKit available).
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import time

from sample_interop import COUNTER, SAMPLE_COMMIT, RunningHost, require, slot, tap

PROBE_PLUGIN = "com.deckforge.webview-probe"
PROBE_ACTION = PROBE_PLUGIN + ".action"
TUTORIAL_SHA256 = "ee19cf965f58597e51b33abef1207ba6fc672fe31b851f05545d7cdb15275866"
COUNTER_PI_SHA256 = "06f092dfb14de3a0e29c6b0bb1518a229002f7f277e22643e9ae205d7abe5fac"

PLUGIN_HTML = r'''<!doctype html><html><head><meta charset="utf-8"></head><body><script>
window.connectElgatoStreamDeckSocket = (port, uuid, event, info, actionInfo) => {
    const details = JSON.parse(info);
    const ws = new WebSocket(`ws://127.0.0.1:${port}`);
    const send = (event, context, payload) => ws.send(JSON.stringify({event, context, payload}));
    const report = value => send('logMessage', uuid, {message:'WEBVIEW_PROBE ' + JSON.stringify(value)});
    let popupWindow = null;
    window.addEventListener('message', event => {
        if (event.source === popupWindow && event.data && event.data.deckforgePopup)
            report({kind:'popup_result', ...event.data.deckforgePopup});
    });
    ws.onopen = () => {
        ws.send(JSON.stringify({event,uuid}));
        report({kind:'plugin_callback',portType:typeof port,event,plugin:details.plugin.uuid,actionInfo:actionInfo || null});
    };
    ws.onmessage = ({data}) => {
        const m = JSON.parse(data);
        if (m.event === 'willAppear') send('setTitle',m.context,{title:'HTML ready'});
        if (m.event === 'keyUp') {
            popupWindow = window.open('popup.html', 'Tutorial');
            report({kind:'popup_open_result',returnedWindow:!!popupWindow});
        }
        if (['propertyInspectorDidAppear','propertyInspectorDidDisappear'].includes(m.event))
            report({kind:m.event,context:m.context,action:m.action,device:m.device});
        if (m.event === 'didReceiveSettings' && m.payload.settings.fromPI === 42 && m.payload.settings.fromPlugin !== 43) {
            report({kind:'plugin_received_settings',context:m.context});
            send('setSettings',m.context,{...m.payload.settings,fromPlugin:43});
        }
        if (m.event === 'didReceiveGlobalSettings' && m.payload.settings.fromPIGlobal === 44 && m.payload.settings.fromPluginGlobal !== 45)
            send('setGlobalSettings',uuid,{...m.payload.settings,fromPluginGlobal:45});
        if (m.event === 'sendToPlugin') {
            if (m.payload.ping === 'roundtrip') send('sendToPropertyInspector',m.context,{pong:'roundtrip'});
            if (m.payload.finished) report({kind:'inspector_complete',context:m.context,results:m.payload.finished});
            if (m.payload.failure) report({kind:'failure',message:m.payload.failure});
        }
    };
};
</script></body></html>'''

POPUP_HTML = r'''<!doctype html><html><head><meta charset="utf-8"></head><body>Popup execution probe<script>
if (window.opener) {
    // file:// documents may have distinct opaque origins. postMessage preserves
    // normal browser origin isolation while proving the child's opener relation.
    window.opener.postMessage({deckforgePopup:{opener:true,document:location.pathname,executed:true}}, '*');
    window.close();
}
</script></body></html>'''

INSPECTOR_HTML = r'''<!doctype html><html><head><meta charset="utf-8"></head><body>Contract inspector<script>
window.connectElgatoStreamDeckSocket = (port, uuid, event, info, actionInfo) => {
    const details = JSON.parse(info), action = JSON.parse(actionInfo), results = {};
    const ws = new WebSocket(`ws://127.0.0.1:${port}`);
    const send = (event,payload,extra={}) => ws.send(JSON.stringify({event,context:uuid,action:action.action,payload,...extra}));
    let started = false, finished = false;
    const fail = message => send('sendToPlugin',{failure:message});
    const check = () => {
        if (!finished && results.settings && results.global && results.message && results.initial) {
            finished = true;
            send('sendToPlugin',{finished:results});
        }
    };
    ws.onopen = () => {
        ws.send(JSON.stringify({event,uuid}));
        if (event !== 'registerPropertyInspector' || typeof port !== 'string' || !action.context || !details.plugin.uuid)
            return fail('invalid five-argument PI callback');
        results.distinctIdentity = uuid !== action.context;
        send('getSettings',undefined,{id:'initial'});
        send('getGlobalSettings',undefined,{id:'global-initial'});
        send('sendToPlugin',{ping:'roundtrip'});
    };
    ws.onmessage = ({data}) => {
        const m = JSON.parse(data);
        if (m.event === 'didReceiveSettings') {
            if (m.context !== action.context || m.action !== action.action || m.device !== action.device)
                return fail('settings response used PI token or wrong action/device');
            if (!started) {
                started = true; results.initial = true;
                send('setSettings',{fromPI:42});
                send('setGlobalSettings',{fromPIGlobal:44});
            }
            if (m.payload.settings.fromPlugin === 43) results.settings = true;
        }
        if (m.event === 'didReceiveGlobalSettings' && m.payload.settings.fromPluginGlobal === 45) results.global = true;
        if (m.event === 'sendToPropertyInspector' && m.payload.pong === 'roundtrip') {
            if (m.context !== action.context || m.action !== action.action) return fail('message response context/action mismatch');
            results.message = true;
        }
        check();
    };
};
</script></body></html>'''

COUNTER_WRAPPER = r'''<!doctype html><html><head><meta charset="utf-8"></head><body>
<iframe id="official" style="border:0;width:100%;height:200px"></iframe>
<script>
document.getElementById('official').srcdoc = __ORIGINAL_HTML__;
window.connectElgatoStreamDeckSocket = async (...args) => {
    try {
    const frame = document.getElementById('official');
    const until = async predicate => {
        for (let i=0;i<300;i++) { if (predicate()) return; await new Promise(r=>setTimeout(r,100)); }
        throw new Error('Official Counter PI control did not initialize');
    };
    await until(()=>frame.contentWindow.SDPIComponents && frame.contentWindow.connectElgatoStreamDeckSocket);
    frame.contentWindow.connectElgatoStreamDeckSocket(...args);
    await frame.contentWindow.SDPIComponents.streamDeckClient.getSettings();
    const range = frame.contentDocument.querySelector('sdpi-range[setting="incrementBy"]');
    await until(()=>range && typeof range.save === 'function');
    range.value = 4;
    } catch (error) {
        // On startup failure the original PI has not registered; surface it via
        // a temporary PI socket instead of silently waiting on an async error.
        const [port,uuid,event] = args;
        const debug = new WebSocket(`ws://127.0.0.1:${port}`);
        debug.onopen = () => {
            debug.send(JSON.stringify({event,uuid}));
            debug.send(JSON.stringify({event:'logMessage',context:uuid,payload:{message:'WEBVIEW_PROBE '+JSON.stringify({kind:'failure',message:String(error)})}}));
        };
    }
};
</script></body></html>'''


def wait(host, predicate, description, seconds=45):
    deadline = time.monotonic() + seconds
    snapshot = None
    while time.monotonic() < deadline:
        snapshot = host.state()
        failures = [s for s in snapshot.get("logs", []) if '"kind":"failure"' in s]
        require(not failures, "Native webview contract failure: " + str(failures))
        if predicate(snapshot):
            return snapshot
        time.sleep(0.1)
    for item in (snapshot or {}).get("slots", []):
        item.pop("preview", None)
        item.pop("background_png", None)
    raise AssertionError(f"Timed out waiting for {description}: {snapshot}\n{host.log.read_text()}")


def probe_events(snapshot):
    results = []
    for line in snapshot.get("logs", []):
        if "WEBVIEW_PROBE " in line:
            results.append(json.loads(line.split("WEBVIEW_PROBE ", 1)[1]))
    return results


def make_probe(root):
    root.mkdir()
    (root / "manifest.json").write_text(json.dumps({
        "UUID": PROBE_PLUGIN, "Name": "HTML conformance probe", "Version": "1.0.0",
        "CodePath": "app.html", "PropertyInspectorPath": "inspector.html", "SDKVersion": 2,
        "Actions": [{"UUID": PROBE_ACTION, "Name": "HTML probe", "Controllers": ["Keypad"], "States": [{}]}],
        "OS": [{"Platform": "windows", "MinimumVersion": "10"}],
        "Software": {"MinimumVersion": "6.5"},
    }))
    (root / "app.html").write_text(PLUGIN_HTML)
    (root / "inspector.html").write_text(INSPECTOR_HTML)
    (root / "popup.html").write_text(POPUP_HTML)
    return root


def instrument_counter(source, target):
    shutil.copytree(source, target, ignore=shutil.ignore_patterns("logs", "node_modules"))
    original = source / "ui/increment-counter.html"
    require(hashlib.sha256(original.read_bytes()).hexdigest() == COUNTER_PI_SHA256, "Official Counter PI source pin changed")
    require(original.read_bytes() == (target / "ui/increment-counter.html").read_bytes(), "Counter PI changed while copying")
    original_html = json.dumps(original.read_bytes().decode("utf-8")).replace("<", "\\u003c")
    (target / "ui/interop-wrapper.html").write_text(COUNTER_WRAPPER.replace("__ORIGINAL_HTML__", original_html))
    manifest_path = target / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    next(a for a in manifest["Actions"] if a["UUID"] == COUNTER)["PropertyInspectorPath"] = "ui/interop-wrapper.html"
    manifest_path.write_text(json.dumps(manifest))
    return target, hashlib.sha256(original.read_bytes()).hexdigest()


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/deckforge")
    parser.add_argument("--evidence", type=Path, default=project.parent / "evidence")
    parser.add_argument("--skip-counter-control", action="store_true", help="Skip CDN-dependent official control instrumentation")
    parser.add_argument("--skip-tutorial", action="store_true")
    args = parser.parse_args()
    binary, evidence = args.binary.resolve(), args.evidence.resolve()
    counter_source = evidence / "official-samples/counter/com.elgato.counter.sdPlugin"
    require((counter_source / "bin/plugin.js").is_file(), "Build official Counter before running this test")
    with tempfile.TemporaryDirectory(prefix="deckforge-webview-") as temporary:
        temporary = Path(temporary)
        probe = make_probe(temporary / (PROBE_PLUGIN + ".sdPlugin"))
        counter, counter_html_hash = instrument_counter(counter_source, temporary / "com.elgato.counter.sdPlugin")
        host = RunningHost(binary, [counter, probe], temporary / "state", temporary / "host.log")
        try:
            host.ready()
            host.post({"command": "assign", "controller": "Keypad", "index": 1, "uuid": PROBE_ACTION})
            snapshot = wait(host, lambda s: slot(s, PROBE_ACTION).get("title") == "HTML ready", "HTML plugin callback and action response")
            context = slot(snapshot, PROBE_ACTION)["context"]
            host.post({"command": "input", "event": "keyDown", "index": 1})
            host.post({"command": "input", "event": "keyUp", "index": 1})
            snapshot = wait(host, lambda s: any(e.get("kind") == "popup_result"
                                                or (e.get("kind") == "popup_open_result" and not e.get("returnedWindow"))
                                                for e in probe_events(s)),
                            "native popup document to execute and call its opener")
            require(not any(e.get("kind") == "popup_open_result" and not e.get("returnedWindow") for e in probe_events(snapshot)),
                    "Native HTML backend window.open returned null; WebSocket-triggered plugin popups are blocked")
            popup = next(e for e in probe_events(snapshot) if e.get("kind") == "popup_result")
            require(popup.get("executed") and popup.get("opener") and popup["document"].endswith("/popup.html"),
                    "Popup did not execute its own document with the expected opener")
            host.post({"command": "open_inspector", "context": context})
            snapshot = wait(host, lambda s: any(e.get("kind") == "inspector_complete" for e in probe_events(s)), "native PI settings/global/message contract")
            events = probe_events(snapshot)
            result = next(e for e in events if e.get("kind") == "inspector_complete")
            require(result["context"] == context, "PI message did not map to actual slot")
            require(result["results"].get("distinctIdentity"), "PI authentication token reused public action context")
            require(any(e.get("kind") == "propertyInspectorDidAppear" and e.get("context") == context for e in events), "Plugin did not receive PI appearance")
            require(slot(snapshot, PROBE_ACTION)["settings"] == {"fromPI": 42, "fromPlugin": 43}, "Cross-runtime action settings mismatch")
            host.post({"command": "close_inspector", "context": context})
            wait(host, lambda s: any(e.get("kind") == "propertyInspectorDidDisappear" and e.get("context") == context for e in probe_events(s)), "PI disappearance")
            if not args.skip_counter_control:
                counter_context = slot(host.state(), COUNTER)["context"]
                host.post({"command": "open_inspector", "context": counter_context})
                wait(host, lambda s: slot(s, COUNTER).get("settings", {}).get("incrementBy") == 4, "original Counter sdpi-range setting save")
                before = slot(host.state(), COUNTER).get("settings", {}).get("count", 0)
                tap(host)
                wait(host, lambda s: slot(s, COUNTER).get("settings", {}).get("count") == before + 4, "Counter action using real PI setting")
                host.post({"command": "close_inspector", "context": counter_context})
        finally:
            host.stop()
        saved = json.loads((temporary / "state/session.json").read_text())
        require(saved["globals"][PROBE_PLUGIN] == {"fromPIGlobal": 44, "fromPluginGlobal": 45}, "Global settings propagation did not persist")

        if not args.skip_tutorial:
            archive = evidence / "elgato-7.6/raw/flsCs3U5bpGlHPaoJZvml3X4InTgss"
            require(hashlib.sha256(archive.read_bytes()).hexdigest() == TUTORIAL_SHA256, "Tutorial artifact source pin changed")
            tutorial = evidence / "plugins/com.elgato.tutorial.sdPlugin"
            require((tutorial / "app.html").is_file(), "Install inert official tutorial fixture before running this test")
            tutorial_host = RunningHost(binary, [counter_source, tutorial], temporary / "tutorial-state", temporary / "tutorial.log")
            try:
                tutorial_host.ready()
                require(any(p["id"] == "com.elgato.tutorial" and p["connected"] for p in tutorial_host.state()["plugins"]), "Unchanged tutorial HTML did not register")
                direct_context = slot(tutorial_host.state(), COUNTER)["context"]
                tutorial_host.post({"command": "open_inspector", "context": direct_context})
                wait(tutorial_host, lambda s: any(i["context"] == direct_context and i["connected"]
                                                 for i in s.get("inspectors", [])),
                     "unchanged Counter PI opening directly with its original manifest")
                tutorial_host.post({"command": "close_inspector", "context": direct_context})
            finally:
                tutorial_host.stop()
    print("PASS: real native helper executed generated HTML conformance plugin/PI: registration, bound identities, settings/global propagation, messages, appearance/disappearance.")
    print("PASS: generated HTML plugin opened a native popup; its child document executed JavaScript and called window.opener through the original plugin socket. No tutorial visual-rendering claim.")
    if not args.skip_counter_control:
        print(f"PASS: official Counter PI HTML sha256={counter_html_hash}, sample revision={SAMPLE_COMMIT}; instrumented original sdpi-range saved incrementBy=4 and Counter consumed it. No manual UI-click claim.")
    if not args.skip_tutorial:
        print("PASS: unchanged shipped tutorial HTML plugin registered in native helper; popup behavior remains unverified.")
        print("PASS: unchanged Counter PI also registered directly through its original manifest; no iframe or source instrumentation on this launch.")
    print("USB was never opened.")


if __name__ == "__main__":
    main()
