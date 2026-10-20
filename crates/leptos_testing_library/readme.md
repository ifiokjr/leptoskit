# leptos_testing_library

> Testing Library style DOM queries for leptos components.

Mount a component with `render_for_test`, then query it the way a user finds things on the page: by text, by label, or by id. The queries run in a real browser through `wasm-bindgen-test`.

```rust,ignore
use leptos::prelude::*;
use leptos_testing_library::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn increments() {
	let render = render_for_test(|| {
		let count = RwSignal::new(0);
		view! {
			<button id="increment" on:click=move |_| count.update(|c| *c += 1)>
				"Increment"
			</button>
			<span>"Count: "{move || count.get()}</span>
		}
	});

	render.get_by_id("increment").unwrap().click();
	leptos::task::tick().await;

	assert!(render.get_by_text("Count: 1").is_ok());
}
```

Run the suite in Chrome with `devenv shell test:browser`.

Part of [leptoskit](https://github.com/ifiokjr/leptoskit).
