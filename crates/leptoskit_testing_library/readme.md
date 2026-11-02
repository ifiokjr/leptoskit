<h1 align="center">leptoskit_testing_library</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_testing_library</strong> provides Testing Library queries and widget-test actions for leptoskit components mounted in a real browser.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_testing_library"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_testing_library"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_testing_library.svg"></a>
  <a href="https://docs.rs/leptoskit_testing_library"><img alt="docs.rs" src="https://docs.rs/leptoskit_testing_library/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

`render` mounts a component with the leptoskit web renderer, exactly as a leptoskit web app does, and returns a `Screen`. Find what a user would find, by text, label, role, or test ID, act on it, and assert on what changed. Tests run in Chrome or Firefox through `wasm-bindgen-test`.

- **Queries.** `get_by_*`, `get_all_by_*`, `query_by_*`, and `query_all_by_*`, each by text, label, role, and test ID, with Testing Library's rules for none and several matches. `find_by_*` waits for a match to appear, one second by default (`Screen::with_timeout`).
- **Actions.** `tap`, `enter_text`, and `toggle` find exactly one enabled control, dispatch the browser events a user's click or typing would, and pump leptos so the next assertion sees the result. `pump` settles leptos on its own.
- **Diagnostics.** Hidden elements never match. Every failure is a `TestError` with the rendered markup, minus stylesheets and leptos's markers.
- **One vocabulary.** `Finder` and `Queries` come from [`leptoskit_testing`](../leptoskit_testing), whose `WidgetTester` runs the same assertions headless against the native renderer. `Element::test_id` marks an element for both; the web renders it as `data-testid`.

Run the suite in Chrome with `devenv shell test:browser`, or `leptoskit test web --package leptoskit_testing_library`.

## How it fits

It mounts through the web renderer and shares finders with `leptoskit_testing`, so a team can test the browser build with the same vocabulary it uses for native widget tests.

## Use it

```rust,ignore
use leptos::prelude::{Get, RwSignal, Update};
use leptoskit::prelude::{Button, Element, Text, View, component};
use leptoskit::view;
use leptoskit_testing_library::{Finder, Queries, Role, TestError, render};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[component]
fn Counter(count: RwSignal<u32>) -> Element {
	view! {
		<View>
			<Text>{format!("Count: {}", count.get())}</Text>
			<Button on:press=move || count.update(|count| *count += 1)>"Increment"</Button>
		</View>
	}
}

#[wasm_bindgen_test]
async fn increments() -> Result<(), TestError> {
	let count = RwSignal::new(0);
	let screen = render(move || view! { <Counter count=count /> });

	screen.get_by_text("Count: 0")?;
	screen.tap(&Finder::aria_role(Role::Button, "Increment")).await?;
	screen.get_by_text("Count: 1")?;
	Ok(())
}
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_testing_library)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Testing](https://ifiokjr.github.io/leptoskit/guide/08-testing.html)
