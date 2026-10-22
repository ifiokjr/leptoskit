# Sharing the RPC: clients for any language

A leptos server function is already an RPC with a route, an encoding, and a typed payload. This chapter is about **publishing** that RPC: marking which functions are part of it, and generating typed clients — TypeScript, Python, Go, Rust, Dart, and a command line — directly from the codebase, docs included. Nothing new is served: the generated clients call the same routes the app itself uses.

```text
app crate ── #[api] #[server] fns ──► spec (routes, schemas, docs) ──► OpenAPI
                                            │
                                            └─► leptoskit fns generate
                                                ├─► TypeScript client (fetch)
                                                ├─► Python client (stdlib + optional httpx)
                                                ├─► Go client (net/http)
                                                ├─► Rust client crate (reqwest)
                                                ├─► Dart client (package:http)
                                                ├─► CLI (one command per function)
                                                └─► openapi.json / openapi.yaml
```

## Publishing functions

Add `leptoskit_fns` to the app, and enable its `ssr` feature from the app's own `ssr` feature so the server build registers the descriptions:

```toml
[dependencies]
leptoskit_fns = "0.1"
schemars = "1"

[features]
ssr = ["leptos/ssr", "leptoskit_fns/ssr", "dep:leptos_axum"]
```

Then put `#[api]` above `#[server]`. Every argument type and the `Ok` type of the return value must implement `schemars::JsonSchema` — one derive next to `Serialize`/`Deserialize`:

```rust
use leptos::prelude::*;
use leptoskit_fns::api;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// One todo item.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct Todo {
	/// The identifier.
	pub id: u64,
	/// What to do.
	pub title: String,
	/// Free-form labels.
	#[serde(default)]
	pub tags: Vec<String>,
}

/// Lists todos.
///
/// # Arguments
///
/// * `filter` - Which todos to list.
/// * `limit` - How many to return.
#[api(tag = "todos")]
#[server]
pub async fn list_todos(
	filter: TodoFilter,
	limit: Option<u32>,
) -> Result<Vec<Todo>, ServerFnError> {
	// ...
}

/// Flushes caches. Callable, just not publicized.
#[api(internal)]
#[server]
pub async fn flush_caches() -> Result<(), ServerFnError> {
	// ...
}

/// A plain server function: private to the app, invisible to clients.
#[server]
pub async fn track_click(id: u64) -> Result<(), ServerFnError> {
	// ...
}
```

What `#[api]` does:

