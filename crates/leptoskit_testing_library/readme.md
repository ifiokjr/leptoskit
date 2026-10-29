<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_testing_library

> Testing Library queries and widget-test actions for leptoskit components in a real browser.

`render` mounts a component with the leptoskit web renderer, exactly as a leptoskit web app does, and returns a `Screen`. Find what a user would find, by text, label, role, or test ID, act on it, and assert on what changed. Tests run in Chrome or Firefox through `wasm-bindgen-test`.

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

- **Queries.** `get_by_*`, `get_all_by_*`, `query_by_*`, and `query_all_by_*`, each by text, label, role, and test ID, with Testing Library's rules for none and several matches. `find_by_*` waits for a match to appear, one second by default (`Screen::with_timeout`).
- **Actions.** `tap`, `enter_text`, and `toggle` find exactly one enabled control, dispatch the browser events a user's click or typing would, and pump leptos so the next assertion sees the result. `pump` settles leptos on its own.
- **Diagnostics.** Hidden elements never match. Every failure is a `TestError` with the rendered markup, minus stylesheets and leptos's markers.
- **One vocabulary.** `Finder` and `Queries` come from [`leptoskit_testing`](../leptoskit_testing), whose `WidgetTester` runs the same assertions headless against the native renderer. `Element::test_id` marks an element for both; the web renders it as `data-testid`.

Run the suite in Chrome with `devenv shell test:browser`, or `leptoskit test web --package leptoskit_testing_library`.

Part of [leptoskit](https://github.com/ifiokjr/leptoskit).
