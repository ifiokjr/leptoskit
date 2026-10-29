---
"release": minor
---

# Move to the leptos 0.9 release candidate

leptoskit now builds on leptos `0.9.0-rc0`, with `leptos_router`, `leptos_meta`, and `leptos_axum` at the same release, `reactive_graph 0.3`, `tachys 0.3`, and `server_fn 0.9`. Projects created by `leptoskit new` depend on the same release.

Calling a signal to read it now works on stable, not only with the `nightly` feature: leptos 0.9 makes every signal deref to a closure. Hook results read and write the same way on any toolchain:

```rust
let open = (toggle.value)();
(toggle.set_value)(true);
```

Because a signal is no longer a closure itself, pass `&*signal` where a function expects `impl Fn() -> T`.

`MaybeRwSignal` follows suit: it is callable on stable too, and gains a `get()` method, so `leptoskit_use` drops the `nightly` feature that existed only for its call syntax. The `nightly` feature on `leptoskit`, `leptoskit_rpc`, and `leptoskit_query` still turns on leptos's own `nightly` feature.
