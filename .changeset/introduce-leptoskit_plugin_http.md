---
leptoskit_plugin_http: major
---

# leptoskit_plugin_http: bounded HTTP

`leptoskit_plugin_http` is the leptoskit plugin for scoped asynchronous HTTP clients with bounded request and response bodies, a total timeout, and cancellation by dropping the request.

Add `leptoskit_plugin_http` as a direct dependency, select the `http` plugin in `native.plugins`, and list the origins the app may reach under `[native.plugin-options.http]`; there is no wildcard, and the guest can't widen the list. Guests use `default-features = false, features = ["guest"]` and send requests through `guest::HttpClient`. Native requests use reqwest with rustls on the host's async runtime; browser apps keep the default `browser` feature and use fetch with incremental body reading. Redirects are not followed: native callers get the 3xx response, and the browser rejects it.
