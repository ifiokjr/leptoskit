---
"leptoskit_plugin_web_auth": patch
"leptoskit_plugin_websocket": patch
---

# Keep desktop plugin helpers out of Wasm builds

Scope web-auth's loopback timing constants to its desktop implementation and compile the WebSocket browser test server's connection helpers only on native hosts, so checking every target for Wasm succeeds.
