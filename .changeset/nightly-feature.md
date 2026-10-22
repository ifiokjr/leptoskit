---
"release": minor
---

# Nightly signal call syntax

Add a `nightly` cargo feature mirroring Leptos's own: on a nightly compiler, signals gained call syntax — `value()` reads, `set_value(true)` writes — across `leptoskit_use` hooks, `leptoskit_fns` queries and mutations, `leptoskit_query` state, and the Leptos stack the toolkit forwards to (`reactive_graph/nightly`, `leptos/nightly`). Signals also compose wherever closures are accepted, and `MaybeRwSignal` itself becomes callable. The feature is a no-op on stable toolchains, so `--all-features` builds, CI, and publishing are unaffected; a dedicated CI job compiles and tests the feature on a nightly toolchain.
