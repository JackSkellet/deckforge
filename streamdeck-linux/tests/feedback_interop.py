#!/usr/bin/env python3
"""Real WebSocket feedback rendering/timing with --simulate and native durations.

No USB, keyboard, audio, external URL, or desktop input is used. Key badges use
real 3000ms timers; dial alerts use the real 330ms animation. No timing override
is installed. Native UI screenshots remain a separate check. The optional
unchanged official Image Resizer fixture exercises its invalid-dimensions alert.
"""
import argparse
import base64
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import time
import uuid

from PIL import Image, ImageChops
from sample_interop import require
from system_interop import Host

PRIMARY = "com.deckforge.feedback-probe"
FOREIGN = "com.deckforge.feedback-foreign"
TRIGGER = "com.elgato.streamdeck.keys.adaptor"
STACK = "com.elgato.streamdeck.dial.stack"
MANIFEST_HINTS = {"rotate": "Manifest turn", "push": "Manifest press", "touch": "Manifest tap", "longTouch": "Manifest hold"}
STATES = [{"Image": "base.png", "Title": "Base"}, {"Image": "alternate.png", "Title": "Alternate"}]
CELLS = [(12, 29, 68, 85), (72, 14, 128, 70), (132, 29, 188, 85)]


def make_plugin(path, plugin, require_root):
    path.mkdir()
    proof = path / "proof.jsonl"
    script = r'''import {createRequire} from 'node:module';
import {appendFileSync} from 'node:fs';
const WebSocket=createRequire(__REQUIRE__)('ws'),proof=__PROOF__,args={};
for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket('ws://127.0.0.1:'+args['-port']),seen=new Set();let seq=0;
const record=(kind,data={})=>appendFileSync(proof,JSON.stringify({seq:++seq,kind,...data})+'\n');
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{try{
 const m=JSON.parse(data.toString());
 if(['willAppear','willDisappear'].includes(m.event))record(m.event,{context:m.context});
 if(m.event==='didReceiveSettings'&&m.id){record('ack',{nonce:m.id,context:m.context});return;}
 const request=m.event==='didReceiveSettings'&&m.payload?.settings?.feedbackProbe;
 if(!request?.nonce||seen.has(request.nonce))return;seen.add(request.nonce);
 for(const command of request.commands){
   const context=command.context||m.context;
   record('sent',{nonce:request.nonce,event:command.event,context});
   ws.send(JSON.stringify({event:command.event,context,payload:command.payload||{}}));
 }
 ws.send(JSON.stringify({event:'getSettings',context:m.context,id:request.nonce}));
}catch(error){record('failure',{message:String(error)});}});
'''
    (path / "plugin.mjs").write_text(script.replace("__REQUIRE__", json.dumps(str(require_root)))
                                    .replace("__PROOF__", json.dumps(str(proof))))
    (path / "manifest.json").write_text(json.dumps({
        "UUID": plugin, "Name": "Feedback probe", "CodePath": "plugin.mjs", "SDKVersion": 2,
        "Nodejs": {"Version": "20"}, "Actions": [
            {"UUID": plugin + ".key", "Name": "Feedback key", "Controllers": ["Keypad"], "States": STATES},
            {"UUID": plugin + ".dial", "Name": "Feedback dial", "Controllers": ["Encoder"], "States": STATES,
             "Encoder": {"layout": "$B1", "TriggerDescription": {
                 "Rotate": "Manifest turn", "Push": "Manifest press", "Touch": "Manifest tap", "LongTouch": "Manifest hold"}}},
        ],
    }))
    for name, color in [("base.png", (8, 16, 36)), ("alternate.png", (42, 8, 60)), ("replacement.png", (185, 22, 180))]:
        Image.new("RGB", (120, 120), color).save(path / name)
    return path, proof


def rows(proof):
    if not proof.exists():
        return []
    return [json.loads(line) for line in proof.read_text().splitlines(keepends=True) if line.endswith("\n")]


def all_nodes(snapshot):
    unique = {}
    def visit(value):
        if isinstance(value, list):
            for child in value:
                visit(child)
        elif isinstance(value, dict):
            if value.get("context"):
                unique.setdefault(value["context"], value)
            for name in ("stack_children", "trigger_children", "multi_children"):
                visit(value.get(name))
    visit(snapshot.get("slots", []))
    return list(unique.values())


def node(host, context):
    return next(s for s in all_nodes(host.state()) if s["context"] == context)


