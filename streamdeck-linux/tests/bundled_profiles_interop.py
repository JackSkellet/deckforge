#!/usr/bin/env python3
"""Unchanged Elgato LightsOut plus scoped SDK profile-switch conformance.

Uses a temporary installation/state directory and --simulate. PNG assertions
inspect rendered pixels, not merely state numbers. The generated plugin only
drives the documented SDK contract; vendor files are never patched.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zlib

from sample_interop import RunningHost, require

LIGHTS = "com.elgato.lightsout"
PIECE = LIGHTS + ".gamepiece"
PROBE = "com.deckforge.bundle-probe"
ARCHIVE_HASH = "01f0fd16e669f2e6b0b1f5807bd99f94a42e7ea0f07571238713f048372d217c"


def current(snapshot):
    return next(p for p in snapshot["library"]["profiles"] if p["id"] == snapshot["library"]["active_profile"])


def keys(snapshot):
    return {s["index"]: s for s in snapshot["slots"] if s["controller"] == "Keypad"}


def png_corner(encoded):
    """Decode the host's noninterlaced RGB/RGBA 8-bit PNG using only stdlib."""
    data = base64.b64decode(encoded, validate=True)
    require(data.startswith(b"\x89PNG\r\n\x1a\n"), "Preview is not PNG")
    offset, compressed = 8, bytearray()
    width = height = channels = None
    while offset < len(data):
        size = struct.unpack_from(">I", data, offset)[0]
        kind, body = data[offset + 4:offset + 8], data[offset + 8:offset + 8 + size]
        if kind == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
            require(depth == 8 and color in (2, 6) and interlace == 0, "Unexpected preview PNG encoding")
            channels = 3 if color == 2 else 4
        elif kind == b"IDAT":
            compressed.extend(body)
        offset += size + 12
    require(width == 120 and height == 120, "Plus key preview must be 120x120")
    decoded = zlib.decompress(compressed)
    stride, previous = width * channels, bytearray(width * channels)
    for y in range(6):
        begin = y * (stride + 1)
        filter_type = decoded[begin]
        row = bytearray(decoded[begin + 1:begin + 1 + stride])
        for i in range(stride):
            left = row[i - channels] if i >= channels else 0
            up = previous[i]
            diagonal = previous[i - channels] if i >= channels else 0
            if filter_type == 0:
                predictor = 0
            elif filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = up
            elif filter_type == 3:
                predictor = (left + up) // 2
            elif filter_type == 4:
                estimate = left + up - diagonal
                distances = (abs(estimate-left), abs(estimate-up), abs(estimate-diagonal))
                predictor = (left, up, diagonal)[distances.index(min(distances))]
            else:
                raise AssertionError(f"Unknown PNG filter {filter_type}")
            row[i] = (row[i] + predictor) & 255
        previous = row
    return tuple(previous[5 * channels:5 * channels + 3]), hashlib.sha256(data).hexdigest()


def create_probe(root, counter_project, profile):
    root.mkdir()
    shutil.copyfile(profile, root / "declared.streamDeckProfile")
    control = root / "commands.json"
    script = r'''import {createRequire} from 'node:module';
import fs from 'node:fs';
const WebSocket=createRequire(__REQUIRE_ROOT__)('ws');
const args={}; for(let i=2;i<process.argv.length;i+=2)args[process.argv[i]]=process.argv[i+1];
const info=JSON.parse(args['-info']), device=info.devices[0].id;
let last=0;try{last=JSON.parse(fs.readFileSync(__CONTROL__,'utf8')).seq;}catch{}
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
const report=(seq,phase)=>ws.send(JSON.stringify({event:'logMessage',payload:{message:'BUNDLE_PROBE '+JSON.stringify({seq,phase})}}));
ws.on('open',()=>{
 ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']}));
 setInterval(()=>{
  let command;try{command=JSON.parse(fs.readFileSync(__CONTROL__,'utf8'));}catch{return;}
  if(command.seq<=last)return;last=command.seq;
  const frame=command.frame;
  if(frame.context==='@registration')frame.context=args['-pluginUUID'];
  if(frame.device==='@device')frame.device=device;
  ws.send(JSON.stringify(frame));
  // Host handles this after the tested frame on the same WebSocket. Its reply
  // is an ordering barrier before Python inspects mutation/rejection results.
  ws.send(JSON.stringify({event:'getGlobalSettings',context:args['-pluginUUID'],id:'barrier-'+last}));
 },20);
});
ws.on('message',data=>{
 const m=JSON.parse(data.toString());
 if(m.event==='didReceiveGlobalSettings'&&m.id?.startsWith('barrier-'))report(Number(m.id.slice(8)),'processed');
});'''
    script = script.replace("__REQUIRE_ROOT__", json.dumps(str(counter_project / "package.json"))).replace("__CONTROL__", json.dumps(str(control)))
    (root / "plugin.js").write_text(script)
    (root / "manifest.json").write_text(json.dumps({
        "UUID": PROBE, "Name": "Bundle SDK probe", "Version": "1.0.0", "SDKVersion": 2,
        "CodePath": "plugin.js", "Nodejs": {"Version": "20"},
        "Actions": [{"UUID": PROBE + ".action", "Name": "Probe", "Controllers": ["Keypad"], "States": [{}]}],
        "Profiles": [
            {"Name": "declared", "DeviceType": 7, "Readonly": True, "AutoInstall": False, "DontAutoSwitchWhenInstalled": True},
            {"Name": "wrong-device", "DeviceType": 0, "AutoInstall": False},
        ],
    }))
    return control


