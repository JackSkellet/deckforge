#!/usr/bin/env python3
"""Native window destruction/recreation with a live SDK plugin; no USB or OS input."""
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
from urllib.error import HTTPError
from urllib.request import urlopen

from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT

PLUGIN = "com.deckforge.window-probe"
JS = r'''import streamDeck from __SDK__;
import {connection} from __CONNECTION__;
import {appendFileSync} from 'node:fs';
const proof=__PROOF__; let seq=0,acks=0;
const record=(kind,extra={})=>appendFileSync(proof,JSON.stringify({kind,seq:++seq,pid:process.pid,ppid:process.ppid,at:Date.now(),...extra})+'\n');
process.on('unhandledRejection',error=>{record('failure',{error:String(error)});process.exit(1)});
connection.on('didReceiveGlobalSettings',message=>{acks++;record('ack',{id:message.id})});
await streamDeck.connect();record('registered');
while(true){
 let timer;
 try {
  await Promise.race([streamDeck.settings.getGlobalSettings(),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Host acknowledgement timed out')),3000)})]);
  record('heartbeat',{acknowledgements:acks});
 } finally {clearTimeout(timer)}
 await new Promise(resolve=>setTimeout(resolve,200));
}
'''


def wait(predicate, description, timeout=15):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        result = predicate()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("Timed out: " + description)


def rows(proof):
    if not proof.exists():
        return []
    return [json.loads(line) for line in proof.read_text().splitlines(keepends=True)
            if line.endswith("\n")]


def pid_identity(pid):
    try:
        # Field22, starttime, distinguishes a child from a subsequently reused PID.
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
    except FileNotFoundError:
        return None


def heartbeat(proof, offset, host_pid, child, phase):
    before = sum(row["kind"] == "heartbeat" for row in rows(proof)[offset:])
    def advanced():
        data = rows(proof)[offset:]
        failures = [row for row in data if row["kind"] == "failure"]
        assert not failures, failures
        registered = [row for row in data if row["kind"] == "registered"]
        assert len(registered) <= 1, "Window operation restarted or duplicated the plugin"
        if not registered:
            return None
        pid = registered[0]["pid"]
        identity = pid_identity(pid)
        assert identity is not None, "Plugin child disappeared"
        if child:
            assert (pid, identity) == child, "Plugin process changed across window lifecycle"
        assert all(row["pid"] == pid and row["ppid"] == host_pid for row in data), data
        beats = [row for row in data if row["kind"] == "heartbeat"]
        acknowledgements = [row for row in data if row["kind"] == "ack"]
        if len(beats) < before + 3:
            return None
        assert len(acknowledgements) >= len(beats)
        assert all(row.get("id") for row in acknowledgements), "Host dropped SDK message IDs"
        assert beats[-1]["acknowledgements"] >= len(beats)
        return ((pid, identity), {"phase": phase, "host_pid": host_pid, "plugin_pid": pid,
                                 "plugin_starttime": identity, "heartbeats": len(beats),
                                 "acknowledgements": len(acknowledgements), "last_seq": data[-1]["seq"]})
    return wait(advanced, f"live plugin acknowledgements while {phase}")


