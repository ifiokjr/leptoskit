# leptoskit_fns_macros

> The `#[api]` attribute behind `leptoskit_fns`. Use it as `leptoskit_fns::api`.

Placed above `#[server]`, `#[api]` publishes the function: the route is pinned to a stable `/api/<name>` (unless `#[server]` sets an `endpoint`), websocket and streaming protocols are rejected, and — under the `ssr` feature, enabled through `leptoskit_fns/ssr` — a description is registered with the route, encodings, doc comment (including its `# Arguments` section), and a JSON Schema thunk for every argument and the return type, which `leptoskit_fns::spec::collect` exports.

Options: `internal` (callable, just not publicized), `name = "..."`, `tag = "..."` (repeatable), and `crate = "..."`.
