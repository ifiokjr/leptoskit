---
"release": minor
---

# Publish the RPC: generated clients for any language

`#[leptoskit_fns::api]` above `#[server]` publishes a server function: the route is pinned to a stable `/api/<name>`, the doc comment (including its `# Arguments` section) documents the function and each argument, and JSON Schemas generated through schemars describe every payload. `#[api(internal)]` leaves a function callable but takes it out of generated clients. The server exports the description on request (`leptoskit_fns::spec::export_if_requested`), and the new `leptoskit fns` CLI commands consume it: `fns list` and `fns spec` (leptoskit JSON, OpenAPI 3.1 JSON or YAML), and `fns generate`, which writes typed clients that speak the RPC's own protocol — TypeScript over fetch, Python over the standard library with an optional httpx async client, Go over net/http, Rust over reqwest, Dart over package:http, and a clap command line with one command per function — plus the OpenAPI document. Scaffold modes decide what regeneration may rewrite, and a new `[fns]` section in leptoskit.toml names the clients, their directories, and exclusion globs. Two new crates carry the machinery: `leptoskit_fns_macros` and `leptoskit_fns_codegen`.
