# Nightly feature

leptos has a `nightly` cargo feature for projects that compile with a nightly Rust compiler, and leptoskit forwards it. Enable it on whichever leptoskit crate you depend on:

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["leptos", "nightly"] }
```

`leptoskit_rpc` and `leptoskit_query` accept it too when you depend on them directly. It turns on leptos's own `nightly` feature, which its `view!` macro and renderer use. On a stable toolchain it compiles as a no-op, so `--all-features` builds, CI, and publishing keep working.

## Calling signals works on stable

You don't need the feature to call a signal. Since leptos 0.9, every signal reads and writes by being called, on stable as well as nightly. That includes the ones leptoskit's hooks return:

```rust,ignore
let toggle = use_toggle(false);

assert!(!(toggle.value)()); // or toggle.value.get()
(toggle.set_value)(true); // or toggle.set_value.set(true)
```

A signal derefs to a closure rather than being one. Where a function expects `impl Fn() -> T`, pass `&*signal`, or wrap it as `move || signal()`:

```rust,ignore
fn render(when: impl Fn() -> bool) { /* ... */ }

render(&*large);
```

`MaybeRwSignal`, the initial value or existing signal a hook accepts, works the same way: `initial()` reads the plain value or the wrapped signal.
