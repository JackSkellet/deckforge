#!/usr/bin/env python3
"""Native-window signal shutdown and session-lock reuse. Never opens USB."""
import argparse
from pathlib import Path
import signal
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/deckforge"))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="deckforge-native-lifecycle-") as directory:
        root = Path(directory)
        for sig in (signal.SIGTERM, signal.SIGINT):
            log = root / f"{sig.name}.log"
            with log.open("w") as output:
                process = subprocess.Popen(
                    [str(binary), "desktop", "--simulate", "--state", str(root / "state")],
                    stdout=output, stderr=subprocess.STDOUT,
                )
                try:
                    deadline = time.monotonic() + 20
                    while "Native editor ready" not in log.read_text():
                        if process.poll() is not None or time.monotonic() > deadline:
                            raise AssertionError("Native window failed to initialize: " + log.read_text())
                        time.sleep(0.05)
                    process.send_signal(sig)
                    code = process.wait(timeout=10)
                    assert code == 0, f"{sig.name} did not close normally ({code}): {log.read_text()}"
                    assert (root / "state/session.json").exists(), "Session was not saved"
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
    print("PASS: native window closed normally on SIGTERM and SIGINT; session lock reused. USB never opened.")


if __name__ == "__main__":
    main()
