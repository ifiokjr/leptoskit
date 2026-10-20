# Server functions

A leptos `#[server]` function is already a typed RPC that works on both sides of the wire. What it leaves to you is everything around the call: retrying a flaky network, telling a dropped connection apart from bad input, and giving the UI one type for loading, success, and failure. [`leptoskit_fns`](https://docs.rs/leptoskit_fns) adds that layer and nothing else.

The same calls work from the browser, from native shells, and during SSR. Axum is the only supported server.

## Errors that mean something

`FnError` sorts every `ServerFnError` into one of five kinds, so the UI can respond to each one differently:

| Variant   | Comes from                                | Retryable |
| --------- | ----------------------------------------- | --------- |
| `Network` | the request never arrived or was dropped  | yes       |
| `Server`  | the function failed while running         | yes       |
| `Invalid` | bad or missing arguments                  | no        |
| `Decode`  | a response or request that wouldn't parse | no        |
| `Other`   | registration and response errors          | no        |

```rust,ignore
use leptoskit_fns::FnError;

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
use leptoskit_fns::FnError;
use leptoskit_fns::RetryPolicy;
use leptoskit_fns::query;

let todos = query(RetryPolicy::reads(), || async {
	list_todos().await.map_err(FnError::from)
});
```

Only retryable errors are retried. An `Invalid` response comes back straight away.

## Mutations

`mutation` wraps a call in a leptos `Action` that returns the same `Result<_, FnError>`. Mutations don't retry, because sending the same POST again without checking is usually a bug. If the call is idempotent, use `mutation_with_retry`:

```rust,ignore
use leptoskit_fns::FnError;
use leptoskit_fns::RetryPolicy;
use leptoskit_fns::mutation;
use leptoskit_fns::mutation_with_retry;

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

Backoff waits use the browser's timer on `wasm32` and Tokio's timer everywhere else.
