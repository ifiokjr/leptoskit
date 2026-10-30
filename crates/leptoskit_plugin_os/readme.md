# leptoskit_plugin_os

The operating system and CPU architecture the app is actually running on. A native leptoskit app runs its interface as a Wasm guest, and from inside the guest everything looks like `wasm32`; this plugin asks the native host instead.

The values come from compile-time constants of the host build, so a call needs no allocation and no platform API:

| Field          | Values                                                                                   |
| -------------- | ---------------------------------------------------------------------------------------- |
| `platform`     | `ios`, `android`, `macos`, `windows`, `linux`, or `web`                                  |
| `architecture` | Rust's `std::env::consts::ARCH`, such as `aarch64`, `x86_64`, or `wasm32` in the browser |

Rust code can read them directly with `leptoskit_plugin_os::info()`. In a browser build the plugin reports `web` and `wasm32` honestly; it never parses a user-agent string, which is spoofable and says nothing reliable about the device. For model, OS version, and memory, use [`leptoskit_plugin_device`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_plugin_device).

Add the crate as a direct app dependency and select `"os"` in `native.plugins`. The plugin is implemented in Rust (`Os`, declared with `rust-type` in the crate's Cargo metadata), so there are no Swift or Kotlin handlers and the host registers it on every native target. A guest has no typed wrapper; it calls the one method through the bridge client and decodes the reply with the bridge codec:

```rust,ignore
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::guest::Client;
use leptoskit_plugin_os::INFO;

let os = Client::new("os")?;
let reply = os.call(INFO, &[]).await?;
let mut decoder = Decoder::new(&reply);
let platform = decoder.string()?;
let architecture = decoder.string()?;
decoder.finish()?;
```

Enable `leptoskit_bridge`'s `guest` feature in the guest crate. Method `0` (`INFO`) takes an empty payload and returns two length-prefixed UTF-8 strings, platform then architecture. A payload on `INFO` returns `InvalidArgument`; any other method returns `Unavailable`. The crate's tests cover the reply encoding and both rejections, and run with `cargo test -p leptoskit_plugin_os`.
