#!/usr/bin/env python3
"""Test whole-profile runtime behavior in simulation using official fixtures.

This proves our host's import/navigation/export/reimport behavior. It does not
prove that the official Windows application accepts our export, nor GUI clicks
or physical Stream Deck input. Original Elgato files remain unchanged.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import time
import zipfile

from sample_interop import COUNTER, RunningHost, require, slot

PROBE_PLUGIN = "com.deckforge.profile-probe"
PROBE_ACTION = PROBE_PLUGIN + ".action"
WINDOWS_FILE = "fls1.KgkvZ_osjTGmFQpIDeeaNhGPU"
WINDOWS_SHA256 = "05ed051d2c0a84d2b989748ce02dc7e2b0db4dd0de9dd227c5ea4b62b37468ee"
OPEN = "com.elgato.streamdeck.profile.openchild"
BACK = "com.elgato.streamdeck.profile.backtoparent"


def key(host, index):
    for event in ("keyDown", "keyUp"):
        host.post({"command": "input", "event": event, "index": index})


def at(snapshot, index, controller="Keypad"):
    return next((s for s in snapshot["slots"] if s["controller"] == controller and s["index"] == index), {})


def current_profile(snapshot):
    library = snapshot["library"]
    return next(p for p in library["profiles"] if p["id"] == library["active_profile"])


def events(snapshot, event=None, context=None):
    found = []
    for line in snapshot.get("logs", []):
        if "PROFILE_PROBE " not in line:
            continue
        item = json.loads(line.split("PROFILE_PROBE ", 1)[1])
        if (event is None or item["event"] == event) and (context is None or item["context"] == context):
            found.append(item)
    return found


def wait(host, predicate, description):
    return host.wait(predicate, description)


def select_page(host, page):
    host.post({"command": "page_select", "id": page})
    return wait(host, lambda s: s["library"]["active_page"] == page, "selected page")


def create_probe(path, counter):
    path.mkdir()
    # Reuse only the generic ws dependency installed for official sample builds.
    script = r'''import {createRequire} from 'node:module';
const WebSocket = createRequire(__REQUIRE_ROOT__)('ws');
const args={}; for(let i=2;i<process.argv.length;i+=2) args[process.argv[i]]=process.argv[i+1];
const ws=new WebSocket(`ws://127.0.0.1:${args['-port']}`);
const send=(event,context,payload)=>ws.send(JSON.stringify({event,context,payload}));
ws.on('open',()=>ws.send(JSON.stringify({event:args['-registerEvent'],uuid:args['-pluginUUID']})));
ws.on('message',data=>{
 const m=JSON.parse(data.toString());
 if(['willAppear','willDisappear','didReceiveSettings'].includes(m.event))
   send('logMessage',args['-pluginUUID'],{message:'PROFILE_PROBE '+JSON.stringify({event:m.event,context:m.context,settings:m.payload.settings})});
 if(m.event==='willAppear') send('setTitle',m.context,{title:'Lifecycle probe'});
 if(m.event==='keyDown') setTimeout(()=>send('setSettings',m.context,{...m.payload.settings,hiddenReply:73}),150);
});'''
    (path / "plugin.js").write_text(script.replace("__REQUIRE_ROOT__", json.dumps(str(counter.parent / "package.json"))))
    (path / "manifest.json").write_text(json.dumps({
        "UUID": PROBE_PLUGIN, "Name": "Profile lifecycle probe", "Version": "1.0.0", "SDKVersion": 2,
        "CodePath": "plugin.js", "Nodejs": {"Version": "20"},
        "Actions": [{"UUID": PROBE_ACTION, "Name": "Lifecycle probe", "Controllers": ["Keypad"], "States": [{}]}],
    }))
    return path


def archive_entries(path):
    with zipfile.ZipFile(path) as archive:
        names = [n for n in archive.namelist() if not n.endswith('/')]
        roots = {n.split('/', 1)[0] for n in names}
        require(len(roots) == 1, "Export should contain one umbrella profile")
        root = next(iter(roots)) + '/'
        return {n[len(root):]: archive.read(n) for n in names}


def contains_original(expected, actual, location):
    """Allow added fields but require every original field/value to survive."""
    if isinstance(expected, dict):
        require(isinstance(actual, dict), f"Object lost at {location}")
        for key, value in expected.items():
            require(key in actual, f"Original field lost: {location}/{key}")
            contains_original(value, actual[key], location + '/' + key)
    elif isinstance(expected, list):
        require(isinstance(actual, list) and len(actual) >= len(expected), f"Array lost at {location}")
        for index, value in enumerate(expected):
            contains_original(value, actual[index], f"{location}/{index}")
    else:
        require(expected == actual, f"Original value changed: {location}: {expected!r} != {actual!r}")


def check_windows_tree(host, fixture, state, output):
    host.post({"command": "import_profile", "path": str(fixture)})
    snapshot = host.state()
    imported = current_profile(snapshot)
    profile_id = imported["id"]
    require(len(imported["page_order"]) == 2, "Folder/default pages leaked into visible page order")
    require(len(imported["pages"]) == 7, "Snapshot should show 2 visible pages and 5 folders, hiding the pin page")
    saved = json.loads((state / "session.json").read_text())
    stored = next(p for p in saved["library"]["profiles"] if p["id"] == profile_id)
    require(len(stored["pages"]) == 8, "Whole original eight-page tree was not retained")
    require(len(snapshot["slots"]) == 10, "Original Plus current page should have 6 keys and 4 dials")
    root_page = snapshot["library"]["active_page"]
    apps = at(snapshot, 0)
    require(apps["uuid"] == OPEN, "Original Apps folder missing")
    apps_page = apps["settings"]["ProfileUUID"]
    key(host, 0)
    snapshot = wait(host, lambda s: s["library"]["active_page"] == apps_page, "original Apps folder press")
    require(snapshot["library"]["folder_depth"] == 1 and at(snapshot, 0)["uuid"] == BACK, "Original folder/back stack mismatch")
    key(host, 0)
    wait(host, lambda s: s["library"]["active_page"] == root_page, "original parent-folder action")
    key(host, 3)
    snapshot = wait(host, lambda s: s["library"]["folder_depth"] == 1, "original Tutorials folder")
    require(at(snapshot, 7)["uuid"] == OPEN, "Nested Advanced folder missing")
    key(host, 7)
    wait(host, lambda s: s["library"]["folder_depth"] == 2, "original nested folder")
    key(host, 0)
    wait(host, lambda s: s["library"]["folder_depth"] == 1, "nested back preserves ancestor")
    key(host, 0)
    wait(host, lambda s: s["library"]["active_page"] == root_page, "root restored")
    host.post({"command": "page_step", "direction": 1})
    wait(host, lambda s: s["library"]["active_page"] == imported["page_order"][1], "original next visible page")
    host.post({"command": "page_step", "direction": -1})
    wait(host, lambda s: s["library"]["active_page"] == root_page, "original previous visible page")
    host.post({"command": "export_profile", "path": str(output)})
    original, exported = archive_entries(fixture), archive_entries(output)
    for path, data in original.items():
        require(path in exported, f"Export lost source member {path}")
        if path.endswith('manifest.json'):
            contains_original(json.loads(data), json.loads(exported[path]), path)
        else:
            require(data == exported[path], f"Original asset bytes changed: {path}")
    host.post({"command": "import_profile", "path": str(output)})
    require(len(current_profile(host.state())["page_order"]) == 2, "Reimport lost visible page order")
    require(len(host.state()["slots"]) == 10, "Reimport lost active slots")
    return profile_id


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/deckforge")
    parser.add_argument("--evidence", type=Path, default=project.parent / "evidence")
    args = parser.parse_args()
    binary, evidence = args.binary.resolve(), args.evidence.resolve()
    counter = evidence / "official-samples/counter/com.elgato.counter.sdPlugin"
    fixture = evidence / "elgato-7.6/raw" / WINDOWS_FILE
    require(hashlib.sha256(fixture.read_bytes()).hexdigest() == WINDOWS_SHA256, "Windows Plus fixture source pin changed")
    with tempfile.TemporaryDirectory(prefix="deckforge-profiles-") as temporary:
        temporary = Path(temporary)
        probe = create_probe(temporary / (PROBE_PLUGIN + ".sdPlugin"), counter)
        state = temporary / "state"
        host = RunningHost(binary, [counter, probe], state, temporary / "host.log")
        try:
            host.ready()
            windows_id = check_windows_tree(host, fixture, state, temporary / "windows.streamDeckProfile")
            host.post({"command": "profile_create", "name": "Interop workflow"})
            snapshot = host.state()
            workflow_id = snapshot["library"]["active_profile"]
            first = snapshot["library"]["active_page"]
            host.post({"command": "page_delete", "id": first}, expected=400)
            require(host.state()["library"] == snapshot["library"], "Rejected last-page deletion changed navigation/library")
            host.post({"command": "assign", "controller": "Keypad", "index": 0, "uuid": PROBE_ACTION})
            host.post({"command": "assign", "controller": "Keypad", "index": 1, "uuid": COUNTER})
            snapshot = wait(host, lambda s: bool(events(s, "willAppear")), "probe appearance")
            probe_context, counter_context = at(snapshot, 0)["context"], at(snapshot, 1)["context"]
            host.post({"command": "settings", "context": counter_context, "settings": {"count": 7, "incrementBy": 2}})
            key(host, 1)
            wait(host, lambda s: at(s, 1).get("title") == "9", "Counter configured settings")
            host.post({"command": "page_create", "name": "Page two"})
            second = host.state()["library"]["active_page"]
            wait(host, lambda s: bool(events(s, "willDisappear", probe_context)), "probe disappearance on page change")
            snapshot = select_page(host, first)
            require(at(snapshot, 0)["context"] == probe_context and at(snapshot, 1)["context"] == counter_context, "Action contexts changed when revisiting a page")
            require(at(snapshot, 1)["settings"]["count"] == 9, "Hidden page lost Counter settings")
            host.post({"command": "input", "event": "keyDown", "index": 0})
            select_page(host, second)
            time.sleep(0.3)  # Probe intentionally sends its update 150 ms after leaving.
            snapshot = select_page(host, first)
            require(at(snapshot, 0)["settings"].get("hiddenReply") == 73, "Delayed plugin reply lost its hidden-page owner")
            host.post({"command": "slot_pin", "context": counter_context, "pinned": True})
            snapshot = select_page(host, second)
            require(at(snapshot, 1).get("pinned") and at(snapshot, 1)["context"] == counter_context, "Pinned action did not follow page")
            host.post({"command": "assign", "controller": "Keypad", "index": 1, "uuid": PROBE_ACTION})
            override_context = at(host.state(), 1)["context"]
            host.post({"command": "slot_pin", "context": override_context, "pinned": True}, expected=400)
            snapshot = select_page(host, first)
            require(at(snapshot, 1)["context"] == counter_context, "Page override deleted shared pin")
            snapshot = select_page(host, second)
            require(at(snapshot, 1)["context"] == override_context, "Rejected pin conflict deleted page-local override")
            select_page(host, first)
            host.post({"command": "folder_create", "name": "Nested", "index": 2})
            folder = at(host.state(), 2)["settings"]["ProfileUUID"]
            key(host, 2)
            snapshot = wait(host, lambda s: s["library"]["active_page"] == folder, "created folder")
            require(at(snapshot, 1).get("pinned") and at(snapshot, 1)["settings"]["count"] == 9, "Pinned action did not appear inside folder")
            host.post({"command": "profile_select", "id": windows_id})
            host.post({"command": "profile_select", "id": workflow_id})
            snapshot = host.state()
            require(snapshot["library"]["active_page"] == folder and snapshot["library"]["folder_depth"] == 1,
                    "Switching profiles lost folder cursor/ancestor stack")
            key(host, 0)
            wait(host, lambda s: s["library"]["active_page"] == first, "restored folder ancestor after profile switch")
            key(host, 2)
            wait(host, lambda s: s["library"]["active_page"] == folder, "folder reentry")
            host.post({"command": "slot_pin", "context": counter_context, "pinned": False})
            require(not at(host.state(), 1)["pinned"], "Unpin did not materialize in viewed folder")
            key(host, 0)
            snapshot = wait(host, lambda s: s["library"]["active_page"] == first, "created folder back")
            require(not at(snapshot, 1), "Unpin inside folder left global action behind")
            snapshot = select_page(host, second)
            require(at(snapshot, 1)["context"] == override_context, "Unpin destroyed page-specific override")
            host.post({"command": "profile_select", "id": windows_id})
            host.post({"command": "profile_select", "id": workflow_id})
            require(host.state()["library"]["active_page"] == second, "Profile cursor did not restore last active page")
            host.post({"command": "page_rename", "id": first, "name": "First renamed"})
            host.post({"command": "page_rename", "id": second, "name": "Second renamed"})
            select_page(host, first)
            host.post({"command": "export_profile", "path": str(temporary / "workflow.streamDeckProfile")})
            host.post({"command": "import_profile", "path": str(temporary / "workflow.streamDeckProfile")})
            snapshot = host.state()
            restored_id = snapshot["library"]["active_profile"]
            names = {p["name"] for p in current_profile(snapshot)["pages"]}
            require({"First renamed", "Second renamed", "Nested"} <= names, "Export/reimport lost native page/folder names")
            key(host, 2)
            snapshot = wait(host, lambda s: s["library"]["folder_depth"] == 1, "reimported folder reference")
            require(at(snapshot, 1)["settings"] == {"count": 9, "incrementBy": 2}, "Export/reimport lost moved Counter settings")
            require(not at(snapshot, 1)["pinned"], "Export/reimport changed unpin semantics")
            restored_page = snapshot["library"]["active_page"]
            profile_count = len(snapshot["library"]["profiles"])
        finally:
            host.stop()
        restart = RunningHost(binary, [counter, probe], state, temporary / "restart.log")
        try:
            restart.ready()
            snapshot = restart.state()
            require(len(snapshot["library"]["profiles"]) == profile_count, "Restart lost library profiles")
            require(snapshot["library"]["active_profile"] == restored_id and snapshot["library"]["active_page"] == restored_page, "Restart lost selected profile/folder")
            require(at(snapshot, 1)["settings"] == {"count": 9, "incrementBy": 2}, "Restart lost folder-owned settings")
            key(restart, 1)
            wait(restart, lambda s: at(s, 1).get("title") == "11", "Counter behavior after library restart")
            key(restart, 0)
            wait(restart, lambda s: s["library"]["folder_depth"] == 0, "parent stack after restart")
        finally:
            restart.stop()
    print("PASS: original Windows Plus whole-tree import (2 visible/8 stored), folder/back/navigation, source-field and asset preservation on export/reimport.")
    print("PASS: real plugin lifecycle, stable contexts/settings including hidden replies, pins in pages/folders, local overrides, folder-local unpin, export/reimport and restart library.")
    print("Simulation only; no USB opened. Official Windows acceptance of exported files remains unverified.")


if __name__ == "__main__":
    main()
