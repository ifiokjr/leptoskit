# Nightly feature

Leptos offers a `nightly` cargo feature that unlocks extra ergonomics when the project compiles with a nightly Rust compiler. leptoskit ships the same opt-in: enable the `nightly` feature and signals gain call syntax across the toolkit and the Leptos APIs it exposes.

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["nightly"] }
```

Individual crates accept it too (`leptoskit_use`, `leptoskit_fns`) when you depend on them directly.

## What it unlocks

With the feature active, read and write signals by calling them:

```rust,ignore
use leptoskit::hooks::*;

let toggle = use_toggle(false);

assert!(!toggle.value()); // instead of toggle.value.get()
toggle.set_value(true); // instead of toggle.set_value.set(true)
```

leptoskit's own [`MaybeRwSignal`] becomes callable as well, reading the plain value or querying the wrapped signal:

```rust,ignore
let initial = MaybeRwSignal::Value(3);
assert_eq!(initial(), 3);

let shared = RwSignal::new(3);
let reactive = MaybeRwSignal::Signal(shared);
assert_eq!(reactive(), 3);
```

Because signals implement the `Fn` traits, they also compose where closures are expected:

```rust,ignore
fn render(when: impl Fn() -> bool) { /* ... */ }

let large = breakpoints.greater_or_equal(&"large").unwrap();
render(large);
```

The feature forwards to `reactive_graph/nightly` and `leptos/nightly`, so every signal type in the Leptos stack — `RwSignal`, `ReadSignal`, `WriteSignal`, `Signal`, `Memo`, `ArcRwSignal`, and friends — supports call syntax in the same build.

## Toolchain requirements

The extra impls rely on the unstable `fn_traits` and `unboxed_closures` features, so they only compile when **both** of these hold:

1. the `nightly` cargo feature is enabled, and
2. the compiler is a nightly build.

On a stable toolchain the feature compiles as a no-op: `--all-features` builds, CI, and publishing all keep working. The build scripts detect the compiler channel and set a `rustc_nightly` cfg guard, the same mechanism Leptos uses.

Test targets do not run build scripts, so when you run nightly tests yourself, pass the cfg through `RUSTFLAGS`:

```sh
RUSTFLAGS='--cfg rustc_nightly' cargo +nightly test --features nightly
```

[`MaybeRwSignal`]: https://docs.rs/leptoskit_use/latest/leptoskit_use/enum.MaybeRwSignal.html
