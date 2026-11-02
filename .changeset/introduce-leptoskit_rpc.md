---
leptoskit_rpc: breaking
---

# leptoskit_rpc: typed server operations

`leptoskit_rpc` is the app's operations layer: leptos server functions with retries and error classification, plus queries and mutations that run on the database and publish typed clients for other languages.

It is the boundary between shared screens and the server. The same call runs during SSR, in the browser, and in a native guest; `leptoskit_rpc_codegen` turns the specification into TypeScript, Python, Go, Rust, Dart, and CLI clients.
