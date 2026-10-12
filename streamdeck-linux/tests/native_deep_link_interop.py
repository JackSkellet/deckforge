#!/usr/bin/env python3
"""Native URI activation through owned processes/IPC and an unchanged SDK fixture.

Temporary simulated state only. Does not invoke OS URI dispatch or a real opener.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

from native_editor_interop import cleanup, command, environment, wait, result
from sample_interop import COUNTER_LOCK_SHA256, SAMPLE_COMMIT
from window_interop import rows

PLUGIN = "com.deckforge.uri-probe"
SCRIPT = r'''
import streamDeck from __SDK__;
import {connection} from __CONNECTION__;
import {appendFileSync} from 'node:fs';
const record=value=>appendFileSync(__PROOF__,JSON.stringify({pid:process.pid,...value})+'\n');
connection.on('didReceiveDeepLink',message=>record({kind:'link',message}));
await new Promise(resolve=>setTimeout(resolve,600));
await streamDeck.connect();record({kind:'registered'});
'''


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/release/deckforge")
    parser.add_argument("--samples", type=Path, default=project.parent / "evidence/official-samples")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    binary, samples = args.binary.resolve(strict=True), args.samples.resolve(strict=True)
    sdk = samples / "counter/node_modules/@elgato/streamdeck"
    assert (sdk / "dist/plugin/index.js").is_file()
    assert hashlib.sha256((samples / "counter/package-lock.json").read_bytes()).hexdigest() == COUNTER_LOCK_SHA256
    assert subprocess.check_output(["git", "-C", str(samples), "rev-parse", "HEAD"], text=True).strip() == SAMPLE_COMMIT
    report = {"ok": False, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "sample_commit": SAMPLE_COMMIT, "boundary": "Owned native IPC and generated plugin using unchanged SDK; temporary simulation; no USB, user URI defaults or external network"}
    report_path = args.report.resolve()
    report_path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="deckforge-native-uri-") as temporary:
        root = Path(temporary)
        data = root / "isolated data"
        state = data / "deckforge"
        plugin = root / "uri-probe.sdPlugin"
        plugin.mkdir()
        proof = plugin / "proof.jsonl"
        source = SCRIPT.replace("__SDK__", json.dumps((sdk / "dist/plugin/index.js").as_uri())).replace("__CONNECTION__", json.dumps((sdk / "dist/plugin/connection.js").as_uri())).replace("__PROOF__", json.dumps(str(proof)))
        (plugin / "plugin.mjs").write_text(source)
        (plugin / "manifest.json").write_text(json.dumps({"UUID": PLUGIN, "Name": "URI probe", "Version": "1.0.0", "SDKVersion": 2, "Software": {"MinimumVersion": "7.1"}, "CodePath": "plugin.mjs", "Nodejs": {"Version": "20"}, "Actions": [{"UUID": PLUGIN + ".key", "Name": "URI probe", "Controllers": ["Keypad"], "States": [{}]}]}))
        env = environment(root)
        env["XDG_DATA_HOME"] = str(data)
        env["XDG_CACHE_HOME"] = str(root / "cache")
        cold_tail = "/cold?code=fixture%2Fonly&streamdeck=HiDdEn#Keep"
        uri = lambda tail, target=PLUGIN: f"streamdeck://plugins/message/{target}{tail}"
        ui_script, ui_proof = root / "ui-probe.js", root / "ui-proof.json"
        ui_script.write_text("""for(let n=0;n<120;n++){const s=await window.deckforge.request('snapshot');const text=document.querySelector('.statusbar [role=status]')?.textContent;if(text&&text===s.system_status)return{status:s.system_status,visible_status:text};await new Promise(r=>setTimeout(r,40));}throw Error('Native status did not render');""")
        env.update(DECKFORGE_WEBVIEW_TEST_SCRIPT=str(ui_script), DECKFORGE_WEBVIEW_TEST_RESULT=str(ui_proof))
        log_path = root / "native.log"
        process = None
        try:
            with log_path.open("w") as log:
                process = subprocess.Popen([str(binary), "desktop", "--simulate", "--state", str(state), "--plugin", str(plugin), "--uri", uri(cold_tail)], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                status = wait(lambda: command(binary, state, "status", required=False), "native URI endpoint")
                assert status["pid"] == process.pid and not status["visible"] and status["editor_generation"] == 0
                wait(lambda: any(row.get("kind") == "link" for row in rows(proof)), "queued cold callback after registration")

                def callback(tail, positional=False, target=PLUGIN, success=True):
                    before = len([row for row in rows(proof) if row.get("kind") == "link"])
                    arguments = [str(binary), uri(tail, target)] if positional else [str(binary), "desktop", "--simulate", "--state", str(state), "--uri", uri(tail, target)]
                    call = subprocess.run(arguments, env=env, capture_output=True, text=True, timeout=5)
                    assert (call.returncode == 0) == success, call.stderr
                    assert "SECRET_NOT_IN_LOG" not in call.stdout + call.stderr
                    if success:
                        wait(lambda: len([row for row in rows(proof) if row.get("kind") == "link"]) == before + 1, "one targeted callback")
                    return call

                active_tail = "/Active%20callback?x=%2F&x=a+b#Case"
                callback(active_tail, positional=True)
                wait(lambda: (value := command(binary, state, "status"))["visible"] and value["window_open"] and value["editor_generation"] == 1, "active URI shows same native window")
                wait(lambda: result(ui_proof), "active callback status rendered")
                command(binary, state, "hide")
                wait(lambda: not command(binary, state, "status")["window_open"], "window hidden")
                # Real GIO Desktop Entry field-code expansion, against this
                # owned simulated state. No default scheme registration occurs.
                spec = importlib.util.spec_from_file_location("installer", project / "scripts/install-desktop.py")
                installer = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(installer)
                entry = data / "applications/owned-uri-probe.desktop"
                entry.parent.mkdir(parents=True)
                entry.write_text("[Desktop Entry]\nType=Application\nName=Owned URI probe\nTerminal=false\n"
                                 + f"Exec={installer.desktop_quote(binary)} desktop --simulate --state {installer.desktop_quote(state)} --uri %u\n"
                                 + "MimeType=x-scheme-handler/streamdeck;\n")
                entry.chmod(0o700)
                gio_tail = "/GIO%20callback?code=A%252FB&repeat=a+b&repeat=%26%3D&literal=%24HOME%3B&streamdeck=Hidden#%22Quoted%22"
                before_gio = len([row for row in rows(proof) if row.get("kind") == "link"])
                gio = subprocess.run(["gio", "launch", str(entry), uri(gio_tail)], env=env, capture_output=True, text=True, timeout=10)
                assert gio.returncode == 0, gio.stderr
                wait(lambda: len([row for row in rows(proof) if row.get("kind") == "link"]) == before_gio + 1, "GIO percent-u callback roundtrip")
                assert not command(binary, state, "status")["visible"]
                hidden_tail = "?streamdeck=HIDDEN&x=%252F#passive"
                callback(hidden_tail)
                status = command(binary, state, "status")
                assert status["pid"] == process.pid and not status["visible"] and status["editor_generation"] == 1
                callback("/callback?code=SECRET_NOT_IN_LOG&streamdeck=HIDDEN", target="com.deckforge.missing", success=False)
                assert not command(binary, state, "status")["visible"]
                ui_proof.unlink(missing_ok=True)
                callback("/callback?code=SECRET_NOT_IN_LOG", target="com.deckforge.missing", success=False)
                wait(lambda: (s := command(binary, state, "status"))["pid"] == process.pid and s["visible"] and s["window_open"] and s["editor_generation"] == 2, "unknown active callback displays native error window")
                rendered_error = wait(lambda: result(ui_proof), "sanitized native callback error")
                assert "not installed" in rendered_error["result"]["visible_status"]
                assert "SECRET_NOT_IN_LOG" not in json.dumps(rendered_error)
                messages = [row["message"] for row in rows(proof) if row.get("kind") == "link"]
                assert messages == [{"event": "didReceiveDeepLink", "payload": {"url": tail}} for tail in (cold_tail, active_tail, gio_tail, hidden_tail)], messages
                assert len({row["pid"] for row in rows(proof)}) == 1
                assert len([row for row in rows(proof) if row["kind"] == "registered"]) == 1
                assert "SECRET_NOT_IN_LOG" not in log_path.read_text()
                command(binary, state, "quit")
                assert process.wait(timeout=10) == 0, log_path.read_text()
                report.update(ok=True, callbacks=messages, cold_hidden=True, positional_existing_handoff=True, gio_desktop_handoff=True, percent_u_suffix_preserved=True, quoted_state_path=True, active_foreground=True, hidden_preserves_window=True, one_host=True, one_plugin_process=True, unknown_plugin_error_redacted=True, active_unknown_error_visible=True, passive_unknown_stays_hidden=True)
            # A cold unknown callback must leave a recoverable app, including
            # explicit background startup even when the callback is active.
            for background in (False, True):
                cold_state = root / f"unknown-cold-{background}"
                ui_proof.unlink(missing_ok=True)
                arguments = [str(binary), "desktop", "--simulate", "--state", str(cold_state), "--uri", uri("/callback?code=SECRET_NOT_IN_LOG", "com.deckforge.missing")]
                if background:
                    arguments.append("--background")
                with log_path.open("a") as log:
                    process = subprocess.Popen(arguments, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                    status = wait(lambda: command(binary, cold_state, "status", required=False), "cold unknown callback retains app")
                    assert status["pid"] == process.pid and status["visible"] is not background
                    if background:
                        assert not status["window_open"] and status["editor_generation"] == 0
                    else:
                        rendered_error = wait(lambda: result(ui_proof), "cold callback visible error")
                        assert "not installed" in rendered_error["result"]["visible_status"]
                        assert "SECRET_NOT_IN_LOG" not in json.dumps(rendered_error)
                    command(binary, cold_state, "quit")
                    assert process.wait(timeout=10) == 0
            assert "SECRET_NOT_IN_LOG" not in log_path.read_text()
            report.update(cold_unknown_keeps_app=True, explicit_background_preserved=True)
        except Exception as error:
            report["ok"] = False
            report["error"] = str(error)
            raise
        finally:
            if process is not None:
                cleanup(process)
            if log_path.exists():
                report_path.with_suffix(".log").write_text(log_path.read_text())
            report_path.write_text(json.dumps(report, indent=2) + "\n")
    print("PASS: cold queued callback, positional and GIO warm handoff, active/hidden window policy, exact SDK suffix and redacted unknown-plugin error; no USB/network.")


if __name__ == "__main__":
    main()
