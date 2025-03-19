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

// We need to use unit_tests feature because wasm_pack can only run either an
// integration test or unit_tests at once?
#[cfg(all(test, not(target_arch = "wasm32")))]
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
		if results.len() != 1 {
			panic!(
				"{}",
				results
					.into_iter()
					.map(|n| n.inner_html())
					.collect::<Vec<String>>()
					.join("\nSEP\n")
			)
		}
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
		if results.len() != 1 {
			panic!(
				"{}",
				results
					.into_iter()
					.map(|n| n.inner_html())
					.collect::<Vec<String>>()
					.join("\nSEP\n")
			)
		}
	}
}
