# leptoskit_analytics

See which application routes are failing or slow. The app keeps bounded minute aggregates and a small correlation buffer in memory. Connect the optional deployment service for durable fleet history, or use the per-process endpoint without an extra observability service. Unexported in-memory data resets on restart.

The dependency-free core works with a caller-provided Unix clock, including native hosts. The `server` feature adds Axum request instrumentation, a protected JSON evidence API, explicit backend timers, and a small browser module.

```rust,ignore
let (app, analytics) = leptoskit::analytics::server::mount(app, config)?;
// Serve app normally. config contains the resource identity, limits,
// public origin, browser route allowlist, and a server-only read key.
```

Enable `analytics` alongside `server` on the `leptoskit` dependency. The managed infra plugin configures the environment; `leptoskit::server::serve_listener_with` installs it when `LEPTOSKIT_ANALYTICS=1`. For a custom Axum entrypoint call `mount` explicitly, and avoid also enabling the automatic environment mount.

Query `GET /_leptoskit/analytics` with `Authorization: Bearer <read-key>` over HTTPS. The response includes release, environment, instance, window counts, fixed latency histograms, collection loss, bounded recent trace contexts, limitations, and advisory backend-error findings. It never authorizes a scale or deployment operation. Each replica has a separate snapshot; a request through a load balancer does **not** produce a fleet total.

The browser module is served at `/_leptoskit/analytics.mjs`:

```javascript
import { install } from "/_leptoskit/analytics.mjs";
// Install after your application's consent decision; supply route templates.
const analytics = install({ route: "/notes/{id}", enabled: userConsented });
// Use for selected same-origin API calls. Cross-origin calls get no context.
await analytics.tracedFetch("/api/list_notes");
// Call from the app's router. Never pass location.pathname or a user's ID.
analytics.pageView("/settings");
```

It measures page views, navigation duration, selected fetch duration/failure, and uncaught error counts. It honors Global Privacy Control and Do Not Track. It collects no persistent session/user ID, cookie, query string, DOM content, request/response body, error message, stack trace or arbitrary properties. Browser reports require the configured origin and an explicit route allowlist, have size and per-process batch limits, and remain untrusted browser data. Origin checks control browser access; they do not authenticate telemetry. Instrumented fetches reject redirects so a redirect cannot forward the correlation header to another origin. Use ordinary fetch when redirects are needed.

Use `server::Timer` to measure a database/RPC/job/live operation with an application-owned name. Handlers can extract `Analytics` and `TraceContext` from request extensions. Explicit `finish(Outcome::Ok)`/`ServerError` records an outcome; dropping a timer records cancellation. Database and RPC hooks are not installed automatically, and HTTP response timing excludes streaming bodies.

W3C version-00 `traceparent` is validated and continued with a fresh server span ID; only the trace ID is shared. No baggage or remote sampling decision is trusted. This provides request correlation; the read API has its own format version. The server also supports deployment-service export, OTLP/HTTP JSON metrics, and platform-dependent host samples. The connected service supplies durable fleet history and a dashboard. Pool metrics, Web Vitals, session replay, funnels and full trace export remain separate capabilities.

Validation: `cargo test -p leptoskit_analytics --all-features` and `deno run crates/leptoskit_analytics/tests/browser.mjs` through the repository's devenv shell. The browser test uses mocked browser APIs; it is not a real-browser performance test.

## Export to the deployment service

`lk console serve` hosts the shared journal, analytics collector and dashboard. Managed service releases bind `LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT`, the `leptoskit` protocol and a scoped write key automatically. Standalone servers can set those bindings explicitly. Export is bounded to one in-flight snapshot every 30 seconds. The app retains its normal memory limits and reports export health. Controller batches are deduplicated per producer/window; unexported data can be lost on a crash or outage longer than the retained window.

The `otlp` protocol exports completed histogram windows as OTLP/HTTP JSON delta metrics. It is a metrics adapter, not a full distributed tracer. Partial rejects count as export failures. Ambiguous retries can repeat a delta, so the receiving backend must deduplicate identical time-series timestamps.
