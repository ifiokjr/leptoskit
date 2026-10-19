# leptoskit_fns

> An ergonomic API over leptos server functions.

Leptos server functions are already typed, isomorphic RPC. This crate adds the layer around the call:

- `query` — a retrying, reactive read with an error type that keeps loading and failure in one value;
- `mutation` / `mutation_with_retry` — leptos actions with unified errors; mutations only retry when you say so, because blindly re-sending a POST is usually wrong;
- `FnError` — failures classified as network, invalid, server, or decode, so "offline" and "your input was rejected" stop looking identical in the UI;
- `RetryPolicy` — exponential backoff with a cap, tuned defaults for reads and flaky mobile networks.