def at(host, index, controller="Keypad"):
    return next(s for s in host.state()["slots"] if s["controller"] == controller and s["index"] == index)


def preview(value):
    with Image.open(io.BytesIO(base64.b64decode(value["preview"]))) as image:
        result = image.convert("RGB")
        require(result.size in ((120, 120), (200, 100)), "Unexpected key/dial preview dimensions")
        return result


def equal(left, right):
    return left.size == right.size and ImageChops.difference(left, right).getbbox() is None


def badge_pixels(image, kind):
    if kind == "ok":
        return sum(g > 70 and g > r * 1.3 and g > b + 8 for r, g, b in image.getdata())
    return sum(r > 70 and g > 60 and b < min(r, g) * .65 for r, g, b in image.getdata())


def wait(host, predicate, description, timeout=5, interval=.025):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        host.check_process()
        result = predicate()
        if result:
            return result
        time.sleep(interval)
    raise AssertionError(f"Timed out waiting for {description}\n{host.log.read_text()[-5000:]}")


def wait_until(deadline):
    remaining = deadline - time.monotonic()
    if remaining > 0:
        time.sleep(remaining)


def issue(host, proof, source, commands, *, acknowledge=True):
    settings = copy.deepcopy(node(host, source).get("settings", {}))
    nonce = uuid.uuid4().hex
    settings["feedbackProbe"] = {"nonce": nonce, "commands": commands}
    started = time.monotonic()
    host.post({"command": "settings", "context": source, "settings": settings})
    if acknowledge:
        wait(host, lambda: any(r["kind"] == "ack" and r.get("nonce") == nonce for r in rows(proof)), "same-socket SDK barrier")
    require(not [r for r in rows(proof) if r["kind"] == "failure"], "Generated plugin fixture failed")
    return started


def event(name, context=None, **payload):
    result = {"event": name, "payload": payload}
    if context is not None:
        result["context"] = context
    return result


def assign(host, plugin, index, controller="Keypad"):
    host.post({"command": "assign", "controller": controller, "index": index,
               "uuid": plugin + (".dial" if controller == "Encoder" else ".key")})
    return at(host, index, controller)["context"]


def child(kind, name):
    return {"UUID": PRIMARY + "." + kind, "ActionID": str(uuid.uuid4()), "Name": name,
            "Settings": {}, "State": 0, "States": copy.deepcopy(STATES)}


def configure_trigger(host, index):
    host.post({"command": "assign", "controller": "Encoder", "index": index, "uuid": TRIGGER})
    parent = at(host, index, "Encoder")
    host.post({"command": "trigger_save", "context": parent["context"], "raw": {
        "Settings": {}, "Actions": [child("key", name) for name in ("Left", "Center", "Right")]}})
    return at(host, index, "Encoder")


def evidence_image(output, name, image):
    if output:
        image.save(output / name)


def timed_key_badges(host, proof, key, reference, output):
    base = preview(node(host, key))
    started = issue(host, proof, key, [event("showOk")])
    wait(host, lambda: badge_pixels(preview(node(host, key)), "ok") > 30, "green key check")
    evidence_image(output, "key-ok.png", preview(node(host, key)))
    wait_until(started + 2.55)
    require(badge_pixels(preview(node(host, key)), "ok") > 30, "Key badge expired before its native 3-second lifetime")
    wait(host, lambda: equal(preview(node(host, key)), base), "key image restore", timeout=2)
    first_duration = time.monotonic() - started
    require(2.8 <= first_duration <= 4.6, f"Wrong key badge duration: {first_duration:.3f}s")

    # State, title and image must continue changing beneath the transparent badge.
    started = issue(host, proof, key, [event("showAlert")])
    wait(host, lambda: badge_pixels(preview(node(host, key)), "alert") > 30, "yellow key triangle")
    updates = []
    for target in (key, reference):
        updates.extend([event("setState", target, state=1), event("setTitle", target, title="Updated beneath badge"),
                        event("setImage", target, image="replacement.png")])
    issue(host, proof, key, updates)
    expected = preview(node(host, reference))
    require(node(host, key)["state"] == 1 and node(host, key)["title"] == "Updated beneath badge", "Badge blocked underlying state/title updates")
    require(badge_pixels(preview(node(host, key)), "alert") > 30, "Underlying update removed the visible alert")
    evidence_image(output, "key-alert-underlying.png", preview(node(host, key)))
    wait(host, lambda: equal(preview(node(host, key)), expected), "updated content after badge expiry", timeout=4.6)
    require(time.monotonic() - started >= 2.8, "Underlying update shortened the alert duration")
    evidence_image(output, "key-restored.png", preview(node(host, key)))

    # One native-duration chain distinguishes replacement from same-kind retrigger.
    original = issue(host, proof, key, [event("showOk")])
    wait_until(original + 1.25)
    replaced = issue(host, proof, key, [event("showAlert")])
    require(badge_pixels(preview(node(host, key)), "alert") > 30, "showAlert did not replace showOk")
    require(badge_pixels(preview(node(host, key)), "ok") == 0, "Previous green check survived replacement")
    wait_until(original + 3.2)
    require(badge_pixels(preview(node(host, key)), "alert") > 30, "Replacement kept the original expiry deadline")
    retriggered = issue(host, proof, key, [event("showAlert")])
    wait_until(replaced + 3.2)
    require(badge_pixels(preview(node(host, key)), "alert") > 30, "Retrigger did not extend the alert deadline")
    wait(host, lambda: equal(preview(node(host, key)), expected), "retriggered badge restore", timeout=3)
    retrigger_duration = time.monotonic() - retriggered
    require(2.8 <= retrigger_duration <= 4.6, "Retriggered badge did not use native 3-second duration")
    return {"key_restore_seconds": first_duration, "retrigger_restore_seconds": retrigger_duration}


