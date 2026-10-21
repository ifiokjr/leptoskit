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

Redirect following is bounded to **zero**. Native callers receive the 3xx status and Location header. Browser fetch rejects redirects, since its manual mode cannot reveal cross-origin redirect destinations before following them. A caller may explicitly request a new allowed destination. Native clients do not use ambient proxies, cookie jars, or automatic content decompression. Browser fetch omits ambient credentials, still enforces CORS, may decode content encodings, and exposes only permitted response headers. Browser-owned request headers are rejected explicitly rather than silently discarded. Native header values retain bytes; browser request header values require UTF-8.

These are direct asynchronous APIs. No automatic native plugin registration or guest RPC is claimed yet. The current bridge worker contract runs synchronous `Plugin::invoke`; wrapping an async request there would lose cancellation and would need an application-owned runtime. A future guest adapter must carry host-owned destination scopes and cancel actual work when its guest unloads.

Verification uses loopback TCP servers for binary/status/header round trips, chunked oversize bodies, redirects, timeouts, and cancellation. The browser test fetches its local wasm-bindgen test server and exercises streamed body limits:

```sh
cargo test -p leptoskit_plugin_http
CHROMEDRIVER=/path/to/chromedriver cargo test -p leptoskit_plugin_http \
  --target wasm32-unknown-unknown --test browser
```

References: [Tauri HTTP plugin](https://v2.tauri.app/plugin/http/), [reqwest](https://docs.rs/reqwest/0.12.28/reqwest/), [Fetch API](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API), and [ReadableStream reader](https://developer.mozilla.org/en-US/docs/Web/API/ReadableStreamDefaultReader/read).
