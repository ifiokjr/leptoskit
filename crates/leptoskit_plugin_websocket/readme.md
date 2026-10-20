# LeptosKit WebSocket

Scoped asynchronous WebSocket connections for native Rust hosts and browsers. Native connections use tokio-tungstenite with rustls; browser connections own a WebSocket and all its event callbacks. A Tokio runtime is required natively.

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

Origins, message sizes, pending bytes, queue item counts, and operation deadlines are explicit host configuration. URL userinfo, fragments, non-WS schemes, invalid or duplicate subprotocols, and invalid close codes/reasons are rejected before I/O. Plain `ws://` must be explicitly allowed. TLS validation is never disabled. Origins grant every path and do not function as an IP firewall. Native and browser handshakes reject HTTP redirects. Browsers enforce their own TLS, cookie, CSP, and HSTS policies.

Native receive calls read directly from the socket and apply transport backpressure; there is no background receiver queue. Browser events arrive independently, so receive queues have both byte and item limits (including empty messages). Overflow closes the socket and reports `Backpressure` instead of silently losing messages. Individual oversized messages report `TooLarge`. Browser APIs allocate incoming JavaScript messages before Rust receives them; these limits bound retained Rust queues, not browser-internal allocations.

Native sends await bounded buffer flushing with a deadline. Browser sends check `bufferedAmount` and return `Backpressure` if the next message would exceed the budget; callers can retry after it drains. Dropping a send future cannot retract bytes already sent. No automatic resend occurs. `receive()` returns None on orderly close and a transport error on abnormal closure. Drop releases native TCP/TLS or removes browser callbacks and initiates browser closure. Explicit `close()` waits for a handshake with a deadline. Browsers cannot forcibly destroy a TCP connection, so a timed-out browser close remains subject to browser policy.

These are direct APIs. Automatic native plugin registration and guest bridge handles are not wired yet; the synchronous bridge worker contract cannot express a cancellable async socket lifecycle. Browser custom handshake headers and native TLS/client-certificate customization are deliberately outside this API.

Run native loopback server tests with `cargo test -p leptoskit_plugin_websocket`. For browser integration in CI, the Rust runner starts an ephemeral loopback server, builds and runs the actual Chrome tests, propagates their exit status, and stops the server when finished:

```sh
CHROMEDRIVER=/path/to/chromedriver cargo run -p leptoskit_plugin_websocket \
  --example browser_test_server -- --test
```

For manual debugging, run the example without `--test`, then set its printed URL as `LEPTOSKIT_WEBSOCKET_TEST_URL` when building the `browser` test target with `--features browser-integration --target wasm32-unknown-unknown`.

The browser tests exercise binary echo and graceful close, oversized messages, and a flood of empty messages that must trip the queue item limit. All servers are local; tests do not contact external services.

References: [Tauri WebSocket plugin](https://v2.tauri.app/plugin/websocket/), [tokio-tungstenite](https://docs.rs/tokio-tungstenite/0.30.0/tokio_tungstenite/), and [browser bufferedAmount](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/bufferedAmount).
