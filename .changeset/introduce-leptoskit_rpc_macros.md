---
leptoskit_rpc_macros: breaking
---

# leptoskit_rpc_macros: RPC attributes

`leptoskit_rpc_macros` provides the `#[api]`, `#[query]`, and `#[mutation]` attributes behind `leptoskit_rpc`.

Use them through `leptoskit_rpc`; an `#[api]` publishes a leptos server function to generated clients, while queries and mutations add the client-side retry and cache behavior.
