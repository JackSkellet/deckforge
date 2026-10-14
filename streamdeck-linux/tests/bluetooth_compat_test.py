#!/usr/bin/env python3
"""Focused BlueZ protocol and reversible installer checks. No live writes."""

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


helper = load("bluetooth_helper", ROOT / "plugins/bluetooth-compat/fk-bt-helper.py")
installer = load("bluetooth_installer", ROOT / "scripts/install-bluetooth-compat.py")
ADAPTER = "/org/bluez/hci0"
DEVICE = ADAPTER + "/dev_AA_BB_CC_DD_EE_FF"


def props(**values):
    return {key: {"type": "v", "data": value} for key, value in values.items()}


def fixture():
    return {
        ADAPTER: {"org.bluez.Adapter1": props(Powered=True)},
        DEVICE: {
            "org.bluez.Device1": props(Address="AA:BB:CC:DD:EE:FF", Alias="Headphones",
                                       Paired=True, Connected=True, Icon="audio-headphones"),
            "org.bluez.Battery1": props(Percentage=73),
            "org.bluez.Bearer.BREDR1": {},
        },
        ADAPTER + "/dev_00": {"org.bluez.Device1": props(Address="00", Paired=False)},
    }


class FakeBlueZ:
    def __init__(self):
        self.data = fixture()
        self.calls = []
        self.error = None

    def objects(self):
        if self.error:
            raise helper.BlueZError(self.error)
        return copy.deepcopy(self.data)

    def device(self, path, connect, timeout):
        self.calls.append((path, connect))
        self.data[path]["org.bluez.Device1"]["Connected"]["data"] = connect

    def power(self, path, powered, timeout):
        self.calls.append((path, powered))
        self.data[path]["org.bluez.Adapter1"]["Powered"]["data"] = powered


