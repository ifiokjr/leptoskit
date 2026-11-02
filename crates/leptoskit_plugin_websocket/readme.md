<h1 align="center">leptoskit_plugin_websocket</h1>

<p align="center">
  <strong>leptoskit_plugin_websocket</strong> is the leptoskit plugin for scoped WebSocket connections with bounded messages and explicit lifecycle.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_websocket"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_websocket"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_websocket.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_websocket"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_websocket/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Scoped asynchronous WebSocket connections for native Rust hosts and browsers. Native connections use tokio-tungstenite with rustls; browser connections own a WebSocket and all its event callbacks. A Tokio runtime is required natively.

Origins, message sizes, pending bytes, queue item counts, and operation deadlines are explicit host configuration. URL userinfo, fragments, non-WS schemes, invalid or duplicate subprotocols, and invalid close codes/reasons are rejected before I/O. Plain `ws://` must be explicitly allowed. TLS validation is never disabled. Origins grant every path and do not function as an IP firewall. Native and browser handshakes reject HTTP redirects. Browsers enforce their own TLS, cookie, CSP, and HSTS policies.

Native receive calls read directly from the socket and apply transport backpressure; there is no background receiver queue. Browser events arrive independently, so receive queues have both byte and item limits (including empty messages). Overflow closes the socket and reports `Backpressure` instead of silently losing messages. Individual oversized messages report `TooLarge`. Browser APIs allocate incoming JavaScript messages before Rust receives them; these limits bound retained Rust queues, not browser-internal allocations.

Native sends await bounded buffer flushing with a deadline. Browser sends check `bufferedAmount` and return `Backpressure` if the next message would exceed the budget; callers can retry after it drains. Dropping a send future cannot retract bytes already sent. No automatic resend occurs. `receive()` returns None on orderly close and a transport error on abnormal closure. Drop releases native TCP/TLS or removes browser callbacks and initiates browser closure. Explicit `close()` waits for a handshake with a deadline. Browsers cannot forcibly destroy a TCP connection, so a timed-out browser close remains subject to browser policy.

Native clients admit at most four queued or running OS hostname lookups, shared by their clones. Cancellation and connect deadlines release the caller promptly, but a resolver keeps its admission until the blocking OS lookup finishes; further hostname opens return `Backpressure` (`Busy` through the guest bridge) while saturated. IP literals do not need a resolver job. TCP uses the resolved addresses while the original URL remains authoritative for the handshake, TLS certificate validation, and SNI.

Native guest apps use `guest::WebSocketClient::connect()` then `open(url, protocols)`, returning a typed `GuestConnection` with `send`, `receive`, and `close` methods. Close borrows the handle so invalid arguments can be corrected and retried; operations after a completed close return `InvalidCall`. Add this crate with `default-features = false, features = ["guest"]`, select `"websocket"` in `native.plugins`, and configure `[native.plugin-options.websocket]` with exact `origins`. The CLI enables the native `async-host` factory automatically; browser apps retain the default `browser` feature.

Host options accept `max_connections` (default 8, maximum 128), `max_message_bytes` (1 MiB), `max_pending_bytes` (4 MiB across queued outgoing messages), and `timeout_ms` (10 seconds for connect/send). Each connection has at most 32 queued commands and one pending receive. A pending receive permits concurrent sends; canceling that receive removes its waiter. Canceling a partially written send closes the connection. Explicitly close guest connections when finished; app shutdown closes all remaining resources and waits for cleanup. Revoking the open grant also closes its committed connections.

An open reply owns a provisional resource lease until the guest successfully consumes it. Canceling an unread reply, rejecting an oversized response, or unloading the app releases that socket. Connection IDs are never reused. Binary RPC uses stable bridge errors; direct clients retain richer transport errors. Browser custom handshake headers and native TLS/client-certificate customization remain outside this API.

Run native loopback server tests with `cargo test -p leptoskit_plugin_websocket --all-features`. For browser integration in CI, the Rust runner starts an ephemeral loopback server, builds and runs the actual Chrome tests, propagates their exit status, and stops the server when finished:

```sh
CHROMEDRIVER=/path/to/chromedriver cargo run -p leptoskit_plugin_websocket \
  --example browser_test_server -- --test
```

For manual debugging, run the example without `--test`, then set its printed URL as `LEPTOSKIT_WEBSOCKET_TEST_URL` when building the `browser` test target with `--features browser-integration --target wasm32-unknown-unknown`.

The browser tests exercise binary echo and graceful close, oversized messages, and a flood of empty messages that must trip the queue item limit. All servers are local; tests do not contact external services.

References: [Tauri WebSocket plugin](https://v2.tauri.app/plugin/websocket/), [tokio-tungstenite](https://docs.rs/tokio-tungstenite/0.30.0/tokio_tungstenite/), and [browser bufferedAmount](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/bufferedAmount).

## How it fits

Add `leptoskit_plugin_websocket` as a direct dependency and select the `websocket` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native connections use tokio-tungstenite with rustls and need a Tokio runtime; browser connections own their socket and event callbacks. Every destination origin is explicit.

## Use it

```rust,no_run
# async fn example() -> Result<(), leptoskit_plugin_websocket::Error> {
use leptoskit_plugin_websocket::{Client, Config, Message};
let client = Client::new(Config::new(["wss://api.example.com"])? )?;
let mut connection = client.connect("wss://api.example.com/channel", &[]).await?;
connection.send(Message::Text("hello".into())).await?;
let message = connection.receive().await?;
connection.close(1000, "finished").await?;
# Ok(()) }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_websocket)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
