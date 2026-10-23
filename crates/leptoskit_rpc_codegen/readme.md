# leptoskit_rpc_codegen

> The RPC specification behind leptoskit server functions, and the typed clients generated from it.

The specification is plain `serde` data: one `ApiFunction` per `#[leptoskit_rpc::api]` function — route, HTTP encodings, docs, and a JSON Schema for every argument and the return value — plus the shared schemas. The server side (`leptoskit_rpc`) writes it; this crate reads it back and generates clients that speak the RPC's own protocol: TypeScript over `fetch`, Python over the standard library, Go over `net/http`, Rust over `reqwest`, Dart over `package:http`, a clap command line with one command per function, and the OpenAPI 3.1 document.

The generators sit behind the default `generators` feature; the server binary depends on this crate without it, for the specification types alone. The `leptoskit` CLI drives everything through `leptoskit rpc`.
