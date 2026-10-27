# Security and scale

This chapter is about the moment a leptoskit server stops being `leptoskit dev` on your laptop and starts answering strangers. It covers what the toolkit already does for you, what you have to add before you expose a server, and how to serve a lot of traffic without a lot of servers. If you are preparing a deployment, you can work straight through the [pre-launch checklist](#pre-launch-checklist) at the end; the rest of the chapter explains each item.

## What faces the internet

A leptoskit project can put up to three things on the network:

| Piece                  | How it runs                                                                    | What it does                                                             | What it costs under load                                     |
| ---------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------ | ------------------------------------------------------------ |
| The static site        | `leptoskit serve` with `app.render = "csr"`                                    | Serves `dist/`: `index.html`, the wasm, the assets.                      | Disk reads and bandwidth. It runs none of your code.         |
| Your app server        | `leptoskit serve` with `"ssr"` or `"islands"`, which runs the binary you built | Renders pages, answers server functions, and serves `dist/` behind them. | CPU per page, and it reads request bodies.                   |
| The code push registry | `leptoskit codepush serve`, or a bucket behind a CDN                           | Serves signed release files to devices.                                  | Disk reads and bandwidth, plus a hash on two dynamic routes. |

`leptoskit dev` is left off the list on purpose. It is a build loop with a server attached: unoptimised code, a file watcher, and a restart on every change. Keep it on `127.0.0.1`.

## What you get without doing anything

**Servers bind to loopback.** `serve.address` defaults to `127.0.0.1`, so a freshly built project is reachable only from the machine it runs on. Exposing it is a choice you make in `leptoskit.toml`, not a default you have to undo.

**Static files cannot be used to read other files.** `tower-http`'s `ServeDir` serves `dist/` and refuses paths that would leave it. The SPA fallback reads one fixed file, `index.html`, and only for `GET` and `HEAD` requests whose last path segment has no extension (see [`leptoskit serve`](04-building.md#leptoskit-serve)). Every response carries `Vary: Accept-Encoding`, so a shared cache never hands brotli bytes to a client that did not ask for them.

**Compression happens at build time.** A release build writes `.br` and `.gz` copies next to every compressible file, and the server picks the one the browser accepts. Nothing compresses on the request path, so a burst of traffic costs the origin disk reads, not CPU.

**The registry holds no secrets and makes no decisions.** The code push server serves files. The signing key is needed only by `leptoskit codepush release`, on the machine or CI runner that releases, and devices verify every release against the public keys built into the app shell. Someone who takes over the registry can withhold updates; they cannot ship code or roll a device back. The checks a device makes are listed under [what the client checks](05-code-push.md#what-the-client-checks).

**Registry paths cannot escape the store.** The app and channel names in a URL are reduced to `[A-Za-z0-9_-]` before they are joined onto the store directory, a release directory has to be exactly `<version>+<build>`, and a delta is named by a 64-character lowercase hex hash or it is a 404. A `..` anywhere in the path is a 404.

**No registry request grows with the channel's history.** The static routes and the older `check`, `download`, and `delta` endpoints all answer from `index.json` plus at most two more files. No request lists or parses a channel's releases, so a channel with a thousand releases is as cheap to serve as one with two. The static routes and `download` only read a file. `check` and `delta` also hash one container, the device's own, so their cost grows with the size of your bundle; [step 5](#5-rate-limit-what-is-left) covers them.

**Devices defend themselves too.** The built-in `HttpTransport` requires HTTPS, follows no redirects, caps every body at 32 MiB, and times out: 15 seconds to connect, 30 for the response headers, 10 minutes for the whole request. A container over the client's size limit is refused whether it arrives whole or is rebuilt from a delta, a container is parsed with every length checked against the bytes that arrived, every file has to decompress to exactly its declared size, and a delta's output size is checked before anything is allocated. A hostile registry can waste a device's bandwidth, not its memory.

**Errors do not describe the server.** A storage failure is a `500` with a fixed message. Filesystem paths and file contents stay on the server. When you mount `codepush_router` in your own server, the cause of each failure is logged through `tracing` at the error level, so install a subscriber to see it; `leptoskit codepush serve` does not print it yet.

**Dependencies are audited.** `cargo deny` runs in CI over advisories, licenses, bans, and sources, with the policy in `deny.toml`, and `cargo clippy -D warnings` runs over every target.

## What you have to add

A Rust server built on hyper handles malformed requests and oversized headers for you. It does not decide how long a client may take, how many connections one address may hold open, or who may call which route. Those are policy, and they belong to you.

### 1. Put something in front of it

Terminate TLS, absorb floods, and enforce connection limits at a layer built for it: a CDN, your platform's load balancer, or a reverse proxy such as Caddy or nginx on the same host. Any of them will:

- speak HTTPS to the world and plain HTTP to your server on a private address;
- close idle and slow connections, which is the defence against attacks that hold a socket open by sending one byte at a time;
- cap concurrent connections and request rates per client address;
- cache everything that carries a `Cache-Control` header, which for a leptoskit deployment is nearly everything.

Set `serve.address` to the address the proxy reaches: `127.0.0.1` when they share a host, `0.0.0.0` or `::` inside a container. The server's port is never exposed directly.

The nginx version of this, for an app server on port 3000:

```nginx
limit_req_zone  $binary_remote_addr zone=app:10m rate=30r/s;
limit_conn_zone $binary_remote_addr zone=conn:10m;

server {
    listen 443 ssl;
    http2 on;
    client_header_timeout 10s;
    client_body_timeout   10s;
    send_timeout          30s;
    client_max_body_size  1m;

    location / {
        limit_req  zone=app burst=60 nodelay;
        limit_conn conn 50;
        proxy_pass http://127.0.0.1:3000;
    }
}
```

### 2. Bound every request your own server accepts

The static server and the registry only read files. The layers below matter for an `ssr` or `islands` app, where every page runs your code and every server function reads a body. Add them to the router your server binary builds. The islands example ships with them, so `router()` and `bounded()` in `examples/islands/src/server.rs` are the reference; this is the shape of it:

```rust
use std::time::Duration;

use leptos::prelude::*;
use leptoskit::server::axum::BoxError;
use leptoskit::server::axum::Router;
use leptoskit::server::axum::error_handling::HandleErrorLayer;
use leptoskit::server::axum::http::StatusCode;
use leptoskit::server::axum::routing::get;
use leptoskit::server::leptos_axum::LeptosRoutes;
use leptoskit::server::leptos_axum::generate_route_list;
use leptoskit::server::tower::ServiceBuilder;
use leptoskit::server::tower::limit::GlobalConcurrencyLimitLayer;
use leptoskit::server::tower_http::limit::RequestBodyLimitLayer;
use leptoskit::server::tower_http::timeout::TimeoutLayer;

pub fn router(options: LeptosOptions) -> Router {
	let site_root = std::path::PathBuf::from(options.site_root.as_ref());
	let routes = generate_route_list(App);
	Router::new()
		// The load balancer's probe: no render, just `ok`.
		.route("/_leptoskit/health", get(|| async { "ok" }))
		.leptos_routes(&options, routes, {
			let options = options.clone();
			move || shell(options.clone())
		})
		.fallback_service(leptoskit::server::static_files(&site_root))
		.with_state(options)
		// A render or server function that has not answered by now is a 408,
		// not a task pinned until the client gives up.
		.layer(TimeoutLayer::with_status_code(
			StatusCode::REQUEST_TIMEOUT,
			Duration::from_secs(10),
		))
		// Caps the body stream itself, so it holds for leptos server
		// functions, which read the body without axum's extractors. A body
		// declared larger than this is a 413 before a byte of it is read.
		.layer(RequestBodyLimitLayer::new(1024 * 1024))
		.layer(
			ServiceBuilder::new()
				.layer(HandleErrorLayer::new(|_: BoxError| async {
					StatusCode::SERVICE_UNAVAILABLE
				}))
				// Past this many requests in flight, answer 503 at once rather
				// than queue work the server cannot finish.
				.load_shed()
				.layer(GlobalConcurrencyLimitLayer::new(512)),
		)
}
```

Every layer comes through `leptoskit::server`, which re-exports `tower` and `tower-http` with the `limit`, `load-shed`, and `timeout` features already on (see [one dependency for the server](04-building.md#one-dependency-for-the-server)), so this needs nothing added to `Cargo.toml` beyond `leptoskit` with its `server` feature.

Two details are easy to get wrong. `axum`'s own `DefaultBodyLimit` applies only to axum's extractors; `RequestBodyLimitLayer` wraps the body, which is why it is the one to use with leptos. And axum applies a router layer to each route separately, so `ServiceBuilder::concurrency_limit` would hand every route its own 512 permits; `GlobalConcurrencyLimitLayer` shares one semaphore across all of them, which is what "requests in flight" means for a server. The islands example has a test that fills every slot and watches the next request get its 503.

Take the numbers from measurements, not from this page: the timeout a little above your slowest legitimate page, the body limit a little above your largest legitimate server function argument, and the concurrency limit from a load test on the instance size you deploy.

### 3. Shut down cleanly

In `csr` mode, `leptoskit serve` finishes the requests in flight when it receives `SIGINT` or `SIGTERM`. In `ssr` and `islands` mode the server is yours, and your orchestrator relies on the same behaviour to replace instances without dropping requests. Containers stop a process with `SIGTERM`, so a binary that waits on Ctrl-C alone cuts off whatever was in flight on every deploy. `leptoskit::server::serve_listener` (and `serve_router`, which binds the address for you) waits for both and drains before it returns; the islands example's `main.rs` is the whole of it:

```rust
use leptoskit::server::tokio;

#[tokio::main]
async fn main() {
	let options = get_configuration(None)
		.expect("leptoskit sets the LEPTOS_* environment")
		.leptos_options;
	let address = options.site_addr;
	let listener = tokio::net::TcpListener::bind(address)
		.await
		.expect("bind the server address");
	println!("listening on http://{address}");
	leptoskit::server::serve_listener(listener, router(options))
		.await
		.expect("serve");
}
```

The health route from [step 2](#2-bound-every-request-your-own-server-accepts) completes the picture: the load balancer stops sending traffic to an instance that fails its probe, and the instance finishes what it has before it exits.

### 4. Keep the registry read-only and keyless

Run `leptoskit codepush serve` as a user that can read `releases/` and write nothing. The server never writes the store; `leptoskit codepush release` does, from your release machine or CI. The signing key does not belong on the serving host, and `LEPTOSKIT_CODEPUSH_SIGNING_KEY` should not be set in its environment. Better still, do not run a registry server: devices only ever `GET` static files, so a bucket behind a CDN is the whole registry (see [code push with no server](05-code-push.md#code-push-with-no-server)).

### 5. Rate-limit what is left

After the steps above, the routes that do more than read a file are the registry's dynamic `check` and `delta`: each reads the container the client says it runs and hashes it, `check` to learn whether a delta applies and `delta` to find it. That is bounded by your bundle size and takes a few milliseconds for a few megabytes, but it is work an anonymous caller can request as often as they like, so give both a per-address rate limit at the proxy (`limit_req` above). The current client never calls the dynamic routes, so if nothing older is in the field you can block `check`, `download`, and `delta` at the proxy outright.

## Scaling

One rule drives everything below: **keep the origin boring.** A leptoskit server either serves immutable files or renders stateless pages, so every scaling step is either "cache it" or "run more of it."

### Client-rendered apps: the CDN is the server

`counter`, `file_routes`, and `gallery` are `csr` apps, so `dist/` is the whole deployment. Put a CDN in front of `leptoskit serve`, or upload `dist/` to a bucket and skip the server, and the edge absorbs any load you can attract while the origin sees only cache misses. The server sets no `Cache-Control` of its own; set it at the proxy or on the bucket. `ServeDir` sends `Last-Modified` and answers conditional requests, so a short `max-age` with revalidation keeps `pkg/<client>_bg.wasm` fresh across deploys while the edge still serves it. Give `index.html` a few seconds at most, since it is what picks up a new release.

### Server-rendered apps: stateless, so run more

An `ssr` or `islands` server keeps nothing between requests; leptoskit adds no sessions or server-side state of its own. Run as many replicas as you need behind the load balancer and scale on CPU, because rendering is CPU-bound. Build with `--release`, serve `dist/` from the CDN so replicas spend their CPU on pages, and add the layers from [step 2](#2-bound-every-request-your-own-server-accepts) so a slow dependency degrades one request rather than the whole instance.

`leptoskit serve` sets the `LEPTOS_*` environment for the binary from `leptoskit.toml`. A container entry point does the same itself: `LEPTOS_SITE_ADDR=0.0.0.0:3000`, `LEPTOS_SITE_ROOT=dist`, `LEPTOS_OUTPUT_NAME=<client library name>`, and `LEPTOS_ENV=PROD`.

### The code push registry: publish to a bucket

The static layout is designed so that a bucket behind a CDN serves it with nothing running: release files are immutable and cached for a year, and `index.json` for sixty seconds (see [hosting](05-code-push.md#hosting)). Ten thousand devices checking at once is ten thousand cache hits.

If you run `leptoskit codepush serve` instead, this is what one request costs the origin:

| Route                               | Reads                                                                                                                    |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `index.json`                        | One small file. Sent with `Cache-Control: no-cache` and no `ETag` or `Last-Modified`, so every check downloads it again. |
| `release.lkb`, `release.sig`, delta | One file, served with `immutable` caching.                                                                               |
| `check`                             | `index.json`, a persisted container hash, and a delta existence check.                                                   |
| `download`                          | One file.                                                                                                                |
| `delta`                             | `index.json`, a persisted base hash, and one streamed delta file.                                                        |

File responses stream in 64 KiB chunks and support Range and HEAD requests. The router admits at most 16 active responses, including downloads still in progress, and returns 503 for excess requests without queuing them. Metadata reads run off the async workers. Keep a proxy or CDN in front of a public origin to limit request rates and close slow connections.

New ingests and imports persist the container hash. Older stores without this metadata continue to offer full downloads through the dynamic routes; importing the same release again restores delta offers without changing its bytes.

Nothing depends on how many releases the channel has. The store grows with every release, since each adds a container, a signature, and, when it saves enough, a delta. Deleting an old release directory is safe as long as it is not the one `index.json` names: a device that ran the deleted release asks for its delta, gets a 404, and downloads the full container instead.

### Measure before you launch

Point a load generator at the staging deployment, not at `leptoskit dev`: `GET /`, `GET /pkg/<client>_bg.wasm`, `GET /api/codepush/<app>/<channel>/index.json`, and a `POST` to a server function. Watch the p99 latency and the server's memory. The limits in [step 2](#2-bound-every-request-your-own-server-accepts) should come from these numbers, and a timeout, body cap, or concurrency limit that never trips in the test is probably too loose to protect you in production.

## Pre-launch checklist

- `serve.address` is `127.0.0.1`, or a private or container address behind TLS termination. The server's port is not reachable from the internet.
- A proxy or CDN terminates TLS, closes idle and slow connections, and limits connections and request rate per client address.
- `ssr` and `islands` only: the router has a `TimeoutLayer`, a `RequestBodyLimitLayer`, and `load_shed` with a `GlobalConcurrencyLimitLayer`, with numbers from a load test.
- `ssr` and `islands` only: the binary has a health route and stops on `SIGTERM` by finishing requests in flight.
- Static assets and release files are cached at the edge; `index.html` and `index.json` revalidate.
- The code push registry is a bucket, or a server that runs read-only with no signing key in its environment.
- The registry's dynamic routes are rate-limited, or blocked when no older client needs them.
- Every device transport requires HTTPS, caps the body, and times out. The built-in one does all three.
- `cargo deny check` and `cargo clippy --all-targets -- -D warnings` are green.
- `leptoskit dev` is not the production server.