class HelperTests(unittest.TestCase):
    def setUp(self):
        self.bus = FakeBlueZ()
        self.messages = []
        self.helper = helper.Helper(self.bus, self.messages.append)

    def test_inventory_filters_unpaired_and_keeps_battery_and_category(self):
        self.helper.refresh()
        devices = self.messages[-1]["devices"]
        self.assertEqual(len(devices), 1)
        self.assertEqual(devices[0]["address"], "aa:bb:cc:dd:ee:ff")
        self.assertEqual(devices[0]["battery"], 73)
        self.assertEqual(devices[0]["category"], "headphones")
        self.assertEqual(devices[0]["transport"], "classic")
        self.helper.refresh()
        self.assertEqual(len(self.messages), 2)

    def test_removed_device_disappears_and_rejects_stale_command(self):
        self.helper.refresh()
        del self.bus.data[DEVICE]
        self.helper.command({"cmd": "disconnect", "id": DEVICE, "reqId": 4})
        self.assertFalse(next(m for m in self.messages if m["type"] == "result")["ok"])
        self.assertEqual(self.messages[-1]["devices"], [])
        self.assertEqual(self.bus.calls, [])

    def test_disconnect_connect_and_idempotent_connect(self):
        for cmd in ("disconnect", "connect", "connect"):
            self.helper.command({"cmd": cmd, "id": DEVICE, "reqId": 2})
        self.assertEqual(self.bus.calls, [(DEVICE, False), (DEVICE, True)])
        self.assertTrue(all(m["ok"] for m in self.messages if m["type"] == "result"))

    def test_adapter_toggle_off_and_on(self):
        self.helper.command({"cmd": "radio", "state": "toggle", "reqId": 3})
        self.helper.command({"cmd": "radio", "state": "on", "reqId": 4})
        self.assertEqual(self.bus.calls, [(ADAPTER, False), (ADAPTER, True)])

    def test_disconnect_all_and_failure_propagation(self):
        self.helper.command({"cmd": "disconnectAll", "reqId": 7})
        self.assertEqual(self.bus.calls, [(DEVICE, False)])
        self.bus.data[DEVICE]["org.bluez.Device1"]["Connected"]["data"] = True
        with patch.object(self.bus, "device", side_effect=helper.BlueZError("NotAuthorized")):
            self.helper.command({"cmd": "disconnectAll", "reqId": 8})
        failure = next(m for m in self.messages if m.get("reqId") == 8)
        self.assertFalse(failure["ok"])
        self.assertIn("NotAuthorized", failure["error"])

    def test_service_loss_clears_stale_state_then_recovers(self):
        self.helper.refresh()
        self.bus.error = "Service unavailable"
        self.helper.refresh()
        self.assertEqual(self.messages[-2], {"type": "radio", "state": "unavailable"})
        self.assertEqual(self.messages[-1]["devices"], [])
        self.helper.command({"cmd": "connect", "id": DEVICE, "reqId": 9})
        self.assertFalse(next(m for m in self.messages if m.get("reqId") == 9)["ok"])
        self.bus.error = None
        self.helper.refresh()
        self.assertEqual(len(self.messages[-1]["devices"]), 1)

    def test_invalid_radio_and_arbitrary_object_do_not_write(self):
        self.helper.command({"cmd": "radio", "state": "invalid"})
        self.helper.command({"cmd": "disconnect", "id": "/org/bluez/hci0"})
        self.helper.command([])
        self.assertEqual(self.bus.calls, [])

    def test_busctl_wire_format_errors_and_commands(self):
        bus = helper.BlueZ()
        with patch.object(bus, "run", return_value=json.dumps({"type": "a{oa{sa{sv}}}", "data": [fixture()]})):
            self.assertEqual(bus.objects(), fixture())
        with patch.object(bus, "run", return_value='{"data":[]}'):
            self.assertRaises(helper.BlueZError, bus.objects)
        process = Mock(returncode=1)
        process.communicate.return_value = ("", "Access denied")
        with patch("subprocess.Popen", return_value=process) as run:
            self.assertRaisesRegex(helper.BlueZError, "Access denied", bus.device, DEVICE, True, 20)
            self.assertEqual(run.call_args.args[0][-4:], ["org.bluez", DEVICE, "org.bluez.Device1", "Connect"])

    def test_process_json_lines_invalid_json_and_eof(self):
        with tempfile.TemporaryDirectory() as directory:
            fake = Path(directory) / "busctl"
            fake.write_text("#!/usr/bin/env python3\nprint(" + repr(json.dumps({"type": "a{oa{sa{sv}}}", "data": [fixture()]})) + ")\n")
            fake.chmod(0o755)
            env = {**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"]}
            result = subprocess.run([sys.executable, helper.__file__], input='invalid\n{"cmd":"refresh"}\n',
                                    text=True, capture_output=True, env=env, timeout=5)
            messages = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(messages[0], {"type": "ready"})
            self.assertEqual(len([m for m in messages if m["type"] == "snapshot"]), 2)
            self.assertIn("Invalid JSON", next(m["message"] for m in messages if m["type"] == "log"))

    def test_sigterm_reaps_blocked_busctl_child(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake = root / "busctl"
            pid_file = root / "child.pid"
            fake.write_text("#!/usr/bin/env python3\nimport os,time\nfrom pathlib import Path\n"
                            + f"Path({str(pid_file)!r}).write_text(str(os.getpid()))\n"
                            + "time.sleep(60)\n")
            fake.chmod(0o755)
            env = {**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"]}
            process = subprocess.Popen([sys.executable, helper.__file__], stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
            try:
                deadline = time.monotonic() + 3
                while not pid_file.exists() and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertTrue(pid_file.exists(), "busctl fixture did not start")
                child_pid = int(pid_file.read_text())
                process.terminate()
                process.communicate(timeout=3)
                self.assertEqual(process.returncode, 0)
                self.assertFalse(Path(f"/proc/{child_pid}").exists(), "busctl survived helper termination")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.source.mkdir()
        (self.source / "manifest.json").write_text(json.dumps({"UUID": installer.UUID, "CodePath": "plugin.mjs"}))
        (self.source / "plugin.mjs").write_text("original source")
        (self.source / "fk-bt-helper.py").write_text("#!/usr/bin/env python3\n")
        self.target = self.root / "plugins" / "com.flowkits.bluetooth.sdPlugin"

    def test_install_idempotence_update_backup_and_local_change_guard(self):
        self.assertEqual(installer.install(self.source, self.target)["status"], "installed")
        self.assertTrue(os.access(self.target / "fk-bt-helper.py", os.X_OK))
        self.assertEqual(installer.install(self.source, self.target)["status"], "already installed")
        (self.source / "plugin.mjs").write_text("new source")
        result = installer.install(self.source, self.target)
        self.assertEqual((Path(result["backup"]) / "plugin.mjs").read_text(), "original source")
        (self.target / "plugin.mjs").write_text("local edit")
        self.assertRaisesRegex(ValueError, "changed", installer.install, self.source, self.target)

    def test_unknown_stock_refused_and_original_package_backed_up(self):
        self.target.mkdir(parents=True)
        manifest = self.target / "manifest.json"
        manifest.write_bytes(b"ELGATO\x00\xff\xfe protected binary fixture")
        (self.target / "bin").mkdir()
        stock = self.target / "bin/plugin.js"
        stock.write_text("stock fixture")
        self.assertRaisesRegex(ValueError, "Unrecognized", installer.install, self.source, self.target)
        with patch.object(installer, "STOCK_JS_SHA256", installer.digest(stock)), \
                patch.object(installer, "STOCK_MANIFEST_SHA256", installer.digest(manifest)):
            result = installer.install(self.source, self.target)
        self.assertEqual((Path(result["backup"]) / "bin/plugin.js").read_text(), "stock fixture")
        self.assertEqual((Path(result["backup"]) / "manifest.json").read_bytes(), b"ELGATO\x00\xff\xfe protected binary fixture")

    def test_rename_failure_restores_previous_package(self):
        installer.install(self.source, self.target)
        (self.source / "plugin.mjs").write_text("update")
        original_rename = os.rename

        def fail_stage(source, target):
            if Path(source).name.startswith(".bluetooth-linux-stage"):
                raise OSError("fixture rename failure")
            return original_rename(source, target)

        with patch("os.rename", side_effect=fail_stage):
            self.assertRaises(OSError, installer.install, self.source, self.target)
        self.assertEqual((self.target / "plugin.mjs").read_text(), "original source")


if __name__ == "__main__":
    unittest.main()
