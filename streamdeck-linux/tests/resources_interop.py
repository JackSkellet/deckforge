#!/usr/bin/env python3
"""Resources API conformance using real SDK plugins and native WebKit inspectors.

Generated fixtures import the unchanged @elgato/streamdeck 2.0.4 installation
from the pinned official Counter sample. No USB, keyboard injection, audio,
external network request, or desktop input is used. Run in a graphical session
or under Xvfb. This proves our resource wire/ZIP roundtrip, not Windows acceptance.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid
import zipfile

from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT, require
from system_interop import Host

PRIMARY = "com.deckforge.resources-probe"
FOREIGN = "com.deckforge.resources-foreign"
TRIGGER = "com.elgato.streamdeck.keys.adaptor"
ROUTINE = "com.elgato.streamdeck.multiactions.routine"
OP = "resourceProbe"

PLUGIN_JS = r'''import streamDeck from __SDK_ENTRY__;
import {connection} from __SDK_CONNECTION__;
import {appendFileSync} from 'node:fs';
const proof=__PROOF__, seen=new Set();let serial=0,queue=Promise.resolve();
const record=(kind,fields={})=>appendFileSync(proof,JSON.stringify({seq:++serial,kind,...fields})+'\n');
streamDeck.actions.onWillAppear(ev=>record('appear',{context:ev.action.id,role:ev.payload.settings?.role}));
connection.on('didReceiveResources',m=>record('plugin_resource',{envelope:m}));
connection.on('sendToPlugin',m=>{
 if(m.payload?.resourceReport){const {kind,...fields}=m.payload.resourceReport;record(kind,{context:m.context,...fields});}
});
async function execute(m){
 const request=m.payload?.settings?.resourceProbe;
 if(!request?.nonce||seen.has(request.nonce))return;seen.add(request.nonce);
 const action=streamDeck.actions.getActionById(m.context);
 if(!action)throw Error('SDK action store missing '+m.context);
 const target=request.target||action.id;
 if(request.op==='sdk_set_get'){
   await action.setResources(request.resources);
   record('result',{nonce:request.nonce,context:action.id,resources:await action.getResources(),sdk:'2.0.4'});
 }else if(request.op==='get'){
   record('result',{nonce:request.nonce,context:action.id,resources:await action.getResources(),sdk:'2.0.4'});
 }else if(request.op==='raw_set_get'){
   await connection.send({event:'setResources',context:target,payload:request.resources});
   await connection.send({event:'getResources',context:action.id,id:request.nonce});
 }else if(request.op==='raw_get'){
   await connection.send({event:'getResources',context:target,id:request.nonce});
   record('barrier',{nonce:request.nonce,context:action.id,resources:await action.getResources()});
 }else if(request.op==='pi'){
   await connection.send({event:'sendToPropertyInspector',context:action.id,payload:{resourceCommand:request}});
 }else throw Error('Unknown test operation '+request.op);
}
connection.on('didReceiveSettings',m=>{
 queue=queue.then(()=>execute(m)).catch(error=>record('failure',{message:String(error),stack:error.stack}));
});
process.on('unhandledRejection',error=>record('failure',{message:String(error)}));
await streamDeck.connect();record('connected',{sdk:'2.0.4'});
'''

INSPECTOR_HTML = r'''<!doctype html><html><head><meta charset="utf-8"></head>
<body>Resources conformance inspector<script>
window.connectElgatoStreamDeckSocket=(port,registration,event,info,actionInfo)=>{
 const action=JSON.parse(actionInfo),ws=new WebSocket(`ws://127.0.0.1:${port}`);
 const send=(event,payload,extra={})=>ws.send(JSON.stringify({event,context:registration,action:action.action,payload,...extra}));
 const report=value=>send('sendToPlugin',{resourceReport:value});
 ws.onopen=()=>{ws.send(JSON.stringify({event,uuid:registration}));send('getResources',undefined,{id:'pi-initial'});};
 ws.onmessage=({data})=>{
   const m=JSON.parse(data);
   if(m.event==='didReceiveResources'){
     report({kind:'pi_resource',envelope:m});
     if(m.id==='pi-initial')report({kind:'pi_ready',distinctIdentity:registration!==action.context});
   }
   if(m.event==='sendToPropertyInspector'&&m.payload?.resourceCommand){
     const command=m.payload.resourceCommand,target=command.target||registration;
     if(command.pi_op==='set')send('setResources',command.resources,{context:target});
     if(command.pi_op==='foreign_get')send('getResources',undefined,{context:target,id:command.nonce+'-denied'});
     // Same-socket barrier: preceding notifications must arrive before this reply.
     send('getResources',undefined,{id:command.nonce});
   }
 };
};
</script></body></html>'''


def hash_files(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob("*") if p.is_file()}


def make_plugin(root, plugin, sdk):
    root.mkdir()
    proof = root / "proof.jsonl"
    (root / "plugin.mjs").write_text(PLUGIN_JS
        .replace("__SDK_ENTRY__", json.dumps((sdk / "dist/plugin/index.js").as_uri()))
        .replace("__SDK_CONNECTION__", json.dumps((sdk / "dist/plugin/connection.js").as_uri()))
        .replace("__PROOF__", json.dumps(str(proof))))
    (root / "inspector.html").write_text(INSPECTOR_HTML)
    (root / "manifest.json").write_text(json.dumps({
        "UUID": plugin, "Name": "Resource conformance probe", "Version": "1.0.0",
        "SDKVersion": 2, "Software": {"MinimumVersion": "7.1"},
        "CodePath": "plugin.mjs", "Nodejs": {"Version": "20"},
        "Actions": [{"UUID": plugin + ".action", "Name": "Resources probe",
                     "Controllers": ["Keypad"], "States": [{}],
                     "PropertyInspectorPath": "inspector.html", "SupportedInMultiActions": True}],
    }))
    return root, proof


def rows(path):
    if not path.exists():
        return []
    # Append writes are line-delimited; ignore only a not-yet-complete final line.
    return [json.loads(line) for line in path.read_text().splitlines(keepends=True)
            if line.endswith("\n")]


def wait(host, proofs, predicate, description, timeout=25):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        host.check_process()
        failures = [row for proof in proofs for row in rows(proof) if row["kind"] == "failure"]
        require(not failures, "Generated fixture failed: " + repr(failures))
        result = predicate()
        if result:
            return result
        time.sleep(.04)
    raise AssertionError(f"Timed out waiting for {description}\n{host.log.read_text()[-6000:]}")


def all_slots(snapshot):
    def descend(value):
        if isinstance(value, list):
            for item in value:
                yield from descend(item)
        elif isinstance(value, dict):
            if value.get("context"):
                yield value
            for field in ("trigger_children", "multi_children", "stack_children"):
                yield from descend(value.get(field))
    # Compatibility snapshots expose the same descendants under typed fields
    # and multi_children. Each action context represents one resource owner.
    unique = {}
    for action in descend(snapshot.get("slots", [])):
        unique.setdefault(action["context"], action)
    return list(unique.values())


def by_context(host, context):
    return next(s for s in all_slots(host.state()) if s["context"] == context)


def by_role(host, role):
    return next(s for s in all_slots(host.state()) if s.get("settings", {}).get("role") == role)


def request(host, context, op, **extra):
    nonce = uuid.uuid4().hex
    settings = copy.deepcopy(by_context(host, context).get("settings", {}))
    settings[OP] = {"nonce": nonce, "op": op, **extra}
    host.post({"command": "settings", "context": context, "settings": settings})
    return nonce


def sdk(host, proof, context, resources=None, *, get=False):
    start = len(rows(proof))
    nonce = request(host, context, "get" if get else "sdk_set_get", **({} if get else {"resources": resources}))
    result = wait(host, [proof], lambda: next((r for r in rows(proof)
                  if r["kind"] == "result" and r.get("nonce") == nonce), None), "official SDK resource result")
    require(result["sdk"] == "2.0.4" and result["context"] == context, "SDK result lost action ownership")
    replies = [row["envelope"] for row in rows(proof)[start:]
               if row["kind"] == "plugin_resource" and row["envelope"].get("context") == context
               and "id" in row["envelope"]]
    require(replies, "SDK resource result had no observed correlated wire reply")
    envelope_check(host, context, replies[-1], result["resources"], replies[-1]["id"])
    return result["resources"]


def envelope_check(host, context, envelope, expected, request_id=None, *, unsolicited=False):
    action = by_context(host, context)
    require(envelope["event"] == "didReceiveResources", "Wrong resources event")
    require(envelope["context"] == context and envelope["action"] == action["uuid"], "Wrong action/context envelope")
    require(envelope["device"] == host.state()["device"], "Resources reply has wrong device")
    payload = envelope.get("payload", {})
    require(payload.get("resources") == expected, "Resources map mismatch")
    require(payload.get("controller") == action["controller"], "Resources event omitted its controller")
    require(isinstance(payload.get("settings"), dict)
            and payload["settings"].get("role") == action.get("settings", {}).get("role"),
            "Resources event omitted or misrouted its action settings")
    in_multi = action.get("settings", {}).get("role") in ("nested-own", "nested-foreign")
    require(payload.get("isInMultiAction") is in_multi, "Resources event has wrong Multi Action membership")
    if in_multi:
        require("coordinates" not in payload and "userDesiredState" not in payload,
                "Multi resource reply included key-event-only coordinates/desired state")
    else:
        coordinates = payload.get("coordinates", {})
        require(isinstance(coordinates.get("column"), int) and isinstance(coordinates.get("row"), int),
                "Normal resource reply omitted action coordinates")
    if unsolicited:
        require("id" not in envelope, "Unsolicited resources update gained a request ID")
    else:
        require(envelope.get("id") == request_id, "Resources request ID was not preserved")


def raw_set(host, proof, context, resources, target=None):
    nonce = request(host, context, "raw_set_get", resources=resources, **({"target": target} if target else {}))
    result = wait(host, [proof], lambda: next((r["envelope"] for r in rows(proof)
                  if r["kind"] == "plugin_resource" and r["envelope"].get("id") == nonce), None), "raw resource request correlation")
    return nonce, result


def pi_request(host, proof, context, operation="get", **extra):
    nonce = request(host, context, "pi", pi_op=operation, **extra)
    result = wait(host, [proof], lambda: next((r["envelope"] for r in rows(proof)
                  if r["kind"] == "pi_resource" and r.get("context") == context
                  and r["envelope"].get("id") == nonce), None), "native inspector resources reply")
    return nonce, result


def resources_since(proof, offset, side, context):
    return [r["envelope"] for r in rows(proof)[offset:] if r["kind"] == side
            and r["envelope"].get("context") == context and "id" not in r["envelope"]]


def assign(host, plugin, index, role):
    host.post({"command": "assign", "controller": "Keypad", "index": index, "uuid": plugin + ".action"})
    action = next(s for s in host.state()["slots"] if s["controller"] == "Keypad" and s["index"] == index)
    host.post({"command": "settings", "context": action["context"], "settings": {"role": role}})
    return action["context"]


def child(plugin, role, **extra):
    return {"ActionID": str(uuid.uuid4()), "UUID": plugin + ".action", "Name": role,
            "Settings": {"role": role}, "State": 0, "States": [{}], "FutureField": {"keep": role}, **extra}


def exported_records(path):
    with zipfile.ZipFile(path) as archive:
        def walk(value):
            if isinstance(value, dict):
                if value.get("UUID") in (PRIMARY + ".action", FOREIGN + ".action"):
                    yield value
                for key in ("Controllers", "Actions"):
                    nested = value.get(key)
                    if isinstance(nested, dict):
                        for item in nested.values():
                            yield from walk(item)
                    else:
                        yield from walk(nested)
            elif isinstance(value, list):
                for item in value:
                    yield from walk(item)
        return [action for name in archive.namelist() if name.endswith("manifest.json")
                for action in walk(json.loads(archive.read(name)))]


def check_archive_resources(path, expected):
    with zipfile.ZipFile(path) as archive:
        registry = json.loads(archive.read("Resources/manifest.json"))["resources"]
        records = {r["Settings"]["role"]: r for r in exported_records(path)}
        for role, resources in expected.items():
            record = records[role]
            require(set(record.get("Resources", {})) == set(resources), f"Wrong resource keys in {role} export")
            for key, (basename, data) in resources.items():
                identifier = record["Resources"][key]
                relative = registry[identifier]["file"]
                member = relative if relative.startswith("Resources/") else "Resources/" + relative
                require(Path(member).name == basename, f"Resource basename lost for {role}/{key}")
                require(archive.read(member) == data, f"Resource bytes changed for {role}/{key}")


def verify_materialized(host, proofs, expected, originals):
    maps = {}
    for role, resources in expected.items():
        action = by_role(host, role)
        proof = proofs[FOREIGN if action["uuid"].startswith(FOREIGN) else PRIMARY]
        actual = sdk(host, proof, action["context"], get=True)
        require(set(actual) == set(resources), f"Restored resource keys differ for {role}")
        for key, (basename, data) in resources.items():
            path = Path(actual[key])
            require(path.is_absolute() and path.is_file(), f"Resource is not a readable absolute path: {path}")
            require(str(path) not in originals, "Import kept the original external resource path")
            require(path.name == basename and path.read_bytes() == data, f"Restored asset changed: {role}/{key}")
        maps[role] = actual
    return maps


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/deckforge")
    parser.add_argument("--evidence", type=Path, default=project.parent / "evidence")
    args = parser.parse_args()
    require(os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"), "Run in a graphical session or under Xvfb for the native inspector checks")
    evidence = args.evidence.resolve()
    counter = evidence / "official-samples/counter"
    sdk_root = counter / "node_modules/@elgato/streamdeck"
    require(json.loads((sdk_root / "package.json").read_text())["version"] == "2.0.4", "Expected unchanged official SDK 2.0.4")
    require(hashlib.sha256((counter / "package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256, "Official sample lockfile pin changed")
    sample_root = counter.parent
    if (sample_root / ".git").exists():
        revision = subprocess.check_output(["git", "-C", str(sample_root), "rev-parse", "HEAD"], text=True).strip()
        require(revision == SAMPLE_COMMIT, "Official sample source revision changed")
    before_sdk = hash_files(sdk_root)
    with tempfile.TemporaryDirectory(prefix="deckforge-resources-") as temporary:
        root = Path(temporary)
        plugins, proofs = {}, {}
        for plugin in (PRIMARY, FOREIGN):
            plugins[plugin], proofs[plugin] = make_plugin(root / (plugin + ".sdPlugin"), plugin, sdk_root)
        # Same basename, different bytes: global export identifiers must not collide.
        source_a, source_b = root / "original-a", root / "original-b"
        source_a.mkdir(); source_b.mkdir()
        a, b = source_a / "sample å notes.dat", source_b / "sample å notes.dat"
        bytes_a, bytes_b = b"A\x00resource\xff\n", b"B\x00different resource\xfe\n"
        a.write_bytes(bytes_a); b.write_bytes(bytes_b)
        a, b = a.resolve(), b.resolve()
        state = root / "state"
        host = Host(args.binary.resolve(), list(plugins.values()), state, root / "host.log")
        expected = {}
        try:
            host.ready()
            host.wait(lambda s: len(s["plugins"]) == 2 and all(p["connected"] for p in s["plugins"]), "two SDK fixtures to register")
            host.post({"command": "profile_create", "name": "Resource conformance"})
            primary = assign(host, PRIMARY, 0, "direct")
            foreign = assign(host, FOREIGN, 1, "foreign")
            sibling = assign(host, PRIMARY, 2, "sibling")
            wait(host, proofs.values(), lambda: all(any(r["kind"] == "appear" and r.get("context") == c for r in rows(proofs[p]))
                 for p, c in [(PRIMARY, primary), (FOREIGN, foreign), (PRIMARY, sibling)]), "SDK action-store registration")
            require(sdk(host, proofs[PRIMARY], primary, get=True) == {}, "New action resources were not empty")
            require(sdk(host, proofs[PRIMARY], primary, {"first": str(a), "drop": str(b)}) == {"first": str(a), "drop": str(b)}, "SDK set/get failed")
            require(sdk(host, proofs[PRIMARY], primary, {"second": str(b), "empty": ""}) == {"second": str(b)}, "setResources did not replace or omit empty values")
            require(sdk(host, proofs[PRIMARY], primary, {}) == {}, "Empty replacement did not clear resources")
            require(sdk(host, proofs[PRIMARY], primary, {"file": str(a)}) == {"file": str(a)}, "Resource setup failed")
            nonce, message = raw_set(host, proofs[PRIMARY], primary, {"valid": str(b), "zz_relative": "relative/file.dat"})
            envelope_check(host, primary, message, {"file": str(a)}, nonce)
            nonce, message = raw_set(host, proofs[PRIMARY], primary, {"bad": 17})
            envelope_check(host, primary, message, {"file": str(a)}, nonce)
            require(sdk(host, proofs[FOREIGN], foreign, {"protected": str(b)}) == {"protected": str(b)}, "Foreign owner setup failed")
            require(sdk(host, proofs[PRIMARY], sibling, {"sibling": str(b)}) == {"sibling": str(b)}, "Sibling setup failed")
            nonce, message = raw_set(host, proofs[PRIMARY], primary, {"stolen": str(a)}, target=foreign)
            envelope_check(host, primary, message, {"file": str(a)}, nonce)
            require(sdk(host, proofs[FOREIGN], foreign, get=True) == {"protected": str(b)}, "Plugin overwrote another owner's resources")
            start = len(rows(proofs[PRIMARY]))
            nonce = request(host, primary, "raw_get", target=foreign)
            wait(host, proofs.values(), lambda: any(r["kind"] == "barrier" and r.get("nonce") == nonce for r in rows(proofs[PRIMARY])), "foreign get rejection barrier")
            require(not any(r["kind"] == "plugin_resource" and r["envelope"].get("id") == nonce for r in rows(proofs[PRIMARY])[start:]), "Foreign resources leaked to another plugin")
            # Native inspector identity/correlation, both notification directions, and no self-echo.
            for context in (primary, sibling):
                host.post({"command": "open_inspector", "context": context})
                ready = wait(host, proofs.values(), lambda: next((r for r in rows(proofs[PRIMARY])
                             if r["kind"] == "pi_ready" and r.get("context") == context), None), "native resources inspector")
                require(ready["distinctIdentity"], "Inspector reused public action context for registration")
            # A correlated get response must reach only its requesting socket.
            start = len(rows(proofs[PRIMARY]))
            get_nonce = request(host, primary, "raw_get")
            wait(host, proofs.values(), lambda: any(r["kind"] == "barrier" and r.get("nonce") == get_nonce for r in rows(proofs[PRIMARY])), "plugin get requester-only barrier")
            pi_request(host, proofs[PRIMARY], primary)
            pi_request(host, proofs[PRIMARY], sibling)
            require(not any(r["kind"] == "pi_resource" and r["envelope"].get("id") == get_nonce for r in rows(proofs[PRIMARY])[start:]), "Plugin getResources reply was broadcast to a PI")
            start = len(rows(proofs[PRIMARY]))
            pi_nonce, message = pi_request(host, proofs[PRIMARY], primary)
            envelope_check(host, primary, message, {"file": str(a)}, pi_nonce)
            sdk(host, proofs[PRIMARY], primary, get=True)
            pi_request(host, proofs[PRIMARY], sibling)
            require(not any(r["kind"] == "plugin_resource" and r["envelope"].get("id") == pi_nonce for r in rows(proofs[PRIMARY])[start:]), "PI getResources reply was broadcast to its plugin")
            require(not any(r["kind"] == "pi_resource" and r.get("context") == sibling and r["envelope"].get("id") == pi_nonce for r in rows(proofs[PRIMARY])[start:]), "PI getResources reply crossed inspector scope")
            start = len(rows(proofs[PRIMARY]))
            updated = {"plugin": str(a)}
            require(sdk(host, proofs[PRIMARY], primary, updated) == updated, "Plugin resource update failed")
            wait(host, proofs.values(), lambda: resources_since(proofs[PRIMARY], start, "pi_resource", primary), "plugin-origin PI notification")
            pi_request(host, proofs[PRIMARY], sibling)  # Other-PI ordering barrier.
            pi_request(host, proofs[PRIMARY], primary)
            require(not resources_since(proofs[PRIMARY], start, "plugin_resource", primary), "Plugin setResources self-echoed")
            require(not resources_since(proofs[PRIMARY], start, "pi_resource", sibling), "Plugin resource update crossed PI action scope")
            for message in resources_since(proofs[PRIMARY], start, "pi_resource", primary):
                envelope_check(host, primary, message, updated, unsolicited=True)
            start = len(rows(proofs[PRIMARY]))
            pi_map = {"fromPI": str(b)}
            nonce, message = pi_request(host, proofs[PRIMARY], primary, "set", resources={**pi_map, "empty": ""})
            envelope_check(host, primary, message, pi_map, nonce)
            notifications = wait(host, proofs.values(), lambda: resources_since(proofs[PRIMARY], start, "plugin_resource", primary), "PI-origin plugin notification")
            pi_request(host, proofs[PRIMARY], sibling)
            require(not resources_since(proofs[PRIMARY], start, "pi_resource", primary), "PI setResources self-echoed")
            require(not resources_since(proofs[PRIMARY], start, "pi_resource", sibling), "PI setResources notified another PI")
            for message in notifications:
                envelope_check(host, primary, message, pi_map, unsolicited=True)
            # PI identity is bound even when the target has the same plugin UUID.
            nonce, message = pi_request(host, proofs[PRIMARY], primary, "set", target=sibling, resources={"overwrite": str(a)})
            envelope_check(host, primary, message, pi_map, nonce)
            require(sdk(host, proofs[PRIMARY], sibling, get=True) == {"sibling": str(b)}, "PI overwrote a different action context")
            start = len(rows(proofs[PRIMARY]))
            nonce, _ = pi_request(host, proofs[PRIMARY], primary, "foreign_get", target=foreign)
            require(not any(r["kind"] == "pi_resource" and r["envelope"].get("id") == nonce + "-denied" for r in rows(proofs[PRIMARY])[start:]), "PI read foreign resources")
            # Missing absolute files are accepted by set; export must fail atomically.
            missing = root / "not-created.dat"
            require(sdk(host, proofs[PRIMARY], primary, {"missing": str(missing)}) == {"missing": str(missing)}, "setResources incorrectly required file existence")
            bad_export = root / "existing-output.streamDeckProfile"
            bad_export.write_bytes(b"existing output must remain intact")
            host.post({"command": "export_profile", "path": str(bad_export)}, expected=400)
            require(bad_export.read_bytes() == b"existing output must remain intact", "Failed resource export damaged existing output")
            for context in (primary, sibling):
                host.post({"command": "close_inspector", "context": context})
            # Resources on a direct Trigger child and a Multi child inside a Trigger.
            host.post({"command": "assign", "controller": "Encoder", "index": 0, "uuid": TRIGGER})
            trigger = next(s for s in host.state()["slots"] if s["uuid"] == TRIGGER)
            sequence = {"UUID": ROUTINE, "ActionID": str(uuid.uuid4()), "Name": "Resource sequence", "Settings": {}, "State": 0, "States": [{}],
                        "Actions": [{"Actions": [child(PRIMARY, "nested-own"), child(FOREIGN, "nested-foreign")]}, {"Actions": []}]}
            raw = {"Settings": {}, "FutureContainer": 73,
                   "Actions": [child(PRIMARY, "trigger-left"), sequence, {"Actions": []}]}
            host.post({"command": "trigger_save", "context": trigger["context"], "raw": raw})
            assignments = {"direct": {"main": a, "second": b}, "foreign": {"protected": b}, "sibling": {"sibling": b},
                           "trigger-left": {"trigger": a}, "nested-own": {"nested": b}, "nested-foreign": {"foreignNested": a}}
            wait(host, proofs.values(), lambda: all(any(r["kind"] == "appear" and r.get("role") == role
                 for r in rows(proofs[FOREIGN if role == "nested-foreign" else PRIMARY]))
                 for role in ("trigger-left", "nested-own", "nested-foreign")), "nested SDK actions to appear")
            for role, resources in assignments.items():
                action = by_role(host, role)
                proof = proofs[FOREIGN if action["uuid"].startswith(FOREIGN) else PRIMARY]
                paths = {key: str(path) for key, path in resources.items()}
                require(sdk(host, proof, action["context"], paths) == paths, f"Nested/normal resource assignment failed for {role}")
                expected[role] = {key: (path.name, path.read_bytes()) for key, path in resources.items()}
            exported = root / "resources.streamDeckProfile"
            host.post({"command": "export_profile", "path": str(exported)})
            check_archive_resources(exported, expected)
            # Original resources disappear; imported files must use managed archive copies.
            a.unlink(); b.unlink()
            host.post({"command": "import_profile", "path": str(exported)})
            imported_id = host.state()["library"]["active_profile"]
            wait(host, proofs.values(), lambda: len([s for s in all_slots(host.state()) if s.get("settings", {}).get("role") in expected]) == len(expected), "all imported resource owners")
            verify_materialized(host, proofs, expected, {str(a), str(b)})
            # Imported IDs remain in raw records. A runtime replacement or an
            # explicit empty map must survive duplication without rehydrating
            # stale raw Resources references into the new child contexts.
            cleared = by_role(host, "nested-own")
            require(sdk(host, proofs[PRIMARY], cleared["context"], {}) == {}, "Nested resource clear failed")
            replacement = root / "replacement ü.bin"
            replacement_bytes = b"replacement resource after import\x00\xfd"
            replacement.write_bytes(replacement_bytes)
            replaced = by_role(host, "nested-foreign")
            replacement_map = {"replacement": str(replacement)}
            require(sdk(host, proofs[FOREIGN], replaced["context"], replacement_map) == replacement_map, "Nested resource replacement failed")
            expected["nested-own"] = {}
            expected["nested-foreign"] = {"replacement": (replacement.name, replacement_bytes)}
            host.post({"command": "profile_duplicate", "id": imported_id, "name": "Duplicated resource owners"})
            imported_id = host.state()["library"]["active_profile"]
            verify_materialized(host, proofs, expected, {str(a), str(b)})
            page = host.state()["library"]["active_page"]
            host.post({"command": "page_duplicate", "id": page, "name": "Duplicated resource page"})
            verify_materialized(host, proofs, expected, {str(a), str(b)})
            reexported = root / "resources-again.streamDeckProfile"
            host.post({"command": "export_profile", "path": str(reexported)})
            check_archive_resources(reexported, expected)
            replacement.unlink()
            host.post({"command": "import_profile", "path": str(reexported)})
            imported_id = host.state()["library"]["active_profile"]
            verify_materialized(host, proofs, expected, {str(a), str(b), str(replacement)})
        finally:
            host.stop()
        restarted = Host(args.binary.resolve(), [], state, root / "restart.log")
        try:
            restarted.ready()
            restarted.wait(lambda s: all(p["connected"] for p in s["plugins"]), "restart plugin connections")
            require(restarted.state()["library"]["active_profile"] == imported_id, "Restart selected the wrong resource profile")
            verify_materialized(restarted, proofs, expected, {str(a), str(b)})
        finally:
            restarted.stop()
    require(hash_files(sdk_root) == before_sdk, "Official SDK files were modified by the fixture")
    print("PASS: unchanged SDK2.0.4 methods, correlated resources envelopes, native PI routing, owner isolation, full replacement/clear, atomic validation/export, preserved basenames+bytes, nested Trigger/Multi, duplicated overrides/clears, import and restart. No USB or OS actions executed.")


if __name__ == "__main__":
    main()
