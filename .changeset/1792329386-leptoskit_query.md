---
"release": minor
---

# Add `leptoskit_query`, TanStack Query for leptos

A new crate for async state management built on leptos's fine-grained signals: cached, deduplicated, retrying queries with stale-while-revalidate semantics, infinite queries that page in either direction, and mutations with lifecycle callbacks and optimistic updates. Queries refetch on window focus, reconnect, and interval; pause while offline; garbage collect when unobserved; and can stream server-fetched data to the browser through the same hydration mechanism leptos resources use.

Caches sync across browser tabs over a `BroadcastChannel`, or across the windows of a native app over any `BroadcastTransport`. Browser integration sits behind a default `browser` feature, so a wasm bundle hosted outside a browser — an iOS or Android shell — builds without it and installs its own clock and timer. The `leptoskit` meta crate re-exports it behind the `query` feature.
