# leptoskit_rpc_macros

> The `#[api]` attribute behind `leptoskit_rpc`. Use it as `leptoskit_rpc::api`.

Placed above `#[server]`, `#[api]` publishes the function. The route is pinned to a stable `/api/<name>` — the function name, or `#[api(name = "...")]` — unless `#[server]` sets an `endpoint`. What no generated client could call is a compile error that points at its cause: websocket protocols, streaming and multipart encodings, `#[server(flatten)]` arguments, the positional `#[server]` form without an endpoint, and raw-identifier function names. Under the `ssr` feature, enabled through `leptoskit_rpc/ssr`, a description is registered with the route, encodings, doc comment, argument docs (from each argument's doc comment or the `# Arguments` section), and a JSON Schema thunk for every argument and the return type, which `leptoskit_rpc::spec::collect` exports. `#[server(skip)]` arguments are left out.

Options: `internal` (callable, just not publicized), `name = "..."`, `tag = "..."` (repeatable), and `crate = "..."`.
