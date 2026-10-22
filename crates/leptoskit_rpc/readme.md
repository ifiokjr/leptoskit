<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_rpc

> An ergonomic API over leptos server functions.

Leptos server functions are already typed, isomorphic RPC. This crate adds the layer around the call:

- `query` — a retrying, reactive read with an error type that keeps loading and failure in one value;
- `mutation` / `mutation_with_retry` — leptos actions with unified errors; mutations only retry when you say so, because blindly re-sending a POST is usually wrong;
- `FnError` — failures classified as network, invalid, server, or decode, so "offline" and "your input was rejected" stop looking identical in the UI;
- `RetryPolicy` — exponential backoff with a cap, tuned defaults for reads and flaky mobile networks.

## Publishing the RPC to any language

`#[api]` above `#[server]` publishes a function: a stable `/api/<name>` route, a JSON Schema description of its arguments and result (through `schemars`), and its doc comment — including the `# Arguments` section — as documentation for every argument. `#[api(internal)]` keeps a function callable but unpublishes it. With `ssr` enabled next to `leptos/ssr` and `leptoskit_rpc::spec::export_if_requested()` first in `main`, `leptoskit rpc generate` writes typed clients that speak the RPC's own protocol for TypeScript, Python, Go, Rust, Dart, and the command line, plus the OpenAPI 3.1 document. See the book's "Sharing the RPC" chapter for the walkthrough.
