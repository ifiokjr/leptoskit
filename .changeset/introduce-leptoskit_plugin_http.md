---
leptoskit_plugin_http: breaking
---

# leptoskit_plugin_http: bounded HTTP

`leptoskit_plugin_http` is the leptoskit plugin for scoped asynchronous HTTP clients with bounded request and response bodies, a total timeout, and cancellation by dropping the request.

Add `leptoskit_plugin_http` as a direct dependency and select the `http` plugin in `native.plugins`; the generated host discovers it from Cargo metadata, so registration is automatic. Native requests use reqwest with rustls and need a Tokio runtime; the browser uses fetch with incremental body reading. Redirects are not followed, and every destination origin is explicit.
