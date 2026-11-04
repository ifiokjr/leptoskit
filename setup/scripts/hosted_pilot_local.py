#!/usr/bin/env python3
"""Exercise a packaged broker's HTTP surface, restart, backup and restore."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

from hosted_pilot_smoke import probe, request


@contextlib.contextmanager
def serve(binary, site, data, environment):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    origin = f"http://127.0.0.1:{port}"
    process = subprocess.Popen(
        [str(binary), "--data", str(data), "--url", origin, "serve",
         "--assets", str(site), "--bind", f"127.0.0.1:{port}"],
        env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.monotonic() + 30
        while True:
            if process.poll() is not None:
                raise RuntimeError(f"broker exited during startup: {process.returncode}")
            try:
                if request(origin, "/healthz")[0] == 200:
                    break
            except OSError:
                pass
            if time.monotonic() > deadline:
                raise RuntimeError("broker did not become healthy")
            time.sleep(0.1)
        yield origin
    finally:
        if process.poll() is None:
            process.terminate()
        try:
            code = process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise RuntimeError("broker did not drain within 30 seconds") from None
        if code != 0:
            raise RuntimeError(f"broker did not shut down cleanly: {code}")


def run(binary, data, environment, *arguments):
    result = subprocess.run(
        [str(binary), "--data", str(data), *arguments], env=environment,
        capture_output=True, text=True, timeout=30,
    )
    if result.returncode:
        raise RuntimeError(f"operator command {arguments[0]} failed: {result.returncode}")
    return result.stdout.strip()


def check(binary, site):
    # Override inherited keys: this local drill must not contact a real provider.
    environment = dict(os.environ, LEPTOSKIT_HOSTED_KEY="test-only-pilot-drill-key-0123456789",
                       STRIPE_SECRET_KEY="test-only-unused-provider-key",
                       STRIPE_WEBHOOK_SECRET="test-only-unused-webhook-key",
                       STRIPE_CONTROL_PRICE="price_pilot_control", STRIPE_TEAM_PRICE="price_pilot_team")
    with tempfile.TemporaryDirectory(prefix="leptoskit-pilot-") as temporary:
        root = Path(temporary)
        data = root / "data"
        key = run(binary, data, environment, "account", "--id", "pilot", "--customer", "cus_pilot_drill")
        with serve(binary, site, data, environment) as origin:
            report = probe(origin)
            status, _, value = request(origin, "/api/v1/account", key=key)
            if status != 200 or json.loads(value)["status"] != "inactive":
                raise RuntimeError("enrolled account did not start inactive")
            status, _, _ = request(origin, "/api/v1/installations", "POST",
                                   b'{"project":"pilot","environment":"acceptance","ownership":"customer"}', key)
            if status != 403:
                raise RuntimeError("inactive account was permitted to create an installation")
        with serve(binary, site, data, environment) as origin:
            if request(origin, "/api/v1/account", key=key)[0] != 200:
                raise RuntimeError("account did not survive a broker restart")
        backup = root / "backup"
        restored = root / "restored"
        run(binary, data, environment, "backup", "--out", str(backup))
        run(binary, restored, environment, "restore", "--from", str(backup))
        with serve(binary, site, restored, environment) as origin:
            if request(origin, "/api/v1/account", key=key)[0] != 401:
                raise RuntimeError("restore did not revoke the old account key")
        report["local_recovery"] = "restart, offline backup, restore and key revocation passed"
        report["graceful_shutdown"] = "passed"
        return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--site", type=Path, required=True)
    arguments = parser.parse_args()
    print(json.dumps(check(arguments.binary.resolve(), arguments.site.resolve()), indent=2))