def cleanup(process, proof):
    # All test launches have their own session/process group. If the host failed
    # after exiting, only signal a group with this fixture's identifiable child.
    owned_child = False
    for row in rows(proof):
        if row["ppid"] != process.pid:
            continue
        try:
            commandline = Path(f"/proc/{row['pid']}/cmdline").read_bytes()
            owned_child |= (str(proof.parent / "plugin.mjs").encode() in commandline
                            and os.getpgid(row["pid"]) == process.pid)
        except (FileNotFoundError, ProcessLookupError):
            pass
    if process.poll() is None or owned_child:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/release/deckforge")
    parser.add_argument("--samples", type=Path, default=project.parent / "evidence/official-samples")
    parser.add_argument("--report", type=Path, help="Save generated-fixture provenance and heartbeat evidence")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    samples = args.samples.resolve(strict=True)
    sdk = samples / "counter/node_modules/@elgato/streamdeck"
    assert (sdk / "dist/plugin/index.js").is_file(), "Build the official Counter sample dependencies first"
    assert subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip() == SAMPLE_COMMIT
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    report = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "sample_commit": SAMPLE_COMMIT, "counter_lock_sha256": COUNTER_LOCK_SHA256,
              "sdk_version": json.loads((sdk / "package.json").read_text())["version"],
              "sdk_entry_sha256": hashlib.sha256((sdk / "dist/plugin/index.js").read_bytes()).hexdigest(),
              "boundary": "Generated plugin using unchanged official SDK; simulation only; no USB or OS input",
              "runs": []}
    with tempfile.TemporaryDirectory(prefix="deckforge-window-") as directory:
        root = Path(directory)
        state = root / "state"
        plugin = root / "window-probe.sdPlugin"
        plugin.mkdir()
        proof = plugin / "proof.jsonl"
        source = (JS.replace("__SDK__", json.dumps((sdk / "dist/plugin/index.js").as_uri()))
                  .replace("__CONNECTION__", json.dumps((sdk / "dist/plugin/connection.js").as_uri()))
                  .replace("__PROOF__", json.dumps(str(proof))))
        (plugin / "plugin.mjs").write_text(source)
        (plugin / "manifest.json").write_text(json.dumps({
            "UUID": PLUGIN, "Name": "Window heartbeat probe", "Version": "1.0.0",
            "SDKVersion": 2, "Software": {"MinimumVersion": "7.1"},
            "CodePath": "plugin.mjs", "Nodejs": {"Version": "20"},
            "Actions": [{"UUID": PLUGIN + ".key", "Name": "Heartbeat probe",
                         "Controllers": ["Keypad"], "States": [{}]}]}))
        report["fixture_sha256"] = hashlib.sha256(source.encode()).hexdigest()

        def command(name, target=state, required=True):
            result = subprocess.run([str(binary), "window", name, "--state", str(target)],
                                    capture_output=True, text=True, timeout=4)
            if not required:
                return result
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)

        def status_ready(target=state):
            result = command("status", target, False)
            return json.loads(result.stdout) if result.returncode == 0 else None

        def window(visible, generation, target=state):
            def matches():
                status = command("status", target)
                return status if (status["visible"] is visible and status["window_open"] is visible
                                  and status["editor_generation"] == generation) else None
            return wait(matches, f"window_open={visible}, generation={generation}")

        def launch(target, output, env):
            return subprocess.Popen([str(binary), "desktop", "--simulate", "--background",
                                     "--state", str(target), "--plugin", str(plugin)], env=env,
                                    stdout=output, stderr=subprocess.STDOUT, start_new_session=True)

        assert command("status", required=False).returncode != 0
        assert not state.exists(), "Status lookup created user state"
        # An unavailable session bus must not prevent recovery through the launcher.
        for offline in (False, True):
            log = root / f"native-{offline}.log"
            env = os.environ.copy()
            if offline:
                env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=" + str(root / "absent-bus")
            offset = len(rows(proof))
            run = {"missing_bus": offline, "checkpoints": []}
            with log.open("w") as output:
                process = launch(state, output, env)
                child = None
                try:
                    wait(status_ready, "native IPC initialization")
                    initial = window(False, 0)
                    assert initial["pid"] == process.pid, initial
                    child, checkpoint = heartbeat(proof, offset, process.pid, child, "background without window")
                    run["checkpoints"].append(checkpoint)
                    assert command("status", root / "other", False).returncode != 0
                    alias = root / "alias"
                    if not alias.exists():
                        alias.symlink_to(state, target_is_directory=True)
                    assert command("status", alias)["pid"] == process.pid
                    base = re.search(r"(http://127\.0\.0\.1:\d+)", log.read_text())
                    assert base, log.read_text()
                    for route in ("/", "/api/state"):
                        try:
                            urlopen(base.group(1) + route, timeout=2)
                        except HTTPError as error:
                            assert error.code == 404
                        else:
                            raise AssertionError("Native HTTP editor exposed")
                    for generation in (1, 2):
                        activated = subprocess.run([str(binary), "desktop", "--simulate", "--state", str(alias)],
                                                   capture_output=True, text=True, timeout=5)
                        assert activated.returncode == 0, activated.stderr
                        assert "Existing Deckforge" in activated.stdout
                        assert window(True, generation)["pid"] == process.pid
                        child, checkpoint = heartbeat(proof, offset, process.pid, child, f"relaunch generation {generation}")
                        run["checkpoints"].append(checkpoint)
                        command("close")
                        window(False, generation)
                        assert process.poll() is None
                        child, checkpoint = heartbeat(proof, offset, process.pid, child, f"closed window generation {generation}")
                        run["checkpoints"].append(checkpoint)
                    command("show")
                    window(True, 3)
                    command("hide")
                    window(False, 3)
                    child, checkpoint = heartbeat(proof, offset, process.pid, child, "hidden before shutdown")
                    run["checkpoints"].append(checkpoint)
                    if offline:
                        wait(lambda: command("status")["tray_error"], "missing bus is reported")
                        assert not command("status")["tray_online"]
                        process.send_signal(signal.SIGTERM)
                    else:
                        command("quit")
                    assert process.wait(timeout=10) == 0, log.read_text()
                    wait(lambda: pid_identity(child[0]) != child[1], "plugin child reaped after host shutdown", 5)
                    assert (state / "session.json").exists()
                    assert command("status", required=False).returncode != 0
                    run.update({"shutdown": "hidden_sigterm" if offline else "quit", "child_reaped": True,
                                "events": rows(proof)[offset:]})
                    report["runs"].append(run)
                except Exception:
                    print(log.read_text())
                    print(json.dumps(rows(proof)[offset:][-12:], indent=2))
                    raise
                finally:
                    cleanup(process, proof)

        # Two launchers overlap while the state lock and endpoint are being created.
        race_state = root / "race-state"
        offset = len(rows(proof))
        processes = []
        logs = [root / f"race-{index}.log" for index in range(2)]
        outputs = [path.open("w") for path in logs]
        try:
            for output in outputs:
                processes.append(launch(race_state, output, os.environ.copy()))
            status = wait(lambda: status_ready(race_state), "simultaneous-launch instance")
            owner = next(process for process in processes if process.pid == status["pid"])
            other = next(process for process in processes if process is not owner)
            assert other.wait(timeout=10) == 0, "\n".join(path.read_text() for path in logs)
            assert "Existing Deckforge" in logs[processes.index(other)].read_text()
            window(False, 0, race_state)
            child, checkpoint = heartbeat(proof, offset, owner.pid, None, "simultaneous launch winner")
            command("quit", race_state)
            assert owner.wait(timeout=10) == 0
            wait(lambda: pid_identity(child[0]) != child[1], "race winner plugin reaped", 5)
            report["race"] = {"launcher_pids": [process.pid for process in processes],
                              "owner_pid": owner.pid, "forwarded_pid": other.pid,
                              "checkpoint": checkpoint, "child_reaped": True,
                              "events": rows(proof)[offset:]}
        except Exception:
            print("\n".join(path.read_text() for path in logs))
            raise
        finally:
            for process in processes:
                cleanup(process, proof)
            for output in outputs:
                output.close()
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: background without window, native Close destroys window, generation-counted relaunch/IPC recreation, same host/plugin PID and acknowledged heartbeats throughout, quit/hidden SIGTERM reap child, concurrent launcher ownership, state aliases/isolation, missing tray bus, native HTTP routes absent. USB never opened.")


if __name__ == "__main__":
    main()
