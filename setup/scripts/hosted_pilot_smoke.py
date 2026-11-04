#!/usr/bin/env python3
"""Probe our pilot's public routes without credentials or payment mutations."""
import argparse
import json
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file, code, message, headers, new_url):
        return None


def request(origin, path, method="GET", body=None, key=None):
    headers = {}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if key:
        headers["Authorization"] = f"Bearer {key}"
    query = urllib.request.Request(
        origin + path, data=body, headers=headers, method=method
    )
    client = urllib.request.build_opener(NoRedirect)
    try:
        response = client.open(query, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        value = response.read(2 * 1024 * 1024 + 1)
        if len(value) > 2 * 1024 * 1024:
            raise RuntimeError(f"{path}: response exceeded smoke-test limit")
        return response.status, response.headers, value


def probe(origin):
    url = urllib.parse.urlsplit(origin)
    if (url.scheme not in ("http", "https") or not url.hostname
            or url.username or url.password or url.path not in ("", "/")
            or url.query or url.fragment):
        raise RuntimeError("use an origin without credentials, path, query or fragment")
    if url.scheme != "https" and url.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise RuntimeError("HTTP smoke tests are limited to loopback")
    origin = origin.rstrip("/")
    checks = []
    for path, method, status, expected in [
        ("/healthz", "GET", 200, None),
        ("/", "GET", 200, b"One tree."),
        ("/console/", "GET", 200, b"bootstrap.js"),
        ("/docs/", "GET", 200, b"skribble.css"),
        ("/docs/guide/29-hosted-deployments.html", "GET", 200, b"Hosted"),
        ("/docs/_leptoskit/search.json", "GET", 200, None),
        ("/rive/rive.wasm", "HEAD", 200, None),
        ("/pkg/leptoskit_app_bg.wasm", "HEAD", 200, None),
        ("/docs/assets/fonts/skribble/SkribbleNotoSketch-Regular.woff2", "HEAD", 200, None),
        ("/docs/_leptoskit/generated/brand-mint.svg", "GET", 200, b"<svg"),
        ("/docs", "GET", 308, None),
        ("/console", "GET", 308, None),
        ("/docs/missing-pilot-probe.html", "GET", 404, None),
        ("/api/v1/account", "GET", 401, None),
        ("/api/v1/missing-pilot-probe", "GET", 401, None),
    ]:
        actual, headers, body = request(origin, path, method)
        if actual != status:
            raise RuntimeError(f"{method} {path}: expected {status}, got {actual}")
        if expected is not None and expected not in body:
            raise RuntimeError(f"{path}: missing expected content")
        if "script-src 'self'" not in headers.get("Content-Security-Policy", ""):
            raise RuntimeError(f"{path}: missing same-origin script policy")
        if status == 308 and headers.get("Location") != path + "/":
            raise RuntimeError(f"{path}: unexpected redirect")
        if path.endswith("search.json") and not json.loads(body):
            raise RuntimeError("documentation search is empty")
        if path.endswith(".wasm") and headers.get_content_type() != "application/wasm":
            raise RuntimeError(f"{path}: incorrect Wasm media type")
        checks.append(f"{method} {path}: {status}")
    status, _, _ = request(origin, "/api/v1/webhooks/stripe", "POST", b"{}")
    if status != 400:
        raise RuntimeError(f"unsigned Stripe event: expected 400, got {status}")
    checks.append("unsigned Stripe event rejected")
    return {"public_surface": "passed", "checks": checks,
            "cloud_worker_acceptance": "pending", "stripe_payment_acceptance": "pending"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("origin")
    arguments = parser.parse_args()
    print(json.dumps(probe(arguments.origin), indent=2))