def dial_alert(host, proof, source, target, display, output=None, prefix="dial"):
    base = preview(node(host, display))
    corners = [(0, 0), (199, 99)]
    def strength(image):
        values = []
        for position in corners:
            before, after = base.getpixel(position), image.getpixel(position)
            require(before[0] < 230, "Test corner lacks red-tint contrast")
            values.append((after[0] - before[0]) / (255 - before[0]))
        return min(values)
    started = issue(host, proof, source, [event("showAlert", target)], acknowledge=False)
    samples, strong, fading, restored = [], None, None, None
    while time.monotonic() - started < .8:
        image = preview(node(host, display))
        elapsed, alpha = time.monotonic() - started, strength(image)
        samples.append({"seconds": elapsed, "red_strength": alpha})
        if alpha >= .60 and strong is None:
            strong = image.copy()
        if .08 < alpha < .60 and fading is None:
            fading = image.copy()
        if strong is not None and equal(image, base):
            restored = elapsed
            break
        time.sleep(.012)
    require(strong is not None, "Dial alert never tinted both opposite corners of the full cell")
    require(fading is not None, "Dial alert did not expose a fading red phase")
    require(restored is not None and .25 <= restored <= .8, f"Dial alert failed to restore around native 330ms: {restored}")
    evidence_image(output, prefix + "-alert-strong.png", strong)
    evidence_image(output, prefix + "-alert-fading.png", fading)
    return {"restore_seconds": restored, "samples": samples}


