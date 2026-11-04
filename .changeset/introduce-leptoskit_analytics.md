---
leptoskit_analytics: major
---

# leptoskit_analytics: see which routes fail or slow down

`leptoskit_analytics` keeps bounded, in-memory request and browser measurements for a leptoskit app, so you can see which routes fail or slow down without running a separate observability service.

Enable `analytics` alongside `server` on the `leptoskit` dependency. The `server` feature instruments Axum, continues W3C `traceparent` correlation, and serves a protected JSON evidence API at `/_leptoskit/analytics` plus a same-origin browser module at `/_leptoskit/analytics.mjs`. The browser module honors Global Privacy Control and Do Not Track, and collects no user or session ID, cookie, query string, or page content. Each replica reports its own snapshot; connect the deployment service (`lk console serve`) for durable fleet history, or export OTLP/HTTP metrics.
