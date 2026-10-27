---
"release": minor
---

# Portable utilities for leptoskit_use

`leptoskit_use` mirrors more of leptos-use's portable surface: `use_timestamp` reports elapsed host-clock time since creation, `watch_debounced` and `watch_throttled` add trailing-edge debounced and leading-edge throttled watchers on the shared clock, `use_async_state` loads a signal from a caller-driven async operation with explicit pending/success/error state, and `use_supported` wraps capability probes as signals. Like the rest of the crate, none of them spawn work: the host drives every future and timer.
