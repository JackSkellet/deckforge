#!/usr/bin/env python3
"""Bluetooth Buddy's line-delimited JSON protocol backed by Linux BlueZ."""

import json
import os
import selectors
import signal
import subprocess
import sys
import time


class BlueZError(RuntimeError):
    pass


active_busctl = None


def stop_busctl():
    if active_busctl is not None and active_busctl.poll() is None:
        active_busctl.terminate()
        try:
            active_busctl.wait(timeout=.5)
        except subprocess.TimeoutExpired:
            active_busctl.kill()
            active_busctl.wait()


def terminate(_signal, _frame):
    stop_busctl()
    raise SystemExit(0)


class BlueZ:
    def run(self, *args, timeout=5):
        global active_busctl
        try:
            active_busctl = subprocess.Popen(
                ["busctl", "--system", "--json=short", f"--timeout={timeout}s", *args],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            stdout, stderr = active_busctl.communicate(timeout=timeout + 1)
            if active_busctl.returncode:
                raise BlueZError(stderr.strip() or f"busctl exited {active_busctl.returncode}")
            return stdout
        except (OSError, subprocess.TimeoutExpired) as error:
            raise BlueZError(str(error)) from error
        finally:
            stop_busctl()
            active_busctl = None

    def objects(self):
        raw = self.run("call", "org.bluez", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects")
        try:
            value = json.loads(raw)
            objects = value["data"][0]
            if value["type"] != "a{oa{sa{sv}}}" or not isinstance(objects, dict):
                raise ValueError("unexpected GetManagedObjects response")
            return objects
        except (ValueError, KeyError, IndexError, TypeError) as error:
            raise BlueZError(f"Invalid BlueZ inventory: {error}") from error

    def device(self, path, connect, timeout):
        self.run("call", "org.bluez", path, "org.bluez.Device1",
                 "Connect" if connect else "Disconnect", timeout=timeout)

    def power(self, path, powered, timeout):
        self.run("set-property", "org.bluez", path, "org.bluez.Adapter1",
                 "Powered", "b", "true" if powered else "false", timeout=timeout)


def properties(interfaces, name):
    return {key: value["data"] for key, value in interfaces.get(name, {}).items()}


def category(device):
    return {
        "audio-headphones": "headphones", "audio-headset": "headset",
        "audio-card": "speaker", "audio-speakers": "speaker",
        "audio-input-microphone": "microphone", "input-keyboard": "keyboard",
        "input-mouse": "mouse", "input-gaming": "gamepad", "phone": "phone",
        "computer": "laptop", "watch": "watch",
    }.get(device.get("Icon"), "generic")


def inventory(objects):
    adapters, devices = {}, []
    for path, interfaces in sorted(objects.items()):
        if "org.bluez.Adapter1" in interfaces:
            adapters[path] = bool(properties(interfaces, "org.bluez.Adapter1").get("Powered"))
        device = properties(interfaces, "org.bluez.Device1")
        if not (device.get("Paired") or device.get("Bonded")) or not device.get("Address"):
            continue
        battery = properties(interfaces, "org.bluez.Battery1").get("Percentage")
        if type(battery) is not int or not 0 <= battery <= 100:
            battery = None
        # Bearer interfaces are optional. Do not guess LE from public/random addresses.
        le = "org.bluez.Bearer.LE1" in interfaces
        classic = "org.bluez.Bearer.BREDR1" in interfaces
        transport = "dual" if le and classic else "le" if le else "classic" if classic else "unknown"
        devices.append({
            "id": path, "address": device["Address"].lower(),
            "name": device.get("Alias") or device.get("Name") or device["Address"],
            "category": category(device), "connected": bool(device.get("Connected")),
            "battery": battery, "transport": transport, "canDisconnect": True,
        })
    radio = "on" if any(adapters.values()) else "off" if adapters else "unavailable"
    return adapters, devices, radio


class Helper:
    def __init__(self, bluez=None, emit=None):
        self.bluez = bluez or BlueZ()
        self.emit = emit or (lambda message: print(json.dumps(message), flush=True))
        self.previous = None
        self.last_error = None

    def refresh(self, force=False):
        try:
            state = inventory(self.bluez.objects())
            self.last_error = None
        except (BlueZError, KeyError, TypeError, ValueError) as error:
            state = ({}, [], "unavailable")
            if str(error) != self.last_error:
                self.emit({"type": "log", "level": "error", "message": str(error)})
                self.last_error = str(error)
        if force or self.previous != state:
            self.emit({"type": "radio", "state": state[2]})
            self.emit({"type": "snapshot", "devices": state[1]})
            self.previous = state
        return state

    def command(self, message):
        if not isinstance(message, dict):
            self.emit({"type": "log", "level": "error", "message": "Command must be an object"})
            return True
        cmd = message.get("cmd")
        if cmd == "quit":
            return False
        if cmd == "refresh":
            self.refresh(force=True)
            return True
        response = {"type": "result", "reqId": message.get("reqId"), "ok": False}
        try:
            if cmd not in ("connect", "disconnect", "disconnectAll", "radio"):
                raise BlueZError("Unknown command")
            # Refresh before every write, so a removed device ID cannot target stale state.
            adapters, devices, radio = inventory(self.bluez.objects())
            deadline = time.monotonic() + 25

            def remaining():
                seconds = deadline - time.monotonic()
                if seconds <= 0:
                    raise BlueZError("Bluetooth operation timed out")
                return min(seconds, 20)

            if cmd in ("connect", "disconnect"):
                device = next((item for item in devices if item["id"] == message.get("id")), None)
                if device is None:
                    raise BlueZError("Device is no longer paired or is unavailable")
                connect = cmd == "connect"
                if device["connected"] != connect:
                    self.bluez.device(device["id"], connect, remaining())
            elif cmd == "disconnectAll":
                failures = []
                for device in devices:
                    if device["connected"]:
                        try:
                            self.bluez.device(device["id"], False, remaining())
                        except BlueZError as error:
                            failures.append(f"{device['name']}: {error}")
                if failures:
                    raise BlueZError("; ".join(failures))
            else:
                state = message.get("state")
                if state not in ("toggle", "on", "off"):
                    raise BlueZError("Radio state must be on, off, or toggle")
                if not adapters:
                    raise BlueZError("No Bluetooth adapter available")
                powered = radio != "on" if state == "toggle" else state == "on"
                for path, current in adapters.items():
                    if powered != current:
                        self.bluez.power(path, powered, remaining())
            response["ok"] = True
        except (BlueZError, KeyError, TypeError, ValueError) as error:
            response["error"] = str(error)
        self.emit(response)
        self.refresh(force=True)
        return True


def main():
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    helper = Helper()
    helper.emit({"type": "ready"})
    helper.refresh(force=True)
    selector = selectors.PollSelector()
    selector.register(sys.stdin, selectors.EVENT_READ)
    pending = b""
    next_poll = time.monotonic() + 2
    try:
        while True:
            if selector.select(max(0, next_poll - time.monotonic())):
                chunk = os.read(sys.stdin.fileno(), 65536)
                if not chunk:
                    return 0
                pending += chunk
                if len(pending) > 1024 * 1024:
                    print("Command input exceeds 1 MiB", file=sys.stderr)
                    return 1
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    try:
                        message = json.loads(line)
                    except (ValueError, UnicodeError):
                        helper.emit({"type": "log", "level": "error", "message": "Invalid JSON command"})
                        continue
                    if not helper.command(message):
                        return 0
            if time.monotonic() >= next_poll:
                helper.refresh()
                next_poll = time.monotonic() + 2
    finally:
        selector.close()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (BrokenPipeError, KeyboardInterrupt):
        raise SystemExit(0)
