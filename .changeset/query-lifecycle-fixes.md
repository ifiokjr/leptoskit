---
"release": minor
---

# Fix query lifecycle, cancellation, and cache identity

Queries now leave the cache when they should and never take stale data:

- `use_queries` keeps every handle alive as its key list grows, rebinds when its queries are removed, and releases its observers on unmount, so those queries stop counting as active and are garbage collected. An observer that is dropped always unregisters from its query.
- A cancelled or superseded fetch no longer writes anything. Resetting a query before its fetch starts leaves it idle (and `use_is_fetching` drops back to zero), a response racing `cancel_queries` no longer overwrites an optimistic update, and an older response can no longer overwrite a newer one.
- Data written with `set_query_data`, `set_query_data_at`, or `update_query_data` is garbage collected after its gc time like any other unobserved query, so writes now start a timer task on the executor. A query keeps at most one cleanup timer, a stale timer can no longer evict the query that replaced it, and native hosts without a tokio runtime share one timer thread instead of starting a thread per sleep.
- Checking whether queries are active no longer subscribes the caller to every observer's `enabled` closure.

Cache identity is now explicit:

- An unnamed `QueryScope` or `InfiniteQueryScope` whose fetcher closure captures values panics on first use, rather than sharing one cache with every scope built from the same closure. Name it with `.with_name(..)`.
- Key hashes and named scope ids are stable across platforms and Rust releases (`usize` and lengths hash as 64-bit), so a server's dehydrated cache matches the browser's. `QueryClient::dehydrate` panics for an unnamed scope, whose id differs between binaries. Keys whose hashes collide are compared for equality and stay separate queries.

`InfiniteQueryScope` builders no longer change clones made earlier, and gain `with_retry_delay` and `with_data_eq`. The docs now say that hydrated data refetches in the background unless its scope has a stale time, and the optimistic update recipe cancels in-flight fetches and removes the optimistic entry when there was nothing to roll back to.

In `leptoskit_use`, `signal_debounced` and `signal_throttled` no longer notify subscribers when a write leaves the value unchanged.
