---
"release": minor
---

# Measure your deployed app with the analytics package

When a deployed route starts failing, you need its error counts, latency, and release in one place. Turn on the `analytics` feature of `leptoskit` on your server and add `[infra.plugins.analytics]` to `leptoskit.toml`:

```toml
[infra.plugins.analytics]
version = 1
environment = "production"
browser_routes = ["/", "/notes/{id}"]
```

The server then measures every HTTP request in bounded per-minute windows inside its own process, with W3C request correlation. `server::Timer` measures a named database, RPC, job, or live operation. An optional browser module records page views, navigation timing, and error counts for the routes you list, without cookies, query strings, or user identifiers, and keeps those reports apart from the server's own measurements. `lk infra analytics` reads one instance's measurements through a key-protected JSON API, and `lk infra plugins` lists what the plugin needs. It adds no containers or cloud resources.

To keep history across instances, connect the app to a deployment service, or set `otlp_endpoint` to export them to an OTLP/HTTP metrics backend.