- **Stable routes.** Unless `#[server]` sets an `endpoint`, the route becomes `/api/<function_name>`. The default `#[server]` endpoint embeds a hash of the absolute path of the crate, which changes between checkouts — fine for an app calling itself, useless for a published client. If two functions end up on one route, or share a name, exporting the spec fails with the pair.
- **Descriptions**, registered under the `ssr` feature: the route, the encodings, the doc comment, deprecation, tags, and a JSON Schema (via [`schemars`](https://docs.rs/schemars)) for every argument and the return value, honoring `serde` attributes exactly. Argument attributes `#[server(default)]` and `#[server(rename = "...")]` are honored.
- **Docs on everything.** The doc comment travels with the function into every generated client, and the `# Arguments` section — rustdoc's own convention, `*`name`- description` lines — lands on the matching argument: in `--help` texts, parameter doc comments, docstrings, and the OpenAPI descriptions. Property docs on the types (the `/// The identifier.` above) land on the generated model fields.
- **`internal` means unpublicized, not private.** An internal function is still callable — everything on the internet is — it is simply left out of generated clients and the OpenAPI document unless `--include-internal` is passed.

Options: `internal`, `name = "..."` (when the function name is taken), `tag = "..."` (repeatable), and `crate = "..."` (the path to `leptoskit_fns` when it is reached through the `leptoskit` meta crate: `crate = "leptoskit::fns"`).

Not supported: server functions with a custom `protocol` (websockets, streams) — `#[api]` rejects them at compile time, because they have no HTTP request/response shape a client could speak.

## Exporting the spec

The spec comes from the server binary itself, so it always matches the code that was just compiled. Call `export_if_requested` first thing in `main`:

```rust
#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
	leptoskit_fns::spec::export_if_requested();
	// ...mount the app and serve as usual
}
```

When the `LEPTOSKIT_FNS_SPEC_OUT` environment variable is unset it returns immediately. When `leptoskit fns` sets it, the spec is written there and the process exits before anything binds a port. Title, version, description, and servers come from `leptoskit.toml`; `export_if_requested_with` takes them from code instead.

```bash
leptoskit fns list                     # name, route, summary per published function
leptoskit fns spec --format openapi    # the OpenAPI 3.1 document (also openapi-yaml, json)
leptoskit fns spec --include-internal  # the team's view
```

Every `fns` command accepts `--from <file-or-url>` to use a saved spec instead of building and running the server. The spec is also a document the server can serve: with the `axum` feature, `leptoskit_fns::spec::openapi_router(&options)?` answers `openapi.json` and `openapi.yaml` wherever it is nested, for Swagger UI, Redoc, or any OpenAPI toolchain.

One thing to know: descriptions register when the crate that defines them is linked into the binary. Every real server binary uses its app library (it mounts the app), so this holds in practice; a `main` that references nothing from the library would export an empty spec, and the CLI says so.

## Configuring

Names, directories, and exclusions live under `[fns]` in `leptoskit.toml`:

```toml
[fns]
title = "Todos RPC" # defaults to "<app.name> RPC"
description = "The todo backend."
servers = [
	"https://api.todos.example",
] # the first is the clients' default origin
exclude = ["debug_*"] # name globs left out of everything generated
include_internal = false # true for an internal package

[[fns.targets]] # what `leptoskit fns generate` writes by default
language = "typescript"
package = "@acme/todos"

[[fns.targets]]
language = "python"
package = "todos-api"

[[fns.targets]]
language = "openapi"
out = "docs/openapi"
```

## Generating clients

```bash
leptoskit fns generate                     # the configured targets, or all of them
leptoskit fns generate --target rust --build   # ...and cargo-check the Rust crates
leptoskit fns generate --target typescript --out web/api --package @acme/todos
leptoskit fns generate --from spec.json --target cli
```

| Target       | Writes                                                                                                                                                    |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `typescript` | One dependency-free `index.ts` over `fetch`, plus `package.json` and `tsconfig.json`. Errors classify into `FnError`.                                     |
| `python`     | A package with `TypedDict` models, a standard-library `Client`, and an `httpx`-backed `AsyncClient` (install the `async` extra).                          |
| `go`         | A package over `net/http` with struct models, `json` tags, and string-enum types.                                                                         |
| `rust`       | A `reqwest` crate with serde models reproducing the wire shapes — string enums, tagged enums, `Option` — depending on neither leptos nor `leptoskit_fns`. |
| `dart`       | A package over `package:http` with model classes and `fromJson`/`toJson`.                                                                                 |
| `cli`        | A clap command line, one kebab-case command per function, each `--flag` parsed as JSON into the function's own type.                                      |
| `openapi`    | `openapi.json` and `openapi.yaml`.                                                                                                                        |

Functions whose encodings a client cannot speak (CBOR and friends) are skipped with the reason recorded in that client's README rather than failing the run.

### What regeneration overwrites

Every generated file is either **code** — the typed client, the commands, the OpenAPI document — or **scaffolding** — manifests (`package.json`, `pyproject.toml`, `go.mod`, `Cargo.toml`, `pubspec.yaml`), `tsconfig.json`, `.gitignore`, READMEs. Code has to track the RPC and is always rewritten. Scaffolding is yours once it exists, so `--scaffold` (or `scaffold` on a `[[fns.targets]]` entry) decides how much of a package is generated:

| Mode     | Code   | Scaffolding                                                      |
| -------- | ------ | ---------------------------------------------------------------- |
| `create` | always | only where the file does not exist yet — the default             |
| `full`   | always | always, overwriting your edits (useful for CI or a fresh start)  |
| `files`  | always | never: you define the package and point it at the generated code |

`leptoskit fns generate` reports what it kept, and how to rewrite it.

### The clients in use

TypeScript:

```ts
import { Client, FnError } from "@acme/todos";

const api = new Client({ serverUrl: "https://api.todos.example" });
try {
	const todos = await api.listTodos({ filter: "all", limit: 20 }); // Todo[]
} catch (error) {
	if (error instanceof FnError && error.retryable) {
		// a network or server failure: safe to try again
	}
}
```

Python:

```python
from todos_api import Client, FnError

api = Client("https://api.todos.example")
try:
    todos = api.list_todos(filter="all", limit=20)  # list[Todo]
except FnError as error:
    ...
```

The command line:

```bash
todos-cli --server-url https://api.todos.example list-todos --filter '"all"' --limit '20'
```

Every `--flag` takes one JSON value — `'"all"'` for the enum, `'20'` for the number, `'["urgent"]'` for a list — parsed into the function's own type, so invalid input is rejected before anything is sent:

```
error: invalid value '"bogus"' for '--filter <FILTER>': not valid JSON for this argument:
unknown variant `bogus`, expected one of `all`, `done`, `pending`
```

## Errors

Every client classifies failures the way `leptoskit_fns::FnError` does, parsed from the server's `Kind|message` error text:

| `ServerFnError` variant                             | kind      | retryable |
| --------------------------------------------------- | --------- | --------- |
| `Request`                                           | `network` | yes       |
| `ServerError`, `MiddlewareError`, `WrappedServerFn` | `server`  | yes       |
| `Args`, `MissingArg`                                | `invalid` | no        |
| `Deserialization`, `Serialization`                  | `decode`  | no        |
| anything else                                       | `other`   | no        |

## How the wire protocol is spoken

The generated clients reproduce what leptos's own codecs do, per function:

- `#[server]` (default) sends `POST` with a form-urlencoded body, nested values in brackets and arrays by index, exactly as `serde_qs` writes them;
- `#[server(input = GetUrl)]` sends `GET` with the same encoding in the query string;
- `#[server(input = Json)]` sends `POST` with a JSON body, absent optional arguments omitted so the server's `#[server(default)]` applies;
- responses are JSON; errors are the `text/plain` `Kind|message` body above.

Because the schemas come from `schemars`, `serde` attributes — `rename_all`, `tag`, `default`, `skip` — are already reflected in the generated types; the clients do not guess.
