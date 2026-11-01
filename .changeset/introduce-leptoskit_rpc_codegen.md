---
leptoskit_rpc_codegen: breaking
---

# leptoskit_rpc_codegen: the RPC specification and clients

`leptoskit_rpc_codegen` holds the RPC specification behind leptoskit server functions and generates typed clients from it for TypeScript, Python, Go, Rust, Dart, and the command line.

The server writes the specification; the CLI reads it back and generates clients that speak the RPC's own protocol, plus an OpenAPI 3.1 document. The generators sit behind a default feature, so the server binary can depend on the types alone.
