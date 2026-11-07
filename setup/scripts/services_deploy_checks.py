#!/usr/bin/env python3
"""Check a deployed copy of examples/analytics through its edge.

The deployment runs the analytics app, its analytics service, and the echo
service from setup/services_echo, and `--base` is the one address the
outside reaches it at: the Compose edge, or the Kubernetes Ingress. Every
check goes through that address only, the way a browser would.

Target-specific checks (unpublished service ports, Postgres roles) are in
the shell scripts that start each deployment.
"""
import argparse
import base64
import json
import os
import re
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

ANALYTICS = "/_leptoskit/services/analytics"
ECHO = "/_leptoskit/services/echo"
# What the echo service answers on any path it doesn't serve.
ECHO_FALLBACK = "echo-fallback"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file, code, message, headers, new_url):
        return None


def request(base, path, method="GET", body=None, headers=None):
    headers = dict(headers or {})
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    query = urllib.request.Request(base + path, data=data, headers=headers, method=method)
    client = urllib.request.build_opener(NoRedirect)
    try:
        response = client.open(query, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read().decode("utf-8", "replace")


class Checks:
    def __init__(self):
        self.passed = []

    def check(self, name, condition, detail=""):
        if not condition:
            raise SystemExit(f"FAIL {name}{': ' + detail if detail else ''}")
        print(f"PASS {name}", flush=True)
        self.passed.append(name)


def rendered_total(html):
    """The event total the server rendered into the page."""
    # Out-of-order streaming sends the fallback first and the resolved
    # summary later in the same body, between hydration markers.
    match = re.search(r"Events: (?:<!--[^>]*-->|<!>|\s)*<strong>(?:<!--[^>]*-->|<!>|\s)*(\d+)", html)
    return int(match.group(1)) if match else None


def summary_total(base):
    status, _, body = request(base, f"{ANALYTICS}/api/summary", "POST", {"days": 7})
    if status != 200:
        raise SystemExit(f"FAIL summary answered {status}: {body}")
    return json.loads(body)["total"]


def upgrade(base, path, origin):
    """The status of a WebSocket upgrade, read from the raw response."""
    url = urllib.parse.urlsplit(base)
    port = url.port or 80
    key = base64.b64encode(os.urandom(16)).decode()
    lines = [
        f"GET {path} HTTP/1.1",
        f"Host: {url.netloc}",
        "Connection: Upgrade",
        "Upgrade: websocket",
        "Sec-WebSocket-Version: 13",
        f"Sec-WebSocket-Key: {key}",
        f"Origin: {origin}",
    ]
    with socket.create_connection((url.hostname, port), timeout=20) as connection:
        connection.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        head = b""
        while b"\r\n" not in head:
            chunk = connection.recv(1024)
            if not chunk:
                break
            head += chunk
    status = head.split(b"\r\n", 1)[0].decode("latin-1")
    parts = status.split(" ")
    return int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else None


def wait_until_ready(base, deadline):
    while True:
        try:
            status, _, _ = request(base, "/readyz")
            if status == 200:
                return
        except OSError:
            pass
        if time.monotonic() > deadline:
            raise SystemExit(f"FAIL {base}/readyz never answered 200")
        time.sleep(2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="the edge, such as http://127.0.0.1:3000")
    parser.add_argument("--origin", required=True, help="LEPTOSKIT_PUBLIC_ORIGIN of the services")
    parser.add_argument(
        "--echo-trust",
        required=True,
        choices=["isolated", "first-party"],
        help="the echo service's trust: whether the edge strips its cookies",
    )
    arguments = parser.parse_args()
    base = arguments.base.rstrip("/")
    checks = Checks()
    wait_until_ready(base, time.monotonic() + 120)

    # The app's own probes answer through the edge.
    for probe in ("/healthz", "/readyz"):
        status, _, _ = request(base, probe)
        checks.check(f"the app's {probe} answers 200 through the edge", status == 200, str(status))

    # The page renders on the app's server, which asks the service for the
    # summary at its private address.
    status, _, html = request(base, "/")
    checks.check("the page renders", status == 200 and "Events, counted somewhere else." in html, str(status))
    checks.check("the page has no service error", 'class="problem"' not in html, html[-2000:])
    before = rendered_total(html)
    checks.check("the server rendered the service's count", before is not None, html[-2000:])
    checks.check(
        "the rendered count matches the service's summary",
        before == summary_total(base),
        f"rendered {before}",
    )

    # A browser's capture goes to the service through its public prefix,
    # and the next render shows it.
    status, _, body = request(
        base,
        f"{ANALYTICS}/api/capture",
        "POST",
        {"event": "clicked_buy", "distinct_id": "deploy-check", "properties": {"page": "/"}},
    )
    checks.check("a capture through the public prefix succeeds", status == 200, f"{status} {body}")
    _, _, html = request(base, "/")
    checks.check(
        "the next render counts the capture",
        rendered_total(html) == before + 1,
        f"before {before}, after {rendered_total(html)}",
    )
    checks.check("the service's summary counts it", summary_total(base) == before + 1)

    # Public paths are the internet: the app-only operation refuses them.
    status, _, _ = request(base, f"{ANALYTICS}/api/recent", "POST", {"limit": 10})
    checks.check("an app-only operation refuses a caller without the token", status in (401, 403), str(status))
    status, _, _ = request(
        base,
        f"{ANALYTICS}/api/recent",
        "POST",
        {"limit": 10},
        {"Authorization": "Bearer not-the-token"},
    )
    checks.check("an app-only operation refuses a wrong token", status in (401, 403), str(status))

    # The service's live socket upgrades through the edge, for the public
    # origin only.
    status = upgrade(base, f"{ANALYTICS}/live", arguments.origin)
    checks.check("the service's live socket upgrades (101) through the edge", status == 101, str(status))
    status = upgrade(base, f"{ANALYTICS}/live", "https://elsewhere.example")
    checks.check("the live socket refuses a foreign origin", status == 403, str(status))

    # Only the public prefixes reach a service: everything else is the app's.
    # The echo service answers every path it doesn't serve with a marker, so
    # a path the edge sends it is told apart from one it sends to the app.
    status, _, body = request(base, f"{ECHO}/no-such-path")
    checks.check("a path under the echo prefix reaches the echo service", ECHO_FALLBACK in body, f"{status} {body}")
    for path in ("/headers", "/_leptoskit/services/echox/headers", "/_leptoskit/services/echo-headers", "/_leptoskit/services"):
        status, _, body = request(base, path)
        checks.check(f"{path} goes to the app, not the echo service", ECHO_FALLBACK not in body and "cookie:" not in body, f"{status} {body}")
    status, _, body = request(base, "/_leptoskit/services/analyticsx/api/summary", "POST", {"days": 7})
    checks.check("a near miss of the analytics prefix isn't the service's", '"total"' not in body, f"{status} {body}")

    # Cookies on the echo service's public paths.
    status, headers, body = request(base, f"{ECHO}/headers", headers={"Cookie": "session=the-apps-session"})
    checks.check("the echo service answers through its prefix", status == 200, f"{status} {body}")
    set_cookies = headers.get_all("Set-Cookie") or []
    if arguments.echo_trust == "isolated":
        checks.check("the edge strips Cookie from an isolated service's requests", body == "cookie: -\n", body)
        checks.check("the edge strips Set-Cookie from an isolated service's answers", not set_cookies, str(set_cookies))
    else:
        checks.check(
            "a first-party service receives the app's cookies",
            body == "cookie: session=the-apps-session\n",
            body,
        )
        checks.check("a first-party service may set cookies", bool(set_cookies), str(headers))

    # The service's volume is writable, and it reaches the app privately.
    status, _, body = request(base, f"{ECHO}/volume")
    checks.check("the echo service writes to its volume", status == 200, f"{status} {body}")
    status, _, body = request(base, f"{ECHO}/app")
    checks.check("a service reaches the app at LEPTOSKIT_APP_URL", status == 200, f"{status} {body}")

    print(json.dumps({"passed": len(checks.passed)}))


if __name__ == "__main__":
    sys.exit(main())
