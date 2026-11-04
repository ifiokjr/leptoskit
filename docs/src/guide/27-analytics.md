# Application analytics

When a deployed route starts failing, you need its error counts, latency and release identity in one place. Enable the analytics package to collect bounded measurements inside the existing application process.

This example targets an Axum backend serving web and native clients. The server automatically measures HTTP requests. The optional browser module records web page views, navigation duration, selected request timing and error counts. Native hosts can submit explicit observations to the Rust collector; automatic native device instrumentation is not included.

## Enable the package

Add `analytics` to the server package's existing LeptosKit features:

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["server", "analytics"] }
```

For the managed deployment described in [Deploying a server](24-deploying.md), add this table alongside your existing infrastructure configuration:

```toml
[infra.plugins.analytics]
version = 1
environment = "production"
retained_windows = 15
max_series = 128
browser_routes = ["/", "/notes/{id}", "/settings"]
```

With the [deployment service](28-deployment-service.md), app credentials and the collector endpoint are bound automatically. For a direct deployment, supply `LEPTOSKIT_ANALYTICS_READ_KEY` through your deployment secret store. Use a random value of at least 32 printable non-space characters. It grants read access to analytics, so keep it on the server and with trusted agents.

`lk infra plugins` prints the capabilities, app features, bindings and secret names. Regenerate the managed configuration, review and apply the saved plan, then deploy a new app version with your usual release flow. The plugin adds environment settings to the app and its deployment workflow. It adds no new cloud resources or containers. Normal app memory and request costs still apply.

Servers using `leptoskit::server::serve_listener_with` (including the generated entrypoints) install the collector from those settings. A custom Axum entrypoint can instead call `leptoskit::analytics::server::mount` with a `ServerConfig`. Mount it once, after adding application routes, and avoid a second automatic environment mount. Backend code can use `server::Timer` for a named database, RPC, job or live operation; these hooks are explicit.

## Read measurements

With the read key available in your shell, run:

```sh
lk infra analytics
```

When connected to the deployment service, this returns durable measurements merged across reporting instances. Without that connection, it returns JSON from **one running instance**. It includes service, environment, release and instance identity, minute counts, latency histograms, collection loss, a small request-correlation buffer and advisory error findings. The underlying endpoint is `GET /_leptoskit/analytics` with a bearer read key.

Each instance has its own bounded memory buffer. A restart clears it, and a load-balanced request does not merge replicas. The default keeps 15 minute windows, at most 128 aggregate series, and 64 recent correlation records. Latency percentiles are histogram bucket bounds, not exact measurements. HTTP duration ends at response headers; streaming bodies and WebSocket lifetimes need explicit instrumentation.

The app API includes platform-dependent process/cgroup memory, cgroup CPU counters and filesystem capacity samples. Unsupported measurements are absent; database and relay capacity still need their own instrumentation. Its backend-error findings recommend investigation. They do not change infrastructure or prove that the app needs a larger server.

## Add browser measurements

The resource release identifies the server. Client bundle and code-push release identities are not collected in this version.

Load the module after your application's consent decision:

```javascript
import { install } from "/_leptoskit/analytics.mjs";

const analytics = install({
	route: "/notes/{id}",
	enabled: userConsented,
});

await analytics.tracedFetch("/api/list_notes");
analytics.pageView("/settings");
```

Call `pageView` from your router with its route template. The server accepts only the templates listed in `browser_routes`; an empty list disables browser ingestion. `tracedFetch` adds W3C request correlation for calls to the same origin. It does not patch global fetch or send tracing headers to other origins. Instrumented requests reject redirects; use ordinary fetch for calls that need them. `stop()` removes the listeners and discards queued observations.

The module respects Global Privacy Control and Do Not Track. It keeps no persistent user or session identifier and captures no cookies, query strings, DOM content, request bodies, exception messages or arbitrary properties. Error records are counts, not stack traces. Browser reports remain untrusted browser data, separate from backend measurements. Never pass raw URLs, user IDs or SQL as operation names.

## History and external export

The connected deployment service retains aggregate history in its shared SQLite store and displays fleet measurements in the dashboard. Repeated batches update each producer's contribution instead of adding it again. The app exports every 30 seconds with one in-flight batch. Unexported recent data can be lost on an app crash; a prolonged collector outage can exceed the app's retained window.

For an external OTLP/HTTP metrics backend, set `infra.plugins.analytics.otlp_endpoint` to its metrics endpoint. If the backend requires a bearer credential, supply `LEPTOSKIT_ANALYTICS_WRITE_KEY` through the worker's secret store. This selects the external sink in place of the deployment service's collector. The adapter exports completed delta histograms; the backend must deduplicate repeated timestamps after an ambiguous transport failure. The app API reports export failures.

Automatic DB/RPC hooks, Web Vitals, complete distributed traces, session replay, funnels and automatic infrastructure changes are separate capabilities. Current request correlation and explicit timers remain available.
