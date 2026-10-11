#!/usr/bin/env python3
"""Run unchanged official plugins against the real Rust host, without USB.

Usage: python tests/sample_interop.py [--binary target/debug/deckforge]
Fixtures: elgatosf/streamdeck-plugin-samples at SAMPLE_COMMIT below.
Counter is built with its untouched package-lock.json; Layouts uses its shipped ZIP.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

SAMPLE_COMMIT = "8cc82bad8716c000bacc131285ffcf6805dafff9"
LAYOUTS_SHA256 = "38f79ed3b5380ef0b9daf65ab60d52b170e18e2719078775afe16cf29394535b"
COUNTER_LOCK_SHA256 = "35672e0410d470ff51cbcbd0715a9ea7024148790726d796bbea6e5d3ba22102"
COUNTER = "com.elgato.counter.action"
FEEDBACK = "com.elgato.layouts.feedback"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class RunningHost:
    def __init__(self, binary, plugins, state, log):
        self.log = log
        self.output = log.open("w")
        self.process = subprocess.Popen(
            [str(binary), "serve", "--simulate", "--port", "0", "--state", str(state),
             *[item for plugin in plugins for item in ("--plugin", str(plugin))]],
            stdout=self.output, stderr=subprocess.STDOUT, start_new_session=True,
        )
        self.base = None
        self.token = None

    def ready(self):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            self.check_process()
            match = re.search(r"Editor (http://127\.0\.0\.1:\d+)", self.log.read_text())
            if match:
                self.base = match.group(1)
                try:
                    with urlopen(self.base, timeout=2) as response:
                        html = response.read().decode()
                    token = re.search(r"const\s+token\s*=\s*['\"]([^'\"]+)['\"]", html)
                    require(token, "Editor did not expose its session token")
                    self.token = token.group(1)
                    self.wait(lambda s: len(s["plugins"]) == 2 and all(p["connected"] for p in s["plugins"]),
                              "both unchanged official plugins to register")
                    return self
                except (URLError, TimeoutError):
                    pass
            time.sleep(0.05)
        raise AssertionError("Host did not become ready\n" + self.log.read_text())

    def check_process(self):
        require(self.process.poll() is None, "Host exited unexpectedly\n" + self.log.read_text())

    def state(self):
        self.check_process()
        with urlopen(self.base + "/api/state", timeout=3) as response:
            value = json.load(response)
        require(value["simulate"] is True, "Test must never access USB")
        return value

    def wait(self, predicate, description):
        deadline = time.monotonic() + 10
        last = None
        while time.monotonic() < deadline:
            last = self.state()
            if predicate(last):
                return last
            time.sleep(0.04)
        # Do not print preview base64 blobs in failures.
        if last:
            for slot in last.get("slots", []):
                slot.pop("preview", None)
        raise AssertionError(f"Timed out waiting for {description}: {last}\n{self.log.read_text()}")

    def post(self, data, expected=200, authenticated=True):
        headers = {"Content-Type": "application/json"}
        if authenticated:
            headers["X-Deckforge-Token"] = self.token
        request = Request(self.base + "/api/control", json.dumps(data).encode(), headers=headers, method="POST")
        try:
            with urlopen(request, timeout=3) as response:
                status, body = response.status, response.read()
        except HTTPError as error:
            status, body = error.code, error.read()
        require(status == expected, f"Expected HTTP {expected}, got {status}: {body.decode()}")

    def stop(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGTERM)
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(self.process.pid, signal.SIGKILL)
                    self.process.wait()
        self.output.close()


def slot(state, action):
    return next((s for s in state["slots"] if s["uuid"] == action), {})


def tap(host):
    host.post({"command": "input", "event": "keyDown", "index": 0})
    host.post({"command": "input", "event": "keyUp", "index": 0})


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/deckforge")
    parser.add_argument("--evidence", type=Path, default=project.parent / "evidence")
    args = parser.parse_args()
    binary, evidence = args.binary.resolve(), args.evidence.resolve()
    require(binary.is_file(), f"Build the Rust host first: {binary}")
    samples = evidence / "official-samples"
    counter = samples / "counter/com.elgato.counter.sdPlugin"
    layouts = evidence / "plugins/com.elgato.layouts.sdPlugin"
    for plugin in (counter, layouts):
        require((plugin / "bin/plugin.js").is_file(), f"Official fixture is not built/extracted: {plugin}")
    for path, expected in [
        (samples / "layouts/com.elgato.layouts.streamDeckPlugin", LAYOUTS_SHA256),
        (samples / "counter/package-lock.json", COUNTER_LOCK_SHA256),
    ]:
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, f"Fixture source pin changed: {path}")
    if (samples / ".git").exists():
        revision = subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip()
        require(revision == SAMPLE_COMMIT, "Official sample Git revision changed")

    with tempfile.TemporaryDirectory(prefix="deckforge-interop-") as temporary:
        temporary = Path(temporary)
        state = temporary / "state"
        host = RunningHost(binary, [counter, layouts], state, temporary / "first.log")
        try:
            host.ready()
            host.post({"command": "assign", "controller": "Keypad", "index": 0, "uuid": COUNTER})
            host.post({"command": "assign", "controller": "Encoder", "index": 0, "uuid": FEEDBACK})
            host.wait(lambda s: slot(s, COUNTER).get("title") == "0"
                      and slot(s, FEEDBACK).get("settings", {}).get("value") == 50,
                      "Counter and Layouts willAppear responses")
            tap(host)
            snapshot = host.wait(lambda s: slot(s, COUNTER).get("title") == "1", "Counter title to increment")
            counter_slot = slot(snapshot, COUNTER)
            require(counter_slot["settings"]["count"] == 1, "Counter did not persist its increment")
            host.post({"command": "settings", "context": counter_slot["context"],
                       "settings": {"count": 1, "incrementBy": 3}})
            tap(host)
            host.wait(lambda s: slot(s, COUNTER).get("title") == "4", "configured Counter increment")
            host.post({"command": "input", "event": "dialRotate", "index": 0, "ticks": 7})
            snapshot = host.wait(lambda s: slot(s, FEEDBACK).get("settings", {}).get("value") == 57
                                 and slot(s, FEEDBACK).get("feedback", {}).get("indicator", {}).get("value") == 57,
                                 "Layouts feedback and settings to reach 57")
            require(slot(snapshot, FEEDBACK)["feedback"]["value"] == 57, "Layouts displayed value mismatch")
            host.post({"command": "input", "event": "keyDown", "index": 0}, expected=403, authenticated=False)
            host.post({"command": "input", "event": "dialDown", "index": 999}, expected=400)
            host.post({"command": "input", "event": "dialRotate", "index": 0, "ticks": 128}, expected=400)
            host.post({"command": "settings", "context": counter_slot["context"], "settings": []}, expected=400)
            require(slot(host.state(), COUNTER)["settings"]["count"] == 4, "Rejected input changed Counter settings")
        finally:
            host.stop()

        saved = json.loads((state / "session.json").read_text())
        require(slot(saved, COUNTER)["settings"] == {"count": 4, "incrementBy": 3}, "Counter settings missing from disk")
        require(slot(saved, FEEDBACK)["settings"]["value"] == 57, "Layouts settings missing from disk")
        restarted = RunningHost(binary, [counter, layouts], state, temporary / "restart.log")
        try:
            restarted.ready()
            restarted.wait(lambda s: slot(s, COUNTER).get("title") == "4"
                           and slot(s, FEEDBACK).get("feedback", {}).get("value") == 57,
                           "plugin appearance after restart to restore persisted settings")
            tap(restarted)
            restarted.wait(lambda s: slot(s, COUNTER).get("title") == "7", "restored Counter increment setting")
        finally:
            restarted.stop()
    print(f"PASS: unchanged official Counter + Layouts at {SAMPLE_COMMIT}; registration, input, feedback, settings, restart, rejection checks. USB was never opened.")


if __name__ == "__main__":
    main()
