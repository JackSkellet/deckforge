#!/usr/bin/env python3
"""Run the Linux Bluetooth plugin through the real simulated Deckforge host.

Only the helper is replaced with a deterministic protocol fixture. No USB or
Bluetooth adapter access occurs. The Node plugin, manifest, images and host run
unchanged.
"""

import argparse
import base64
import json
import os
from pathlib import Path
import shutil
import tempfile
import time

from sample_interop import require, slot
from system_interop import Host
from feedback_interop import badge_pixels, preview

ROOT = Path(__file__).resolve().parents[1]
UUID = "com.flowkits.bluetooth"

STUB = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
root = Path(__file__).parent
(root / 'helper.pid').write_text(str(os.getpid()))
devices = [
 {'id':'one','address':'aa:bb:cc:dd:ee:01','name':'Test headphones','category':'headphones','connected':True,'battery':73,'transport':'classic','canDisconnect':True},
 {'id':'two','address':'aa:bb:cc:dd:ee:02','name':'Test keyboard','category':'keyboard','connected':False,'battery':None,'transport':'le','canDisconnect':True},
]
radio='on'
def emit(m): print(json.dumps(m),flush=True)
def refresh():
 emit({'type':'radio','state':radio})
 emit({'type':'snapshot','devices':devices})
emit({'type':'ready'})
refresh()
for line in sys.stdin:
 message=json.loads(line)
 with (root / 'commands.jsonl').open('a') as output: output.write(json.dumps(message)+'\n')
 cmd=message['cmd']
 if cmd=='quit': break
 if cmd=='refresh': refresh(); continue
 if (root / 'fail-command').exists():
  emit({'type':'result','reqId':message['reqId'],'ok':False,'error':'Fixture device refused connection'})
  continue
 if cmd in ('connect','disconnect'):
  for d in devices:
   if d['id']==message['id']: d['connected']=cmd=='connect'
 elif cmd=='disconnectAll':
  for d in devices: d['connected']=False
 elif cmd=='radio': radio='off' if radio=='on' else 'on'
 emit({'type':'result','reqId':message['reqId'],'ok':True})
 refresh()