class SDK:
    def __init__(self, host, control):
        self.host, self.control, self.seq = host, control, 0

    def send(self, payload, *, context="@registration", device="@device"):
        self.seq += 1
        temporary = self.control.with_suffix('.tmp')
        temporary.write_text(json.dumps({"seq": self.seq, "frame": {
            "event": "switchToProfile", "context": context, "device": device, "payload": payload,
        }}))
        temporary.replace(self.control)
        marker = 'BUNDLE_PROBE ' + json.dumps({"seq": self.seq, "phase": "processed"}, separators=(',', ':'))
        return self.host.wait(lambda s: any(marker in line for line in s["logs"]), "SDK profile frame ordering barrier")

    def rejects(self, payload, **kwargs):
        before = self.host.state()["library"]
        after = self.send(payload, **kwargs)
        require(after["library"] == before, f"Rejected SDK request changed library: {payload}, {kwargs}")


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/deckforge")
    parser.add_argument("--evidence", type=Path, default=project.parent / "evidence")
    args = parser.parse_args()
    binary, evidence = args.binary.resolve(), args.evidence.resolve()
    archive = evidence / "official-samples/lights-out/com.elgato.lightsout.streamDeckPlugin"
    require(hashlib.sha256(archive.read_bytes()).hexdigest() == ARCHIVE_HASH, "Official LightsOut source pin changed")
    with tempfile.TemporaryDirectory(prefix="deckforge-bundles-") as temporary:
        temporary = Path(temporary)
        subprocess.run([str(binary), "install", str(archive), "--destination", str(temporary / "plugins")], check=True, capture_output=True)
        lights = temporary / "plugins/com.elgato.lightsout.sdPlugin"
        probe = temporary / "com.deckforge.bundle-probe.sdPlugin"
        control = create_probe(probe, evidence / "official-samples/counter", lights / "sdp.streamDeckProfile")
        state = temporary / "state"
        host = RunningHost(binary, [lights, probe], state, temporary / "host.log")
        try:
            host.ready()
            snapshot = host.state()
            initial_ids = {p["id"] for p in snapshot["library"]["profiles"]}
            require(len(initial_ids) == 2, "Only default library + matching Plus LightsOut bundle should auto-install")
            game_id = snapshot["library"]["active_profile"]
            require(current(snapshot)["name"] == "Lights Out" and not current(snapshot)["readonly"], "Official matching bundle did not activate with its manifest permissions")
            before = keys(snapshot)
            require(len(before) == 8 and sum(s["uuid"] == PIECE for s in before.values()) == 7, "Official Plus profile should contain reset + seven game pieces")
            require(all(s["state"] == 0 for s in before.values()), "Fresh official fixture has unexpected states")
            initial_png = {}
            for index, item in before.items():
                pixel, digest = png_corner(item["preview"])
                initial_png[index] = digest
                if item["uuid"] == PIECE:
                    require(pixel[0] > 100 and pixel[1] > 100 and pixel[2] < pixel[0] - 30,
                            f"Initial key {index} failed actual ON SVG yellow-background check: {pixel}")
            for event in ("keyDown", "keyUp"):
                host.post({"command": "input", "event": event, "index": 5})
            affected = {1, 4, 5, 6}
            snapshot = host.wait(lambda s: {i for i,v in keys(s).items() if v["state"] == 1} == affected,
                                 "unchanged LightsOut neighboring state updates")
            for index, item in keys(snapshot).items():
                pixel, digest = png_corner(item["preview"])
                if index in affected:
                    require(digest != initial_png[index], f"State changed but actual PNG did not: {index}")
                    require(pixel[0] < 50 and pixel[1] < 50, f"OFF SVG background did not render: {index}: {pixel}")
                else:
                    require(digest == initial_png[index], f"Unrelated key image changed: {index}")
            host.post({"command": "profile_create", "name": "Return target"})
            return_id = host.state()["library"]["active_profile"]
            host.post({"command": "page_create", "name": "Return page two"})
            return_page = host.state()["library"]["active_page"]
            sdk = SDK(host, control)
            sdk.rejects({"profile": "sdp"})  # Declared only by another plugin.
            sdk.rejects({"profile": "declared"}, device="unmanaged-device")
            sdk.rejects({"profile": "declared"}, context=before[1]["context"])
            sdk.rejects({"profile": "wrong-device"})
            sdk.rejects({"profile": "declared", "page": 999})
            snapshot = sdk.send({"profile": "declared", "page": 0})
            declared_id = snapshot["library"]["active_profile"]
            require(declared_id != return_id and current(snapshot)["readonly"], "Declared lazy bundle did not switch/read-only")
            require(len(snapshot["slots"]) == 8, "Lazy declared profile lost official action tree")
            host.post({"command": "page_create", "name": "Must fail"}, expected=400)
            host.post({"command": "assign", "controller": "Keypad", "index": 0, "uuid": PROBE + ".action"}, expected=400)
            snapshot = sdk.send({})
            require(snapshot["library"]["active_profile"] == return_id and snapshot["library"]["active_page"] == return_page,
                    "Omitted profile failed to restore previous profile/cursor")
            sdk.rejects({"profile": "declared", "page": -1})
            sdk.rejects({"profile": "undeclared"})
            sdk.send({"profile": "declared"})
            sdk.send({})
            host.post({"command": "profile_select", "id": game_id})
            require({i for i,v in keys(host.state()).items() if v["state"] == 1} == affected, "Switching bundles lost original game state")
            ids = {p["id"] for p in host.state()["library"]["profiles"]}
            require(len(ids) == 4, "Expected exactly default, auto bundle, return target and one lazy bundle")
        finally:
            host.stop()
        saved = json.loads((state / "session.json").read_text())
        require(saved["library"]["bundles"][LIGHTS] == {"sdp": game_id}, "Wrong model bundles were installed or binding lost")
        require(saved["library"]["bundles"][PROBE]["declared"] == declared_id, "Lazy bundle binding not saved")
        restart = RunningHost(binary, [lights, probe], state, temporary / "restart.log")
        try:
            restart.ready()
            snapshot = restart.state()
            require({p["id"] for p in snapshot["library"]["profiles"]} == ids, "Restart auto-imported duplicate bundles")
            require(snapshot["library"]["active_profile"] == game_id, "Restart stole selected profile")
            require({i for i,v in keys(snapshot).items() if v["state"] == 1} == affected, "Restart lost game state")
            for index, item in keys(snapshot).items():
                pixel, digest = png_corner(item["preview"])
                if index in affected:
                    require(pixel[0] < 50 and pixel[1] < 50 and digest != initial_png[index], "Restart lost actual OFF image")
        finally:
            restart.stop()
    print("PASS: unchanged official LightsOut auto-imported Plus profile, ran seven-piece adjacency logic and changed actual SVG-backed PNG pixels; no other device bundles imported.")
    print("PASS: registered SDK caller lazy install, read-only enforcement, switch/return/cursor, rejected undeclared/cross-plugin/cross-device/wrong-context/page requests, persisted bundle bindings without restart duplication.")
    print("Simulation only; no USB opened. No Windows export/import or physical rendering claim.")


if __name__ == "__main__":
    main()
