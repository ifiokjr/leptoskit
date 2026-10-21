use leptos::prelude::*;
use leptos::task::tick;
use leptos_testing_library::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
pub fn dropping_a_render_removes_its_dom_and_disposes_its_owner() {
	let body = document().body().unwrap();
	let before = body.child_element_count();
	let disposed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	let cleanup = std::sync::Arc::clone(&disposed);
	let render = render_for_test(move || {
		on_cleanup(move || cleanup.store(true, std::sync::atomic::Ordering::SeqCst));
		view! { <button>"Temporary"</button> }
	});
	assert_eq!(body.child_element_count(), before + 1);
	drop(render);
	assert_eq!(body.child_element_count(), before);
	assert!(disposed.load(std::sync::atomic::Ordering::SeqCst));
}

#[wasm_bindgen_test]
pub async fn find_by_id() {
	let render = render_for_test(|| {
		let count = RwSignal::new(0);
		view! {
			<button on:click=move |_| count.update(|c| *c += 1)>"Increment"</button>
			<div id="output">{move || count.get()}</div>
		}
	});

	render
		// The get_by_X method series return a struct that derefs into web_sys::HtmlElement
		.get_by_text("Increment")
		.unwrap()
		// So we can just click it!
		.click();

	tick().await;

	assert_eq!(
		render
			.get_by_id("output")
			.unwrap()
			.parse::<usize>()
			.unwrap(),
		1
	);
}

#[wasm_bindgen_test]
pub fn handle_errors() {
	let render = render_for_test(|| {
		view! {
			<ul>
				<li id="ghost_noises_1">Boo</li>
				<li id="ghost_noises_2">Boo</li>
			</ul>
		}
	});
	assert!(render.get_by_id_contains("ghost_noises").is_more_than_one());
	assert!(render.get_by_id("shark_noise").is_not_found());
}

#[wasm_bindgen_test]
pub fn iterate_list() {
	let render = render_for_test(|| {
		view! {
			<ul>
				<li id="list_item_1">Hi</li>
				<li id="list_item_2">how</li>
				<li id="list_item_3">are</li>
				<li id="list_item_4">you?</li>
			</ul>
		}
	});
	let questions = render
		// The get_all_by_X method series return a Vec<TestElement> which derefs into HtmlElement
		// but has helper functions describing the behavior of your app in a way that describes the
		// usage of your app.
		.get_all_by_id_contains("list_item")
		.into_iter()
		.map(|test_element| test_element.inner_text())
		.collect::<Vec<String>>()
		.join(" ");

	assert_eq!(questions, String::from("Hi how are you ?"));
}

#[wasm_bindgen_test]
pub fn text_split_across_nodes_is_one_match() {
	let render = render_for_test(|| {
		let count = RwSignal::new(0);
		view! { <span>"Count: "{move || count.get()}</span> }
	});

	assert!(render.get_by_text("Count: 0").is_ok());
	assert!(render.get_by_text_contains("Count: 0").is_ok());
}

#[wasm_bindgen_test]
pub fn labels_without_targets_are_not_found() {
	let render = render_for_test(|| {
		view! {
			<p>"Nickname"</p>
			<label>"Nickname"</label>
			<label for="missing_field">"Nickname"</label>
		}
	});

	assert!(render.get_all_by_label("Nickname").is_empty());
	assert!(render.get_by_label_contains("Nick").is_not_found());
}
