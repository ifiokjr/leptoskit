# LeptosKit HTTP

Reusable asynchronous HTTP for Rust hosts (including native mobile/desktop) and browser apps. Native requests use reqwest with rustls; browsers use fetch with incremental body reading. A Tokio runtime is required on native targets.

```rust,no_run
# async fn example() -> Result<(), leptoskit_plugin_http::Error> {
use leptoskit_plugin_http::{Client, Config, Request};
let client = Client::new(Config::new(["https://api.example.com"])? )?;
let response = client.execute(Request::get("https://api.example.com/data")).await?;
assert_eq!(response.status, 200);
# Ok(()) }
```

The host configures exact allowed origins, body/header limits, and a total request deadline. There is no wildcard or allow-all default. HTTP requires an explicit `http://` entry; credentials in URLs and non-HTTP schemes are rejected. Scopes cover every path on the selected origin. They are application policy, not an IP firewall or a defense against DNS rebinding by an allowed host.

Bodies stay binary, status codes including 4xx/5xx are returned normally, and response body bounds are checked per chunk even without Content-Length. Dropping the request future releases native in-flight work or calls AbortController in the browser. A timeout covers headers and streamed body consumption. Transport implementations can temporarily buffer a chunk beyond the retained body bound; the limit bounds the returned body, not every underlying OS/browser allocation.

Native clients admit at most four queued or running OS hostname lookups, shared by their clones. Cancellation and deadlines release the caller promptly, but each resolver keeps its admission until the blocking OS lookup finishes. Saturated hostname requests return `Backpressure` (`Busy` through the guest bridge); IP literals need no lookup. The custom resolver supplies addresses to reqwest without changing the request URL, Host header, TLS server name, certificate validation, or proxy policy.

Redirect following is bounded to **zero**. Native callers receive the 3xx status and Location header. Browser fetch rejects redirects, since its manual mode cannot reveal cross-origin redirect destinations before following them. A caller may explicitly request a new allowed destination. Native clients do not use ambient proxies, cookie jars, or automatic content decompression. Browser fetch omits ambient credentials, still enforces CORS, may decode content encodings, and exposes only permitted response headers. Browser-owned request headers are rejected explicitly rather than silently discarded. Native header values retain bytes; browser request header values require UTF-8.

Native guest apps use `guest::HttpClient::connect()` and its typed `execute(Request)` method. Add this crate with `default-features = false, features = ["guest"]`, select `"http"` in `native.plugins`, and configure `[native.plugin-options.http]` with `origins = ["https://api.example.com"]`. The CLI enables `async-host` only for the native build and automatically installs its validated factory. Browser apps retain the default `browser` feature.

Host options additionally accept `max_body_bytes` (default 4 MiB), `max_header_bytes` (32 KiB), and `timeout_ms` (30 seconds). The encoded reply, including at most 128 headers and a 4096-byte URL, must fit the bridge's 8 MiB payload bound. Missing scopes and unknown options fail creation. Requests execute on the shared asynchronous Rust runtime; cancellation and app destruction cancel network requests while any outstanding OS lookups retain their separate resolver admission. Revocation cancels queued/running calls, and regranting does not revive them. The binary RPC reports stable bridge errors; direct clients retain richer transport errors.

Verification uses loopback TCP servers for binary/status/header round trips, chunked oversize bodies, redirects, timeouts, and cancellation. The browser test fetches its local wasm-bindgen test server and exercises streamed body limits:

```sh
cargo test -p leptoskit_plugin_http --all-features
CHROMEDRIVER=/path/to/chromedriver cargo test -p leptoskit_plugin_http \
  --target wasm32-unknown-unknown --test browser
```

References: [Tauri HTTP plugin](https://v2.tauri.app/plugin/http/), [reqwest](https://docs.rs/reqwest/0.12.28/reqwest/), [Fetch API](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API), and [ReadableStream reader](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamDefaultReader/read).
