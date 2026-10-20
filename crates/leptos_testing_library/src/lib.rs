//! Testing Library style DOM queries for leptos components.
//!
//! [`render_for_test`] mounts a component into the document, and the returned
//! render exposes queries such as `get_by_id`, `get_by_text`, and
//! `get_all_by_label`. The queries run in a real browser through
//! `wasm-bindgen-test`, so they see the same DOM your users do.

use dom::prelude::*;
use leptos::IntoView;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

/// Renders a Leptos component for testing purposes.
///
/// This function creates a wrapper div element in the document body, mounts the
/// provided component to it, and returns a `LeptosTestingLibraryRender` that
/// can be used to interact with and test the component.
///
/// # Examples
///
/// ```
/// # #[cfg(target_arch = "wasm32")]
/// # mod hidden_example {
/// use leptos::prelude::*;
/// use leptos_testing_library::prelude::*;
/// use wasm_bindgen_test::*;
///
/// wasm_bindgen_test_configure!(run_in_browser);
///
/// #[wasm_bindgen_test]
/// async fn test_counter_component() {
/// 	// Render a counter component for testing
/// 	let render = render_for_test(|| {
/// 		let count = RwSignal::new(0);
/// 		view! {
/// 			<div>
/// 				<button id="increment" on:click=move|_| count.update(|c| *c += 1)>
/// 					"Increment"
/// 				</button>
/// 				<span id="count">{move || count.get()}</span>
/// 			</div>
/// 		}
/// 	});
///
/// 	// Interact with the component
/// 	render.get_by_id("increment").unwrap().click();
///
/// 	// Let the reactive graph settle before asserting
/// 	leptos::task::tick().await;
///
/// 	// Assert the expected state
/// 	assert_eq!(
/// 		render.get_by_id("count").unwrap().text_content().unwrap(),
/// 		"1"
/// 	);
/// }
/// # }
/// ```
///
/// You can also test more complex components and use various query methods:
///
/// ```
/// # #[cfg(target_arch = "wasm32")]
/// # mod hidden_example {
/// use leptos::prelude::*;
/// use leptos_testing_library::prelude::*;
/// use wasm_bindgen_test::*;
///
/// wasm_bindgen_test_configure!(run_in_browser);
///
/// #[wasm_bindgen_test]
/// fn test_todo_list() {
/// 	let render = render_for_test(|| {
/// 		view! {
/// 			<div>
/// 				<h1>"Todo List"</h1>
/// 				<ul>
/// 					<li data-testid="todo-item">"Buy milk"</li>
/// 					<li data-testid="todo-item">"Walk the dog"</li>
/// 				</ul>
/// 			</div>
/// 		}
/// 	});
///
/// 	// Find elements by their text content
/// 	assert!(render.get_by_text("Buy milk").is_ok());
/// 	assert!(render.get_by_text("Walk the dog").is_ok());
///
/// 	// And by exact heading text
/// 	let heading = render.get_by_text("Todo List").unwrap();
/// 	assert_eq!(heading.tag_name().to_lowercase(), "h1");
/// }
/// # }
/// ```
pub fn render_for_test<F, N>(f: F) -> LeptosTestingLibraryRender<N>
where
	F: FnOnce() -> N + 'static,
	N: IntoView,
{
	let body = document().body().unwrap();
	let test_wrapper = document().create_element("div").unwrap();
	body.append_child(&test_wrapper).unwrap();
	let unmount = mount_to(test_wrapper.clone().unchecked_into::<HtmlElement>(), f);

	LeptosTestingLibraryRender {
		unmount,
		element: test_wrapper,
	}
}

pub struct LeptosTestingLibraryRender<N: IntoView> {
	/// Dropping the render unmounts the component, so the handle is held even
	/// though nothing calls it directly.
	#[allow(dead_code)]
	unmount: UnmountHandle<N::State>,
	element: web_sys::Element,
}

impl<N: IntoView> HoldsElement for LeptosTestingLibraryRender<N> {
	fn element(&self) -> ElementWrapper<'_> {
		ElementWrapper(&self.element)
	}
}

pub mod dom;

pub mod prelude {
	pub use super::LeptosTestingLibraryRender;
	pub use super::dom::prelude::*;
	pub use super::render_for_test;
}
