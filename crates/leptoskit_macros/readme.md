<h1 align="center">leptoskit_macros</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_macros</strong> provides the `view!` markup macro and `#[component]` attribute that expand into the same builders a hand-written tree uses.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_macros"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_macros"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_macros.svg"></a>
  <a href="https://docs.rs/leptoskit_macros"><img alt="docs.rs" src="https://docs.rs/leptoskit_macros/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Use them through `leptoskit::prelude`.

- `view!` parses [rstml](https://github.com/rs-tml/rstml) markup and expands each tag into builder calls: `<Text class="p-4">"Hi"</Text>` becomes `Text::builder().class("p-4").children(…).build()`. The result is the same `Element` tree the builders make, so it renders on the web and on every native backend. Parsing recovers from errors, so rust-analyzer keeps completing inside half-written markup.
- `#[component]` keeps a function's signature and declares a props builder beside it, so `view!` can set props by name. `#[prop(optional)]`, `#[prop(optional_no_strip)]`, `#[prop(default = …)]`, and `#[prop(into)]` work as they do in leptos. A missing or repeated prop is a compile error that names it.

See the [`view!` and `#[component]` reference](https://ifiokjr.github.io/leptoskit/reference/view.html).

## Platform modules

Declare modules that need platform overrides through the facade:

```rust
leptoskit::platform_module!(pub mod profile = "routes/profile");
```

The path is relative to the application's `src` directory, including when the declaration is nested. `routes/profile.rs` is the required shared source. An optional sibling such as `routes/profile.ios.rs` replaces it for the `ios` application feature; imports keep using `profile` on every platform. The same convention supports `web`, `android`, `macos`, `windows`, and `linux`.

Call `leptoskit_build::platforms()` from the application's build script to select the platform and track source additions and deletions. Ordinary modules can continue using Rust's `mod` declarations. The macro emits a normal module declaration pointing at the selected source; it does not copy or rewrite the application.

Selected modules follow Rust's `#[path]` lookup rules. Inside `src/routes/profile.rs` or `profile.ios.rs`, a plain `mod details;` searches `src/routes/details.rs`, rather than `src/routes/profile/details.rs`. Register nested modules explicitly to retain the logical directory and allow their own platform overrides:

```rust
// Inside src/routes/profile.rs and its platform overrides:
leptoskit::platform_module!(pub mod details = "routes/profile/details");
```

This retains the original source file and module namespace: `self`, `super`, private parent items, inner attributes, and compiler source locations continue to work normally.

## How it fits

Use them through `leptoskit::prelude`. The macro parses rstml markup and emits builder calls, so markup and builder code produce one element tree and neither is a second-class path.

## Use it

```rust
use leptoskit::prelude::*;

#[component]
fn Greeting(#[prop(into)] name: String, #[prop(optional)] excited: bool) -> impl IntoView {
	view! {
		<View class="gap-2 p-4">
			<Text class="text-xl font-bold">"Hello, " {name}</Text>
			{excited.then(|| view! { <Text>"Welcome back!"</Text> })}
		</View>
	}
}

let screen = view! { <Greeting name="Ada" excited /> };
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_macros)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Shared components](https://ifiokjr.github.io/leptoskit/guide/07-shared-components.html)
- [The view! macro](https://ifiokjr.github.io/leptoskit/reference/view.html)
