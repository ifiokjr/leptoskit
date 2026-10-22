# leptoskit_rpc_macros

> The `#[api]` attribute behind `leptoskit_rpc`. Use it as `leptoskit_rpc::api`.

Placed above `#[server]`, `#[api]` publishes the function: the route is pinned to a stable `/api/<name>` (unless `#[server]` sets an `endpoint`), websocket and streaming protocols are rejected, and — under the `ssr` feature, enabled through `leptoskit_rpc/ssr` — a description is registered with the route, encodings, doc comment (including its `# Arguments` section), and a JSON Schema thunk for every argument and the return type, which `leptoskit_rpc::spec::collect` exports.

Options: `internal` (callable, just not publicized), `name = "..."`, `tag = "..."` (repeatable), and `crate = "..."`.
