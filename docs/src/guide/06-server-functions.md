# Server functions

A leptos `#[server]` function is already a typed RPC that works on both sides of the wire. What it leaves to you is everything around the call: retrying a flaky network, telling a dropped connection apart from bad input, and giving the UI one type for loading, success, and failure. [`leptoskit_rpc`](https://docs.rs/leptoskit_rpc) adds that layer and nothing else.

The same calls work in the browser and during SSR, on the web target. Native apps on iOS, Android, macOS, Windows, Linux, and HarmonyOS run their interface as a Wasm guest that contains no server-function code, so these calls don't exist there: a native app reaches your server through the [HTTP plugin](../platforms/plugins.md#plugins-that-need-a-policy) instead. Axum is the only supported server.

## Errors that mean something

`FnError` sorts every `ServerFnError` into one of five kinds, so the UI can respond to each one differently:

| Variant   | Comes from                                | Retryable |
| --------- | ----------------------------------------- | --------- |
| `Network` | the request never arrived or was dropped  | yes       |
| `Server`  | the function failed while running         | no        |
| `Invalid` | bad or missing arguments                  | no        |
| `Decode`  | a response or request that wouldn't parse | no        |
| `Other`   | registration and response errors          | no        |

`Server` errors aren't retried because they are also how a function says no. `ServerFnError::new("not found")` and a database outage arrive the same way, and retrying a refusal only makes the user wait longer for it. When you know a failure is temporary, decide for yourself: a [`leptoskit_query`](11-fetching-data.md) scope takes `Retry::decide`, which sees the `FnError` and can retry `Server` errors you recognize.

```rust,ignore
use leptoskit_rpc::FnError;

let error = FnError::from(server_fn_error);
if error.retryable() {
	// show "reconnecting…"
} else {
	// show error.message() next to the form
}
```

## Queries

`query` turns a server function into a reactive value under a retry policy. It re-runs when any signal it reads changes, and each run gets the whole retry budget.

```rust,ignore
use leptoskit_rpc::FnError;
use leptoskit_rpc::RetryPolicy;
use leptoskit_rpc::query;

let todos = query(RetryPolicy::reads(), || async {
	list_todos().await.map_err(FnError::from)
});
```

Only network failures are retried. An `Invalid` or `Server` answer comes back straight away.

## Mutations

`mutation` wraps a call in a leptos `Action` that returns the same `Result<_, FnError>`. Mutations don't retry, because sending the same POST again without checking is usually a bug. If the call is idempotent, use `mutation_with_retry`:

```rust,ignore
use leptoskit_rpc::FnError;
use leptoskit_rpc::RetryPolicy;
use leptoskit_rpc::mutation;
use leptoskit_rpc::mutation_with_retry;

let add = mutation(|title: &String| {
	let title = title.clone();
	async move { create_todo(title).await.map_err(FnError::from) }
});

let rename = mutation_with_retry(RetryPolicy::reads(), |input: &(u64, String)| {
	let (id, title) = input.clone();
	async move { rename_todo(id, title).await.map_err(FnError::from) }
});
```

## Retry policies

| Policy                      | Attempts | Backoff                    |
| --------------------------- | -------- | -------------------------- |
| `RetryPolicy::none()`       | 1        | none                       |
| `RetryPolicy::reads()`      | 3        | 150ms, doubling, up to 2s  |
| `RetryPolicy::aggressive()` | 5        | 250ms, doubling, up to 10s |

Backoff waits use the same timer as `leptoskit_query`: `setTimeout` in a browser window or worker, Tokio's timer when a Tokio runtime is running on the server, and a helper thread when none is. A host that is none of those installs its own with `leptoskit_query::install_sleep`.
