//! The example in a real browser: links add history entries, the address bar
//! follows the route, and the browser's back button returns. Run with
//! `test:browser`.
#![cfg(all(feature = "web", target_arch = "wasm32"))]

use file_routes_example::routes::Navigation;
use file_routes_example::routes::Note;
use file_routes_example::routes::Route;
use leptos::mount::mount_to;
use leptos::task::tick;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn window() -> web_sys::Window {
	web_sys::window().expect("a browser window")
}

fn container() -> web_sys::HtmlElement {
	let document = window().document().expect("a document");
	let parent = document
		.create_element("div")
		.expect("a container")
		.dyn_into::<web_sys::HtmlElement>()
		.expect("an HTML container");
	document
		.body()
		.expect("a body")
		.append_child(&parent)
		.expect("attach the container");
	parent
}

/// Waits a browser task, long enough for a history change to arrive.
async fn settle() {
	let wait = js_sys::Promise::new(&mut |resolve, _| {
		window()
			.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 50)
			.expect("a timer");
	});
	JsFuture::from(wait).await.expect("the timer fires");
}

fn path() -> String {
	window().location().pathname().expect("a path")
}

#[wasm_bindgen_test]
async fn links_add_history_and_the_back_button_returns() {
	let history = window().history().expect("session history");
	// Arrive at /notes, the way a visitor following a link would.
	history
		.replace_state_with_url(&JsValue::NULL, "", Some("/notes"))
		.expect("start at /notes");
	let navigation = Navigation::from_browser().expect("/notes is a route");
	assert_eq!(navigation.current(), Route::Notes);
	assert!(!navigation.can_go_back());

	let parent = container();
	let render = navigation.clone();
	let mount = mount_to(parent.clone(), move || {
		move || leptoskit::web::render(render.render())
	});
	tick().await;
	let link = parent
		.query_selector("a[aria-label='Open note 2']")
		.expect("a selector")
		.expect("the link to note 2")
		.dyn_into::<web_sys::HtmlElement>()
		.expect("an HTML link");
	assert_eq!(link.get_attribute("href").as_deref(), Some("/notes/2"));

	link.click();
	tick().await;
	assert_eq!(path(), "/notes/2");
	assert_eq!(
		navigation.current(),
		Route::Note(Note {
			note: 2,
			highlight: None,
		})
	);
	assert!(parent.text_content().unwrap_or_default().contains("Note 2"));
	assert!(navigation.can_go_back());

	// The browser's back button: the route follows when the browser says so.
	assert!(navigation.back());
	settle().await;
	tick().await;
	assert_eq!(path(), "/notes");
	assert_eq!(navigation.current(), Route::Notes);
	assert!(!navigation.can_go_back());
	assert!(
		!navigation.back(),
		"the first app entry has nothing behind it"
	);

	// Replacing keeps the depth and rewrites the address bar.
	navigation.replace(Route::You);
	tick().await;
	assert_eq!(path(), "/you");
	assert!(!navigation.can_go_back());

	drop(mount);
	drop(navigation);
	parent.remove();
	history
		.replace_state_with_url(&JsValue::NULL, "", Some("/"))
		.expect("restore the page");
}
