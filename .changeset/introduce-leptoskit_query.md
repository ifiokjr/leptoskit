---
leptoskit_query: breaking
---

# leptoskit_query: async state in the TanStack style

`leptoskit_query` is TanStack Query for leptos: cached, deduplicated, retrying async state on fine-grained signals, with infinite queries and optimistic mutations.

It is the data layer for screens that read server functions: queries share a cache across components, refetch on focus and reconnect, persist across tabs or native windows, and pause offline. `leptoskit_rpc` supplies the calls it wraps.
