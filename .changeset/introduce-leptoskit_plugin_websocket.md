---
leptoskit_plugin_websocket: breaking
---

# leptoskit_plugin_websocket: WebSocket connections

`leptoskit_plugin_websocket` is the leptoskit plugin for scoped WebSocket connections with bounded messages and explicit lifecycle.

Add `leptoskit_plugin_websocket` as a direct dependency and select the `websocket` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native connections use tokio-tungstenite with rustls and need a Tokio runtime; browser connections own their socket and event callbacks. Every destination origin is explicit.