'''


def commands(plugin):
    path = plugin / "commands.jsonl"
    return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


def dial_text(state):
    canvas = slot(state, UUID + ".battery-dial").get("feedback", {}).get("canvas", "")
    if isinstance(canvas, dict):
        canvas = canvas.get("value", "")
    return base64.b64decode(canvas.partition(",")[2]).decode() if canvas.startswith("data:") else ""


def input_event(host, event, index=0, **values):
    host.post({"command": "input", "event": event, "index": index, **values})


def press(host, index=0, dial=False):
    input_event(host, "dialDown" if dial else "keyDown", index)
    input_event(host, "dialUp" if dial else "keyUp", index)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path.home() / ".local/bin/deckforge")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="deckforge-bluetooth-interop-") as directory:
        root = Path(directory)
        plugin = root / "com.flowkits.bluetooth.sdPlugin"
        shutil.copytree(ROOT / "plugins/bluetooth-compat", plugin, ignore=shutil.ignore_patterns("__pycache__"))
        (plugin / "fk-bt-helper.py").write_text(STUB)
        (plugin / "fk-bt-helper.py").chmod(0o755)
        host = Host(args.binary.resolve(), [plugin], root / "state", root / "host.log")
        pid = None
        try:
            host.ready()
            host.wait(lambda s: any(p["connected"] for p in s["plugins"]), "Bluetooth registration")
            for kind, controller, index in (("device", "Keypad", 0), ("radio", "Keypad", 1),
                                             ("disconnect-all", "Keypad", 2), ("battery-dial", "Encoder", 0)):
                host.post({"command": "assign", "controller": controller, "index": index, "uuid": UUID + "." + kind})
            host.wait(lambda s: "Test headphones" in dial_text(s) and "73%" in dial_text(s), "helper snapshot and dial rendering")
            pid = int((plugin / "helper.pid").read_text())
            context = slot(host.state(), UUID + ".device")["context"]
            preserved = {"address": "aa:bb:cc:dd:ee:01", "customLabel": "My headphones", "showLabel": False,
                         "showBattery": True, "deviceName": "Prior name", "category": "headphones", "otherSetting": {"keep": True}}
            host.post({"command": "settings", "context": context, "settings": preserved})
            press(host)
            host.wait(lambda s: commands(plugin) and commands(plugin)[-1]["cmd"] == "disconnect"
                      and "Disconnected" in dial_text(s), "device key disconnect")
            press(host)
            host.wait(lambda s: commands(plugin)[-1]["cmd"] == "connect" and "Connected" in dial_text(s), "device key reconnect")
            require(slot(host.state(), UUID + ".device")["settings"] == preserved, "Plugin changed existing settings")

            input_event(host, "dialRotate", ticks=1)
            host.wait(lambda s: "Test keyboard" in dial_text(s) and "Battery unavailable" in dial_text(s), "dial selection and absent battery")
            press(host, dial=True)
            host.wait(lambda s: commands(plugin)[-1].get("id") == "two" and "Connected" in dial_text(s), "dial push connects selected device")
            input_event(host, "touchTap", x=100, y=40, hold=False)
            host.wait(lambda s: "Test headphones" in dial_text(s), "touch cycles device")
            input_event(host, "dialRotate", ticks=-1)
            host.wait(lambda s: "Test keyboard" in dial_text(s), "negative rotation wraps")

            dial_context = slot(host.state(), UUID + ".battery-dial")["context"]
            dial_settings = {"pinnedAddress": "aa:bb:cc:dd:ee:01", "customLabel": "Pinned headphones"}
            host.post({"command": "settings", "context": dial_context, "settings": dial_settings})
            input_event(host, "dialRotate", ticks=1)
            host.wait(lambda s: "Pinned headphones" in dial_text(s) and "73%" in dial_text(s), "pinned dial survives rotation")

            press(host, index=1)
            host.wait(lambda s: commands(plugin)[-1]["cmd"] == "radio" and "Bluetooth off" in dial_text(s), "radio toggle off")
            press(host, index=1)
            host.wait(lambda s: "Connected" in dial_text(s), "radio toggle on")
            press(host, index=2)
            host.wait(lambda s: commands(plugin)[-1]["cmd"] == "disconnectAll" and "Disconnected" in dial_text(s), "disconnect all")

            (plugin / "fail-command").touch()
            previous = len(commands(plugin))
            press(host, dial=True)
            host.wait(lambda s: len(commands(plugin)) > previous and "Failed: see settings" in dial_text(s), "failed operation visible on dial")
            plugin_log = Path(next(p["log_path"] for p in host.state()["plugins"] if p["id"] == UUID))
            require("Fixture device refused connection" in plugin_log.read_text(), "Detailed failure was not logged")
            require(slot(host.state(), UUID + ".battery-dial")["settings"] == dial_settings, "Failure discarded settings")
            press(host)
            host.wait(lambda s: badge_pixels(preview(slot(s, UUID + ".device")), "alert") > 30, "failed key command renders warning badge")
            (plugin / "fail-command").unlink()
            host.post({"command": "settings", "context": dial_context, "settings": dial_settings})
            host.wait(lambda s: "Failed:" not in dial_text(s), "settings clear recoverable failure")
            press(host, dial=True)
            host.wait(lambda s: "Connected" in dial_text(s), "retry succeeds")
        finally:
            host.stop()
            require(host.process.returncode == 0, "Host did not stop cleanly")
            if pid:
                deadline = time.monotonic() + 3
                while Path(f"/proc/{pid}").exists() and time.monotonic() < deadline:
                    time.sleep(.04)
                require(not Path(f"/proc/{pid}").exists(), "Helper child survived plugin shutdown")
        saved = json.loads((root / "state/session.json").read_text())
        require(slot(saved, UUID + ".device")["settings"] == preserved, "Prior settings not preserved on disk")
        require(slot(saved, UUID + ".battery-dial")["settings"] == dial_settings, "Dial settings not saved")
    print("PASS: real simulated host + Linux Node plugin; four actions, device toggles, dial rotation/push/touch/pinning, radio, disconnect all, failure/retry, preserved settings, helper cleanup. No USB or real Bluetooth writes.")


if __name__ == "__main__":
    main()
