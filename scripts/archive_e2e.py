#!/usr/bin/env python3
"""Qualify the producer against a DashQ main checkout, entirely under a new temporary root."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dashq", type=Path, required=True, help="isolated DashQ main checkout")
    args = parser.parse_args()
    # DashQ's existing listener is fixed. Refuse to touch a process already using it.
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.1", 18882))
        except OSError:
            raise SystemExit("127.0.0.1:18882 is in use; no process was changed")
    repo = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.dashq, text=True).strip()
    subprocess.run(["cargo", "build", "--locked"], cwd=args.dashq, check=True)
    with tempfile.TemporaryDirectory(prefix="agentq-archive-sink-") as temp:
        root = Path(temp)
        root.chmod(0o700)
        config = root / "config.json"
        config.write_text(json.dumps({"public_host": "fixture.invalid", "archive": {"root": str(root / "archive"), "proxy_token": "ab" * 32}}))
        config.chmod(0o600)
        env = dict(os.environ, DASHQ_CONFIG=str(config), AGENTQ_SELFTEST_OFFLINE="1", AGENTQ_ARCHIVE_TEST_ADDR="127.0.0.1:18882", AGENTQ_ARCHIVE_TEST_CONTROL=str(root))
        binary = args.dashq.resolve() / "target/debug/dashq"
        subprocess.run([str(binary), "admin", "fixture", "fixture-owner"], env=env, check=True)
        with (root / "sink.log").open("wb") as log:
            def start():
                candidate = subprocess.Popen([str(binary), "serve"], env=env, stdout=log, stderr=log)
                deadline = time.monotonic() + 30  # Fixture startup check, not a product limit.
                while True:
                    if candidate.poll() is not None:
                        raise RuntimeError("temporary DashQ exited before listening")
                    try:
                        with socket.create_connection(("127.0.0.1", 18882), timeout=1):
                            return candidate
                    except OSError:
                        if time.monotonic() > deadline:
                            candidate.terminate()
                            candidate.wait(timeout=15)
                            raise RuntimeError("temporary DashQ did not listen")
                        time.sleep(0.05)

            process = start()
            test = None
            try:
                test = subprocess.Popen(["cargo", "test", "--locked", "-p", "agentq-runtime", "--lib", "archive::store::local_sink::dashq_main_wire_and_recovery", "--", "--ignored", "--nocapture"], cwd=repo, env=env)
                while test.poll() is None:
                    if (root / "offline.request").exists() and not (root / "offline.ready").exists():
                        process.terminate()
                        process.wait(timeout=15)
                        (root / "offline.ready").touch()
                    if (root / "resume.request").exists() and not (root / "resume.ready").exists():
                        process = start()
                        (root / "resume.ready").touch()
                    time.sleep(0.05)
                if test.returncode:
                    raise subprocess.CalledProcessError(test.returncode, test.args)
                print(f"DashQ main sink revision: {revision}")
            finally:
                if test is not None and test.poll() is None:
                    test.terminate()
                    test.wait(timeout=15)
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=15)


if __name__ == "__main__":
    main()
