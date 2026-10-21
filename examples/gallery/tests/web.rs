//! The gallery's conformance scenario in a real browser: every screen mounts
//! as DOM, and clicking the counter's `<button>` updates the shared count
//! without replacing the element. Run with `test:browser`.
#![cfg(all(feature = "web", target_arch = "wasm32"))]

use gallery::Assets;
use gallery::SCREENS;
use gallery::Screen;
use gallery::ScreenState;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::task::tick;
use leptoskit::web::render;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn container() -> web_sys::HtmlElement {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.expect("a browser document");
	let parent = document
		.create_element("div")
		.expect("a container")
		.dyn_into::<web_sys::HtmlElement>()
		.expect("an HTML container");
	document
		.body()
		.expect("a body")
		.append_child(&parent)
		.expect("the container attaches");
	parent
}

/// Mounts `screen` like `gallery-web`: state changes re-render it.
fn mount(screen: &'static Screen, parent: &web_sys::HtmlElement) -> (ScreenState, impl Drop) {
	let revision = RwSignal::new(0_u64);
	let assets = Assets::for_browser().expect("the gallery artwork encodes");
	let state = ScreenState::new(assets).on_change(move || revision.update(|value| *value += 1));
	let rendered = state.clone();
	let handle = mount_to(parent.clone(), move || {
		move || {
			let _ = revision.get();
			render(screen.render(&rendered))
		}
	});
	(state, handle)
}

fn buttons(parent: &web_sys::HtmlElement) -> Vec<web_sys::HtmlElement> {
	let found = parent
		.query_selector_all("button")
		.expect("a valid selector");
	(0..found.length())
		.filter_map(|index| found.item(index))
		.filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
		.collect()
}

fn button(parent: &web_sys::HtmlElement, label: &str) -> web_sys::HtmlElement {
	buttons(parent)
		.into_iter()
		.find(|button| button.text_content().as_deref().map(str::trim) == Some(label))
		.unwrap_or_else(|| panic!("no `{label}` button"))
}

#[wasm_bindgen_test]
async fn every_screen_mounts_as_dom() {
	for screen in &SCREENS {
		let parent = container();
		let (_state, handle) = mount(screen, &parent);
		tick().await;
		let elements = parent
			.query_selector_all("*")
			.expect("a valid selector")
			.length();
		assert!(
			elements > 10,
			"`{}` mounted only {elements} elements",
			screen.slug
		);
		let text = parent.text_content().unwrap_or_default();
		assert!(!text.trim().is_empty(), "`{}` shows no text", screen.slug);
		drop(handle);
		parent.remove();
	}
}

#[wasm_bindgen_test]
async fn clicking_the_counter_updates_the_shared_count() {
	let parent = container();
	let screen = gallery::find_screen("counter").expect("the counter screen");
	let (state, handle) = mount(screen, &parent);
	tick().await;
	let add = button(&parent, "Add one");

	add.click();
	tick().await;
	add.click();
	tick().await;

	let expected = ScreenState::INITIAL_COUNT + 2;
	assert_eq!(state.count(), expected);
	assert!(
		parent
			.text_content()
			.unwrap_or_default()
			.contains(&expected.to_string()),
		"the page shows the new count"
	);
	assert_eq!(
		button(&parent, "Add one"),
		add,
		"re-rendering kept the button element"
	);
	drop(handle);
	parent.remove();
}
