---
"release": minor
---

# Harden the published RPC and its generated clients

`#[api(name = "...")]` now also sets the route, `/api/<name>`, so two same-named functions in different modules no longer share one silently; `export_if_requested` checks every start and refuses to run a server where two functions share a name or a route (`leptoskit_rpc::spec::check` runs the same check on its own). `#[api]` rejects, with errors that point at the cause, websocket protocols and the streaming and multipart encodings wherever they are named (a plain `protocol = Http<...>` is now accepted), `#[server(flatten)]` arguments, the positional `#[server]` form without an endpoint, and raw-identifier function names; when it rejects a function it keeps the function, so its error is the only one. `#[server(skip)]` arguments leave the spec, doc comments on arguments document them, wrapped `# Arguments` entries keep every line, raw argument names drop their `r#`, trailing commas in `#[server(...)]` work, and `#[server(custom = Wrapper)]` functions register under `ssr`. `DeleteUrl` functions are described as sending their arguments in the query string.

`FnError::retryable` is now true only for network failures: `ServerFnError::new` is also how a function reports "not found" or "forbidden", and every generated client follows suit. Retry backoff uses `leptoskit_query`'s timer, so it works without a Tokio runtime and in browser workers. Export failures print the IO error that caused them.

Generated clients no longer let schema, function, or argument names shadow the client's own runtime or the language's built-ins (`Client`, `Error`, `Record`, `call`, `headers`, `ctx`, and more get a numeric suffix), the Rust client no longer gives an argument named `result` the function's result type, the Go client keeps every digit of numbers in query strings and JSON bodies and compiles for string-enum results, and TypeScript argument docs can no longer close their comment early. OpenAPI no longer claims custom error types are strings. The `leptoskit` crate gains `rpc-ssr` and `rpc-axum` features that forward `leptoskit_rpc`'s server side.
