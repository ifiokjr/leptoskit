<h1 align="center">leptoskit</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit</strong> is the crate an app depends on: it re-exports the shared components, Tailwind styles, renderers, and runtime behind one feature per optional part.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit.svg"></a>
  <a href="https://docs.rs/leptoskit"><img alt="docs.rs" src="https://docs.rs/leptoskit/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

One dependency, one CLI, every target:

- **Web** — SSR, hydration, and wasm builds, served by Axum.
- **Native** — real native widgets driven by a wasm bundle, no web view: the React Native architecture, rewritten in Rust.
- **Over the air** — code push for the wasm bundle, with binary deltas.
- **Server functions** — leptos fns with an ergonomic retrying API.

Build shared screens with the built-in Rust components and Tailwind classes.

The [component catalog](https://leptoskit.dev/docs/reference/components.html) has 16 primitive builders (including optional `Rive`) for containers, text, images, scrolling, and native controls. Tailwind expresses layout and visual composition. Another 31 composition helpers remain available through an explicit `components::helpers` import. Every builder returns the same `Element` vocabulary for native and web. Use them as PascalCase tags in `view!`; builders remain available for direct tree manipulation.

Commit the tree to a `NativeSurface`, or enable the optional `web` feature and pass the same tree to `leptoskit::web::render`. The web adapter creates Leptos views and emits CSS. Native rendering applies the same utility declarations to Yoga and widget props. No Tailwind installation or Node build is needed.

`.class` appends validated utilities and panics on invalid input. `.try_class` returns a `ClassError` for dynamic input. Classes override typed base styles regardless of builder call order. Native surfaces reapply responsive and dark rules when the host updates the viewport or calls `set_dark_mode`.

Read the [shared component guide](https://leptoskit.dev/docs/guide/07-shared-components.html) and [CSS coverage reference](https://leptoskit.dev/docs/reference/css-coverage.html). CSS support is a defined subset. Unsupported utilities and variants return diagnostics.

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

See [`leptoskit_build`](../leptoskit_build/readme.md) for build-script setup and [creating a project](https://leptoskit.dev/docs/guide/02-creating.html) for the workspace and build commands.

## Generated host limits

<!-- {=generatedHostLimits} -->

Generated macOS, iOS, and Android apps use the Swift or Kotlin renderers in `leptoskit_bridge_ffi`. They support `View`, `Text`, `Button`, `Pressable`, `TextInput`, `PasswordInput`, `ScrollView`, `Image`, `Vector`, and `Rive`. Other control kinds fail with an unsupported-widget error. Accessible names are forwarded; a labelled `Vector` is an accessible image and `Rive` can be decorative. Most other roles and states are not forwarded. The Rust backend crates support a wider control set. Generated Windows and Linux apps use the full Rust backends, but currently omit runtime plugin registration. Automatic code push update checks currently run only in generated iOS and Android hosts.

<!-- {/generatedHostLimits} -->

## How it fits

<!-- {=kitSummary} -->

Start here. A web build enables `web` and renders through leptos; a native build enables a platform backend and commits the same tree as real widgets. Features add routing, queries, forms, localization, database, auth, extensions, and `DevTools` only when the app asks for them, so the meta crate stays a front door rather than a bundle of everything.

<!-- {/kitSummary} -->

## Use it

```rust
use leptoskit::prelude::*;

#[component]
fn Screen() -> Element {
	view! {
		<View class="gap-4 p-6 md:flex-row">
			<Text class="text-2xl font-bold">"Hello, leptoskit"</Text>
			<Button class="rounded-lg bg-indigo-500 px-4 py-2 text-white">"Continue"</Button>
		</View>
	}
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Getting started](https://leptoskit.dev/docs/guide/01-installation.html)
- [Component catalog](https://leptoskit.dev/docs/reference/components.html)
