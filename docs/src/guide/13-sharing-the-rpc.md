# Sharing the RPC: clients for any language

A leptos server function is already an RPC with a route, an encoding, and a typed payload. This chapter is about **publishing** that RPC: marking which functions are part of it, and generating typed clients — TypeScript, Python, Go, Rust, Dart, and a command line — directly from the codebase, docs included. Nothing new is served: the generated clients call the same routes the app itself uses.

```text
app crate ── #[api] #[server] fns ──► spec (routes, schemas, docs) ──► OpenAPI
                                            │
                                            └─► leptoskit rpc generate
                                                ├─► TypeScript client (fetch)
                                                ├─► Python client (stdlib + optional httpx)
                                                ├─► Go client (net/http)
                                                ├─► Rust client crate (reqwest)
                                                ├─► Dart client (package:http)
                                                ├─► CLI (one command per function)
                                                └─► openapi.json / openapi.yaml
```

[`examples/harbor`](https://github.com/ifiokjr/leptoskit/tree/main/examples/harbor) publishes three server functions this way and commits the TypeScript client `leptoskit rpc generate` writes for them; its tests fail when the client falls behind the code.

## Publishing functions

Add `leptoskit_rpc` to the app, and enable its `ssr` feature from the app's own `ssr` feature so the server build registers the descriptions:

```toml
[dependencies]
leptoskit_rpc = "0.1"
schemars = "1"

[features]
ssr = ["leptos/ssr", "leptoskit_rpc/ssr", "dep:leptos_axum"]
```

If the app reaches `leptoskit_rpc` through the `leptoskit` crate instead, enable `rpc` for the client side and `rpc-ssr` from the `ssr` feature, which also turns on the server-side registration and the `leptoskit::rpc::spec` module, and write `#[api(crate = "leptoskit::rpc")]`. `rpc-axum` adds the OpenAPI router described below.

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["rpc"] }

[features]
ssr = ["leptos/ssr", "leptoskit/rpc-ssr", "dep:leptos_axum"]
```

Then put `#[api]` above `#[server]`. Every argument type and the `Ok` type of the return value must implement `schemars::JsonSchema` — one derive next to `Serialize`/`Deserialize`:

```rust
use leptos::prelude::*;
use leptoskit_rpc::api;
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

- **Stable routes.** Unless `#[server]` sets an `endpoint`, the route becomes `/api/<name>`: the function name, or the `#[api(name = "...")]` you gave it. The default `#[server]` endpoint embeds a hash of the absolute path of the crate, which changes between checkouts — fine for an app calling itself, useless for a published client.
- **One route per function.** Two functions named `list` in different modules would both answer `/api/list`, and leptos would quietly run only one of them. `export_if_requested`, which `main` calls first, checks every start: when two functions share a name or a route, the server exits with the pair instead of starting, and exporting the spec fails the same way. Give one of them its own `#[api(name = "...")]`, which renames the operation and moves its route with it.
- **Descriptions**, registered under the `ssr` feature: the route, the encodings, the doc comment, deprecation, tags, and a JSON Schema (via [`schemars`](https://docs.rs/schemars)) for every argument and the return value, honoring `serde` attributes exactly. Argument attributes `#[server(default)]` and `#[server(rename = "...")]` are honored, and `#[server(skip)]` arguments, which the server never reads, are left out of the clients.
- **Docs on everything.** The doc comment travels with the function into every generated client. Each argument is documented by its own doc comment, or else by its entry in the `# Arguments` section — rustdoc's own convention, `*`name`- description` lines, which may wrap — in `--help` texts, parameter doc comments, docstrings, and the OpenAPI descriptions. Property docs on the types (the `/// The identifier.` above) land on the generated model fields.
- **`internal` means unpublicized, not private.** An internal function is still callable — everything on the internet is — it is simply left out of generated clients and the OpenAPI document unless `--include-internal` is passed.

Options: `internal`, `name = "..."` (the operation name and the route, when the function name is taken; letters, digits, `_`, and `-`), `tag = "..."` (repeatable), and `crate = "..."` (the path to `leptoskit_rpc` when it is reached through the `leptoskit` crate: `crate = "leptoskit::rpc"`).

`#[api]` refuses, at compile time and pointing at the cause, what no generated client could call:

- websocket protocols (`protocol = Websocket<...>`), streaming encodings (`Streaming`, `StreamingText`, as `input`, `output`, or inside `protocol = Http<...>`), and multipart forms (`MultipartFormData`). These have no single JSON request and response. An `Http<Input, Output>` protocol of ordinary encodings is fine;
- `#[server(flatten)]` arguments, whose fields travel at the top level of the request, where generated clients would nest them under the argument. Pass the fields as separate arguments, or the struct as one named argument;
- the positional form without its fourth argument, `#[server(Name, "/api")]`, whose route would keep the checkout hash. Add the endpoint, `#[server(Name, "/api", "Url", "name")]`, or use the named form;
- functions named with a raw identifier such as `r#type`, which leptos cannot generate. Rename the function and keep the published name with `#[api(name = "type")]`. Raw argument names are fine and go on the wire without the `r#`.

Encodings that compile but that generated clients cannot build or decode, such as CBOR, are published and then skipped by the generators, as described under [generating clients](#generating-clients).

## Queries and mutations

A leptos `#[server]` function only exists where leptos does. A native app runs its interface as a Wasm guest with no server functions in it, so on iOS, Android, macOS, Windows, Linux, and HarmonyOS a screen can't call one the way a browser page does. Queries and mutations are operations that every platform calls the same way: by name, with their arguments as one value.

```rust
use leptoskit_rpc as rpc;

/// The caller's notes, newest first.
#[api(tag = "notes")]
#[rpc::query]
pub async fn notes(cx: &rpc::QueryCx, limit: u32) -> rpc::Result<Vec<PublicNote>> {
	let notes = Note::query()
		.order_by(|note| note.created_at.desc())
		.limit(u64::from(limit.min(100)))
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(Into::into).collect())
}

/// Keeps a note for the signed-in person.
#[api(tag = "notes")]
#[rpc::mutation]
pub async fn add_note(cx: &rpc::MutationCx, title: String) -> rpc::Result<Id<Note>> {
	let me = cx.require::<auth::SignedIn>()?;
	if title.trim().is_empty() {
		return Err(rpc::Error::invalid("a note needs a title"));
	}
	let note = NewNote { owner: me.account, title }.insert(cx.tx()).await?;
	Ok(note.id)
}
```

Put this in the crate the app shares between its server and its screens. On every platform, each function becomes a struct of its arguments, named in `PascalCase`: `Notes { limit }` and `AddNote { title }`. Any screen sends one:

```rust
let latest = rpc::client::call(Notes { limit: 20 }).await?;
let id = rpc::client::call(AddNote { title: "Milk".into() }).await?;
```

### What each platform does

| Where                                                       | What `call` does                                                                                                                                                     |
| ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The web                                                     | A same-origin `fetch` to `POST /api/<name>`, which carries the session cookie                                                                                        |
| iOS, Android, macOS, Windows, Linux, and HarmonyOS (guests) | The same request through the HTTP plugin, so add `"http"` to `[native] plugins` and the server to its allowed origins. Add `"crypto"` too (see below)                |
| The server, while it renders a page                         | Runs the operation in process, as the person whose request is being rendered, with no HTTP hop                                                                       |
| The terminal                                                | In process, when the terminal build includes the server (`server` and a database). A terminal app that is only a client of a remote server can't call operations yet |

A guest build contains the argument structs and the client, and nothing else: the bodies, and every database type they use, exist only in the server build.

### The context

The first argument is the context, and the body can only do what its context allows:

- **`QueryCx`** reads. `cx.db()` is the database as the caller: their access policies apply to every row, and it has no write methods, so a query that tries to write doesn't compile.
- **`MutationCx`** writes in one transaction, `cx.tx()`, as the caller. On Postgres the transaction is `SERIALIZABLE`; SQLite runs one writer at a time, so it already is. When another transaction gets in its way, the whole body runs again, up to five more times, or `#[rpc::mutation(retries = N)]`. So a mutation must not reach the outside world itself: `cx.after_commit(async move { … })` runs work, such as sending an email, once the transaction has committed.
- Both have `cx.get::<T>()` and `cx.require::<T>()`, which read what policies read, such as the signed-in person. `require` fails with `rpc::Error::Unauthorized` when it isn't there.

A body returns `rpc::Result<T>`. `?` on a database error turns a missing row into `NotFound`, a policy's refusal into `Forbidden`, a broken rule into `Invalid`, and anything else into a `Server` error that says nothing about the database.

### Serving them

The server merges one router, which answers `POST /api/<name>` for every query and mutation. With [sign-in](23-sign-in.md), each runs as the signed-in person:

```rust
let rpc = leptoskit::rpc::server::Server::new(db.clone()).identify(auth.clone());
let app = Router::new()
	.merge(rpc.router())
	.merge(auth.router())
	.layer(auth.layer());
```

Without `identify`, every caller is anonymous. Anything that implements `rpc::Identify`, such as an API-key check, can tell callers apart instead. Server code calls operations directly with `rpc::call(&db, AddNote { … })`: a mutation still gets its transaction and retries.

### Calls that run once

On a phone, the commonest real-time bug is "did my send go through?": the request arrived, the answer was lost, and sending again saves it twice. A mutation called with an `Idempotency-Key` header runs once: the server keeps a receipt, and a repeat with the same key and arguments gets the first call's result back. A repeat with the same key and other arguments is refused with `409`. Each signed-in session's keys are its own.

The app's own client does this for you: every `call` of a mutation sends a fresh key, and sends the same key again when the network drops the answer. In a native app the key comes from the crypto plugin when the app has it, and is harder to guess. Receipts live in the `rpc` schema pack's `rpc.receipts` table, which joins your migrations once you add `leptoskit::db::packs!(leptoskit_rpc)` next to your tables. Delete old ones now and then with `rpc::server::receipts::prune`.

### Caching

`use_rpc_query` reads a query through `leptoskit_query`, cached by its arguments, shared by every screen that reads it, and carried from the server's render to the browser. `mutation_scope` is a `leptoskit_query` mutation, so an optimistic update is its `with_on_mutate`:

```rust
let notes = use_rpc_query(move || Notes { limit: 20 });
let add = use_mutation(mutation_scope::<AddNote>().with_on_success_async(|args| async move {
	args.client.invalidate_scope(&query_scope::<Notes>()).await;
}));
add.mutate(AddNote { title: "Milk".into() });
```

### Setting it up

Through the `leptoskit` crate, `rpc` adds the client, `web` and `guest` give it a way to reach the server, `server` serves operations and calls them in process while rendering, and `db-postgres` or `db-sqlite` runs them on the database. Write `#[rpc::query(crate = "leptoskit::rpc")]` and `#[api(crate = "leptoskit::rpc")]` when the app reaches `leptoskit_rpc` through `leptoskit`. With [sign-in](23-sign-in.md), its `client::configure` sets up this client too, so a native app's calls carry its access token; otherwise call `leptoskit::rpc::client::configure(ClientConfig::new().origin("https://notes.example"))` when a native app starts.

Without `#[api]`, a query or mutation is still served and callable by the app's own screens; it just isn't in the published spec. With it, generated clients call it like any other function, as below.

## Exporting the spec

The spec comes from the server binary itself, so it always matches the code that was just compiled. Call `export_if_requested` first thing in `main`:

```rust
#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
	leptoskit_rpc::spec::export_if_requested();
	// ...mount the app and serve as usual
}
```

When the `LEPTOSKIT_RPC_SPEC_OUT` environment variable is unset it checks that no two functions share a name or a route, then returns. When `leptoskit rpc` sets it, the spec is written there and the process exits before anything binds a port. Title, version, description, and servers come from `leptoskit.toml`; `export_if_requested_with` takes them from code instead.

```bash
leptoskit rpc list                     # name, route, summary per published function
leptoskit rpc spec --format openapi    # the OpenAPI 3.1 document (also openapi-yaml, json)
leptoskit rpc spec --include-internal  # the team's view
```

Every `leptoskit rpc` command accepts `--from <file-or-url>` to use a saved spec instead of building and running the server. The spec is also a document the server can serve: with the `axum` feature, `leptoskit_rpc::spec::openapi_router(&options)?` answers `openapi.json` and `openapi.yaml` wherever it is nested, for Swagger UI, Redoc, or any OpenAPI toolchain.

One thing to know: descriptions register when the crate that defines them is linked into the binary. Every real server binary uses its app library (it mounts the app), so this holds in practice; a `main` that references nothing from the library would export an empty spec, and the CLI says so.

## Configuring

Names, directories, and exclusions live under `[rpc]` in `leptoskit.toml`:

```toml
[rpc]
title = "Todos RPC" # defaults to "<app.name> RPC"
description = "The todo backend."
servers = [
	"https://api.todos.example",
] # the first is the clients' default origin
exclude = ["debug_*"] # name globs left out of everything generated
include_internal = false # true for an internal package

[[rpc.targets]] # what `leptoskit rpc generate` writes by default
language = "typescript"
package = "@acme/todos"

[[rpc.targets]]
language = "python"
package = "todos-api"

[[rpc.targets]]
language = "openapi"
out = "docs/openapi"
```

## Generating clients

```bash
leptoskit rpc generate                     # the configured targets, or all of them
leptoskit rpc generate --target rust --build   # ...and cargo-check the Rust crates
leptoskit rpc generate --target typescript --out web/api --package @acme/todos
leptoskit rpc generate --from spec.json --target cli
```

| Target       | Writes                                                                                                                                                    |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `typescript` | One dependency-free `index.ts` over `fetch`, plus `package.json` and `tsconfig.json`. Errors classify into `FnError`.                                     |
| `python`     | A package with `TypedDict` models, a standard-library `Client`, and an `httpx`-backed `AsyncClient` (install the `async` extra).                          |
| `go`         | A package over `net/http` with struct models, `json` tags, and string-enum types.                                                                         |
| `rust`       | A `reqwest` crate with serde models reproducing the wire shapes — string enums, tagged enums, `Option` — depending on neither leptos nor `leptoskit_rpc`. |
| `dart`       | A package over `package:http` with model classes and `fromJson`/`toJson`.                                                                                 |
| `cli`        | A clap command line, one kebab-case command per function, each `--flag` parsed as JSON into the function's own type.                                      |
| `openapi`    | `openapi.json` and `openapi.yaml`.                                                                                                                        |

Functions whose encodings a client cannot speak (CBOR and friends) are skipped with the reason recorded in that client's README rather than failing the run.

### What regeneration overwrites

Every generated file is either **code** — the typed client, the commands, the OpenAPI document — or **scaffolding** — manifests (`package.json`, `pyproject.toml`, `go.mod`, `Cargo.toml`, `pubspec.yaml`), `tsconfig.json`, `.gitignore`, READMEs. Code has to track the RPC and is always rewritten. Scaffolding is yours once it exists, so `--scaffold` (or `scaffold` on a `[[rpc.targets]]` entry) decides how much of a package is generated:

| Mode     | Code   | Scaffolding                                                      |
| -------- | ------ | ---------------------------------------------------------------- |
| `create` | always | only where the file does not exist yet — the default             |
| `full`   | always | always, overwriting your edits (useful for CI or a fresh start)  |
| `files`  | always | never: you define the package and point it at the generated code |

`leptoskit rpc generate` reports what it kept, and how to rewrite it.

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

Mutations take an optional idempotency key: `idempotencyKey` in TypeScript's options and Dart's named arguments, `idempotency_key=` in Python, a context from `WithIdempotencyKey` in Go, `Client::with_idempotency_key` in Rust, and `--idempotency-key` on the command line. Send the same key again when a call's answer was lost, and it still runs once.

Every `--flag` takes one JSON value — `'"all"'` for the enum, `'20'` for the number, `'["urgent"]'` for a list — parsed into the function's own type, so invalid input is rejected before anything is sent:

```
error: invalid value '"bogus"' for '--filter <FILTER>': not valid JSON for this argument:
unknown variant `bogus`, expected one of `all`, `done`, `pending`
```

## Errors

Every client classifies failures the way `leptoskit_rpc::FnError` does, parsed from the server's `Kind|message` error text:

| `ServerFnError` variant                             | kind      | retryable |
| --------------------------------------------------- | --------- | --------- |
| `Request`                                           | `network` | yes       |
| `ServerError`, `MiddlewareError`, `WrappedServerFn` | `server`  | no        |
| `Args`, `MissingArg`                                | `invalid` | no        |
| `Deserialization`, `Serialization`                  | `decode`  | no        |
| anything else                                       | `other`   | no        |

Only `network` failures are retryable: the request never got an answer. A `server` error is final, because a function reports a refusal such as "not found" (`ServerFnError::new`) the same way as an outage.

Custom error types are not described yet. A server function whose error type is its own, rather than `ServerFnError`, sends errors in that type's encoding: the OpenAPI document marks them `x-leptoskit-error-encoding: custom` with no schema, and every generated client reports them as `other`, with the raw body in its `raw` field (`Error::Fn`'s message in Rust) for you to decode.

## How the wire protocol is spoken

The generated clients reproduce what leptos's own codecs do, per function:

- `#[server]` (default) sends `POST` with a form-urlencoded body, nested values in brackets and arrays by index, exactly as `serde_qs` writes them;
- `#[server(input = GetUrl)]` sends `GET` with the same encoding in the query string, and `DeleteUrl` sends `DELETE` the same way;
- `PatchUrl` and `PutUrl` send `PATCH` and `PUT` with the form-urlencoded body, and `PatchJson` and `PutJson` with a JSON body;
- `#[server(input = Json)]` sends `POST` with a JSON body, absent optional arguments omitted so the server's `#[server(default)]` applies;
- queries and mutations always send `POST /api/<name>` with a JSON body, and a mutation may carry an `Idempotency-Key` header. The spec marks them with `kind`, and the OpenAPI document with `x-leptoskit-kind`;
- responses are JSON; errors are the `text/plain` `Kind|message` body above.

Because the schemas come from `schemars`, `serde` attributes — `rename_all`, `tag`, `default`, `skip` — are already reflected in the generated types; the clients do not guess.

## Checking compatibility

Installed apps and published SDKs are a wire contract. Record the published spec before a release:

```bash
leptoskit rpc spec --out rpc.json
```

Then, in CI, compare the next revision with it:

```bash
leptoskit rpc check --against rpc.json
```

It fails on a removed function, a changed kind, route, or encoding, a changed result schema, a removed argument, a new required argument, or a new required OAuth scope — the changes a client generated from the old spec can't survive. Calls and functions excluded by `rpc.exclude` are compared the same way unless `--include-internal` asks otherwise. Fix the break or give the operation a new name, and keep the old one as a deprecated alias until installed clients have moved on.

`--from` reads the current spec from a file or URL instead of building the server, so the check runs without a toolchain.
