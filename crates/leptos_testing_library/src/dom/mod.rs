use derive_more::Deref;
use derive_more::DerefMut;
use derive_more::From;
use derive_more::Into;
pub use dom_query::*;
pub use element_wrapper::*;
pub use error::*;
use internal::*;
pub use test_element::*;
use thiserror::Error;
use wasm_bindgen::JsCast;
use web_sys::Element;
use web_sys::HtmlElement;
use web_sys::Node;

mod dom_query;
mod element_wrapper;
mod error;
mod internal;
mod test_element;

pub mod prelude {
	pub use super::DomQuery;
	pub use super::ElementWrapper;
	pub use super::HoldsElement;
	pub use super::TestElement;
	pub use super::TestingLibraryErrorTrait;
}

// These touch the DOM, so they only build and run under wasm-bindgen-test in a
// browser.
#[cfg(all(test, target_arch = "wasm32"))]
pub mod test {
	use wasm_bindgen_test::*;

	use super::*;
	wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen_test]
	pub fn find_parents_of_matching_text() {
		let document = leptos::prelude::document();
		let wrapper: Element = document.create_element("div").unwrap();
		let div = document.create_element("div").unwrap();
		div.clone()
			.unchecked_into::<HtmlElement>()
			.set_inner_text("hello");
		wrapper.append_child(&div.into()).unwrap();
		document
			.body()
			.unwrap()
			.append_child(&wrapper.clone().into())
			.unwrap();
		let text_nodes = get_all_text_nodes(&document);
		let results = text_nodes.find_parents_of_matching_text("hello");
		assert!(
			(results.len() == 1),
			"{}",
			results
				.into_iter()
				.map(|n| n.inner_html())
				.collect::<Vec<String>>()
				.join("\nSEP\n")
		);
	}
	#[wasm_bindgen_test]
	pub fn find_parents_of_containing_text() {
		let document = leptos::prelude::document();
		let wrapper: Element = document.create_element("div").unwrap();
		let div = document.create_element("div").unwrap();
		div.clone()
			.unchecked_into::<HtmlElement>()
			.set_inner_text("other");
		wrapper.append_child(&div.into()).unwrap();
		document
			.body()
			.unwrap()
			.append_child(&wrapper.into())
			.unwrap();
		let text_nodes = get_all_text_nodes(&document);
		let results = text_nodes.find_parents_of_containing_text("other");
		assert!(
			(results.len() == 1),
			"{}",
			results
				.into_iter()
				.map(|n| n.inner_html())
				.collect::<Vec<String>>()
				.join("\nSEP\n")
		);
	}

	#[wasm_bindgen_test]
	pub fn text_split_across_nodes_matches_its_element_once() {
		let document = leptos::prelude::document();
		let span = document.create_element("span").unwrap();
		span.append_child(&document.create_text_node("Split count: "))
			.unwrap();
		span.append_child(&document.create_text_node("0")).unwrap();
		document
			.body()
			.unwrap()
			.append_child(&span.clone().into())
			.unwrap();
		let text_nodes = get_all_text_nodes(&span);
		assert_eq!(
			text_nodes
				.find_parents_of_matching_text("Split count: 0")
				.len(),
			1
		);
		assert_eq!(
			text_nodes
				.find_parents_of_containing_text("Split count: 0")
				.len(),
			1
		);
	}
}
