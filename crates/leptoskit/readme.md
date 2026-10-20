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

The [component catalog](https://ifiokjr.github.io/leptoskit/reference/components.html) contains 45 builders, including flex and grid layouts, stacks, cards, eager collections, and controlled native inputs. Every builder returns the same `Element` vocabulary for native and web. They are functions, not `#[component]` functions or `<View>` tags.

Commit the tree to a `NativeSurface`, or enable the optional `web` feature and pass the same tree to `leptoskit::web::render`. The web adapter creates Leptos views and emits CSS. Native rendering applies the same utility declarations to Yoga and widget props. No Tailwind installation or Node build is needed.

`.class` appends validated utilities and panics on invalid input. `.try_class` returns a `ClassError` for dynamic input. Classes override typed base styles regardless of builder call order. Native surfaces reapply responsive and dark rules when the host updates the viewport or calls `set_dark_mode`.

Read the [shared component guide](https://ifiokjr.github.io/leptoskit/guide/07-shared-components.html) and [CSS coverage reference](https://ifiokjr.github.io/leptoskit/reference/css-coverage.html). CSS support is a defined subset. Unsupported utilities and variants return diagnostics.

Enable `use` for platform-neutral reactive hooks under `leptoskit::hooks`. Enable `storage` for persistent JSON preferences as well: the host supplies an application directory on native or a namespace in browser local storage. Shared screens use the same `use_async_storage` model and callbacks; the host chooses their task scheduler. See [`leptoskit_use`](../leptoskit_use/readme.md) for storage contracts, error handling, and examples.
