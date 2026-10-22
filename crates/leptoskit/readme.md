<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit

> The all-encompassing toolkit for building, deploying, and shipping leptos apps to the web and to native platforms.

One dependency, one CLI, every target:

- **Web** — SSR, hydration, and wasm builds, served by Axum.
- **Native** — real native widgets driven by a wasm bundle, no web view: the React Native architecture, rewritten in Rust.
- **Over the air** — code push for the wasm bundle, with binary deltas.
- **Server functions** — leptos fns with an ergonomic retrying API.

Build shared screens with the built-in Rust components and Tailwind classes:

```rust
use leptoskit::prelude::*;

fn screen() -> Element {
	View().class("gap-4 p-6 md:flex-row").children([
		Text("Hello, leptoskit").class("text-2xl font-bold"),
		Button("Continue").class("rounded-lg bg-indigo-500 px-4 py-2 text-white"),
	])
}
```

The [component catalog](https://ifiokjr.github.io/leptoskit/reference/components.html) has 14 primitive builders for containers, text, images, scrolling, and native controls. Tailwind expresses layout and visual composition. Another 31 composition helpers remain available through an explicit `components::helpers` import. Every builder returns the same `Element` vocabulary for native and web. They are functions, not `#[component]` functions or `<View>` tags.

Commit the tree to a `NativeSurface`, or enable the optional `web` feature and pass the same tree to `leptoskit::web::render`. The web adapter creates Leptos views and emits CSS. Native rendering applies the same utility declarations to Yoga and widget props. No Tailwind installation or Node build is needed.

`.class` appends validated utilities and panics on invalid input. `.try_class` returns a `ClassError` for dynamic input. Classes override typed base styles regardless of builder call order. Native surfaces reapply responsive and dark rules when the host updates the viewport or calls `set_dark_mode`.

Read the [shared component guide](https://ifiokjr.github.io/leptoskit/guide/07-shared-components.html) and [CSS coverage reference](https://ifiokjr.github.io/leptoskit/reference/css-coverage.html). CSS support is a defined subset. Unsupported utilities and variants return diagnostics.

Enable `use` for platform-neutral reactive hooks under `leptoskit::hooks`. Enable `storage` for persistent JSON preferences as well: the host supplies an application directory on native or a namespace in browser local storage. Shared screens use the same `use_async_storage` model and callbacks; the host chooses their task scheduler. See [`leptoskit_use`](../leptoskit_use/readme.md) for storage contracts, error handling, and examples.

## Shared application workspaces

`leptoskit new amazing` creates one `amazing_app` crate for web SSR, browser Wasm, and native Wasm. `amazing_backend` owns server-only RPC implementations. `amazing_shared` owns shared contracts and domain types. Platform startup and packaging live in top-level `web/`, `ios/`, `android/`, `macos/`, `windows/`, and `linux/` folders. All six destinations are enabled by default.

The facade exposes platform features `web`, `ios`, `android`, `macos`, `windows`, and `linux`. The generated app forwards its destination feature to the facade and selects one destination per build. Native app features also enable `leptoskit/guest` for the Wasm UI adapter. A platform feature identifies the app's destination even when Rust's compilation target is Wasm.

Native backend dependencies are gated by the actual OS target. `ios`, `macos`, and `linux` also name the existing `uikit`, `appkit`, and `gtk` backend features. OS APIs belong in platform hosts or plugins. Optional dependencies inside the shared app must support its actual compilation target.

## Platform module overrides

The generated app calls `leptoskit_build::platforms()` from `build.rs` and declares its home route with:

```rust
leptoskit::platform_module!(pub mod home = "routes/home");
```

The path is relative to the app's `src` directory. Keep `routes/home.rs` as the shared implementation. Add `routes/home.ios.rs` to replace that module on iOS, or use the corresponding suffix for another platform. Callers keep importing `crate::routes::home`. Ordinary `mod` declarations keep Rust's standard module resolution.

See [`leptoskit_build`](../leptoskit_build/readme.md) for build-script setup and [creating a project](https://ifiokjr.github.io/leptoskit/guide/02-creating.html) for the workspace and build commands.
