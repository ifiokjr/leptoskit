"""Read-only readiness checks; never return container environment or credentials."""
import json
import subprocess
import shutil
import socket
import sys
import urllib.request

name = sys.argv[1]
services = sys.argv[2:]


def inspect(container):
    return json.loads(subprocess.check_output(["docker", "inspect", container], stderr=subprocess.DEVNULL))[0]


def accessory(kind):
    try:
        info = inspect(name + "-" + kind)
        if not info["State"]["Running"]:
            return False
        if kind == "cache":
            result = subprocess.check_output(["docker", "exec", name + "-cache", "sh", "-c",
                'VALKEYCLI_AUTH="$VALKEY_PASSWORD" valkey-cli ping'], stderr=subprocess.DEVNULL)
            return result.strip() == b"PONG"
        if kind == "relay":
            address = next(n["IPAddress"] for n in info["NetworkSettings"]["Networks"].values() if n.get("IPAddress"))
            with socket.create_connection((address, 4222), timeout=5) as connection:
                return connection.recv(4096).startswith(b"INFO ")
        return True
    except Exception:
        return False


ids = subprocess.check_output(["docker", "ps", "-q", "--filter", "label=service=" + name,
                               "--filter", "label=role=web"]).decode().split()
reports = []
for container in ids:
    info = inspect(container)
    port = next((value.split("=", 1)[1].rsplit(":", 1)[1] for value in info["Config"]["Env"]
                 if value.startswith("LEPTOS_SITE_ADDR=")), "3000")
    ready = False
    for network in info["NetworkSettings"]["Networks"].values():
        address = network.get("IPAddress")
        if not address:
            continue
        try:
            with urllib.request.urlopen("http://" + address + ":" + port + "/readyz", timeout=5) as response:
                ready = response.status == 200
        except Exception:
            pass
    reports.append({"container": info["Name"].lstrip("/"), "ready": ready})
checks = {service: accessory(service) for service in ["tunnel", *services]}
disk = shutil.disk_usage("/var/lib/leptoskit/" + name)
disk_used = disk.used / disk.total
print(json.dumps({"healthy": bool(reports) and all(r["ready"] for r in reports) and all(checks.values()) and disk_used < 0.9,
                  "application": reports, "services": checks, "disk_used_fraction": disk_used}))