def unchanged_for(host, display, expected, duration=.15):
    until = time.monotonic() + duration
    while time.monotonic() < until:
        require(equal(preview(node(host, display)), expected), "An ignored/foreign command changed the visible display")
        time.sleep(.02)


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/release/deckforge")
    parser.add_argument("--evidence-output", type=Path)
    parser.add_argument("--official-image-resizer", action="store_true", help="Requires the unchanged official sample to be built beforehand")
    args = parser.parse_args()
    evidence = project.parent / "evidence"
    output = args.evidence_output.resolve() if args.evidence_output else None
    if output:
        output.mkdir(parents=True, exist_ok=True)
    official = evidence / "official-samples/image-resizer/com.elgato.image-resizer.sdPlugin"
    official_hashes = {}
    if args.official_image_resizer:
        for path in [official / "bin/plugin.js", official / "manifest.json"]:
            require(path.is_file(), "Build the unchanged official Image Resizer fixture first")
            official_hashes[path] = hashlib.sha256(path.read_bytes()).hexdigest()
    metrics = {"official_image_resizer": "not requested", "multi_parent_badge_forwarding": "not asserted; contract unproven"}
    with tempfile.TemporaryDirectory(prefix="deckforge-feedback-") as directory:
        root = Path(directory)
        primary_path, proof = make_plugin(root / (PRIMARY + ".sdPlugin"), PRIMARY, evidence / "official-samples/counter/package.json")
        foreign_path, foreign_proof = make_plugin(root / (FOREIGN + ".sdPlugin"), FOREIGN, evidence / "official-samples/counter/package.json")
        plugins = [primary_path, foreign_path] + ([official] if args.official_image_resizer else [])
        host = Host(args.binary.resolve(), plugins, root / "state", root / "host.log")
        try:
            host.ready()
            host.wait(lambda s: all(p["connected"] for p in s["plugins"]), "feedback probes connected")
            host.post({"command": "profile_create", "name": "Feedback conformance"})
            key = assign(host, PRIMARY, 0)
            foreign = assign(host, FOREIGN, 1)
            reference = assign(host, PRIMARY, 2)
            dial = assign(host, PRIMARY, 0, "Encoder")
            metrics.update(timed_key_badges(host, proof, key, reference, output))
            base = preview(node(host, key))
            issue(host, foreign_proof, foreign, [event("showAlert", key), event("showOk", key)])
            unchanged_for(host, key, base)

            # Runtime descriptions are replacement maps; {} restores manifest.
            require(node(host, dial)["trigger_description"] == MANIFEST_HINTS, "Manifest trigger descriptions missing")
            issue(host, proof, key, [event("setTriggerDescription", dial, rotate="Fine adjustment", touch="")])
            require(node(host, dial)["trigger_description"] == {"rotate": "Fine adjustment", "touch": ""}, "Sparse description patch merged omitted fields")
            issue(host, foreign_proof, foreign, [event("setTriggerDescription", dial, rotate="Foreign owner")])
            require(node(host, dial)["trigger_description"] == {"rotate": "Fine adjustment", "touch": ""}, "Foreign plugin changed dial descriptions")
            issue(host, proof, key, [event("setTriggerDescription", dial, rotate=17, push="Do not partially apply")])
            require(node(host, dial)["trigger_description"] == {"rotate": "Fine adjustment", "touch": ""}, "Invalid description changed valid previous state")
            issue(host, proof, key, [event("setTriggerDescription", dial)])
            require(node(host, dial)["trigger_description"] == MANIFEST_HINTS, "Empty description payload did not restore manifest")
            metrics["dial_alert"] = dial_alert(host, proof, key, dial, dial, output)
            dial_base = preview(node(host, dial))
            issue(host, proof, key, [event("showOk", dial)])
            unchanged_for(host, dial, dial_base)

            # Hidden key feedback neither paints another page nor replays on return.
            original_page = host.state()["library"]["active_page"]
            host.post({"command": "page_create", "name": "Hidden feedback target"})
            messenger = assign(host, PRIMARY, 0)
            messenger_base = preview(node(host, messenger))
            issue(host, proof, messenger, [event("showAlert", key), event("showOk", key)])
            unchanged_for(host, messenger, messenger_base, .1)
            host.post({"command": "page_select", "id": original_page})
            unchanged_for(host, key, base, .15)

            # Inactive stack alert is ignored; selected child tints the whole cell.
            host.post({"command": "assign", "controller": "Encoder", "index": 1, "uuid": STACK})
            stack = at(host, 1, "Encoder")
            host.post({"command": "stack_save", "context": stack["context"], "raw": {
                "Settings": {"CurrentIdx": 0, "ShowStackIcon": False},
                "Actions": [child("dial", "First"), child("dial", "Second")]}})
            stack = at(host, 1, "Encoder")
            first, second = [s["context"] for s in stack["stack_children"]]
            stack_base = preview(stack)
            issue(host, proof, key, [event("showAlert", second)])
            require(equal(preview(node(host, stack["context"])), stack_base), "Inactive stack child painted active cell")
            host.post({"command": "stack_select", "context": stack["context"], "index": 1})
            unchanged_for(host, stack["context"], stack_base, .15)
            metrics["stack_alert"] = dial_alert(host, proof, key, second, stack["context"], output, "stack")
            issue(host, proof, key, [event("setTriggerDescription", second, rotate="Selected adjustment", touch="Selected tap")])
            require(node(host, stack["context"])["trigger_description"].get("rotate") == "Selected adjustment", "Stack did not expose selected child's effective description")
            issue(host, proof, key, [event("setTriggerDescription", first, rotate="Hidden adjustment")])
            require(node(host, stack["context"])["trigger_description"].get("rotate") == "Selected adjustment", "Inactive child description replaced the selected child's hint")

            # Two real child badges remain within their mini-icon rectangles.
            trigger = configure_trigger(host, 2)
            clean_trigger = configure_trigger(host, 3)
            trigger_context = trigger["context"]
            ids = [c["context"] for c in trigger["trigger_children"]]
            reference_ids = [c["context"] for c in clean_trigger["trigger_children"]]
            original = preview(trigger)
            started = issue(host, proof, key, [event("showAlert", ids[0]), event("showOk", ids[1])])
            wait(host, lambda: badge_pixels(preview(node(host, trigger_context)).crop(CELLS[0]), "alert") > 3
                 and badge_pixels(preview(node(host, trigger_context)).crop(CELLS[1]), "ok") > 3, "Trigger mini-icon badges")
            badged = preview(node(host, trigger_context))
            diff = ImageChops.difference(original, badged)
            require(all(not any(diff.getpixel((x, y))) or any(a <= x < c and b <= y < d for a, b, c, d in CELLS[:2])
                        for y in range(100) for x in range(200)), "Child badge changed pixels outside its own mini-icon")
            evidence_image(output, "trigger-badges.png", badged)
            changes = []
            for target in (ids[0], reference_ids[0]):
                changes.extend([event("setState", target, state=1), event("setTitle", target, title="Child updated"), event("setImage", target, image="replacement.png")])
            issue(host, proof, key, changes)
            expected = preview(node(host, clean_trigger["context"]))
            wait_until(started + 2.55)
            require(badge_pixels(preview(node(host, trigger_context)).crop(CELLS[1]), "ok") > 3, "Trigger child badge expired early")
            wait(host, lambda: equal(preview(node(host, trigger_context)), expected), "Trigger updated image after expiry", timeout=2)
            metrics["trigger_badge_seconds"] = time.monotonic() - started
            require(2.8 <= metrics["trigger_badge_seconds"] <= 4.6, "Trigger badge did not use native 3-second duration")
            evidence_image(output, "trigger-restored.png", preview(node(host, trigger_context)))

            if args.official_image_resizer:
                action = "com.elgato.image-resizer.resize"
                host.post({"command": "assign", "controller": "Keypad", "index": 3, "uuid": action})
                current = at(host, 3)
                host.post({"command": "settings", "context": current["context"], "settings": {"width": 0, "height": 1}})
                before = preview(at(host, 3))
                for edge in ("keyDown", "keyUp"):
                    host.post({"command": "input", "event": edge, "index": 3})
                wait(host, lambda: badge_pixels(preview(at(host, 3)), "alert") > badge_pixels(before, "alert") + 200, "unchanged official Image Resizer showAlert")
                wait(host, lambda: equal(preview(at(host, 3)), before), "official sample alert restore", timeout=4.6)
                source = root / "sample input.png"
                Image.new("RGB", (7, 9), (35, 95, 160)).save(source)
                resized = root / "sample input (3x4).png"
                host.post({"command": "settings", "context": current["context"], "settings": {
                    "width": 3, "height": 4, "imagePath": str(source), "extension": "png"}})
                for edge in ("keyDown", "keyUp"):
                    host.post({"command": "input", "event": edge, "index": 3})
                wait(host, lambda: resized.exists(), "unchanged official Image Resizer output")
                with Image.open(resized) as result:
                    require(result.size == (3, 4), "Official sample resize dimensions are wrong")
                wait(host, lambda: badge_pixels(preview(at(host, 3)), "ok") > badge_pixels(before, "ok") + 200, "official successful resize feedback")
                wait(host, lambda: equal(preview(at(host, 3)), before), "official success feedback restore", timeout=4.6)
                metrics["official_image_resizer"] = "unchanged invalid-dimensions alert and real temporary-image resize/success passed"
            require(not [row for path in (proof, foreign_proof) for row in rows(path) if row["kind"] == "failure"], "Generated feedback fixture failed")
            if output:
                (output / "metrics.json").write_text(json.dumps(metrics, indent=2) + "\n")
                (output / "host.log").write_text(host.log.read_text())
        finally:
            host.stop()
    for path, expected in official_hashes.items():
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, "Official sample file changed during test")
    print("PASS: native-duration key/Trigger badges, replacement/retrigger, underlying updates, ownership/visibility, 330ms red dial/stack animation, runtime descriptions. No USB, keyboard or audio actions.")
    print("Official sample: " + metrics["official_image_resizer"] + "; Multi parent badge forwarding was not asserted.")


if __name__ == "__main__":
    main()
