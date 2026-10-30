//! The browser screen: rendering, queries, waits, and actions over real DOM.
#![cfg(target_arch = "wasm32")]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptos::prelude::Get;
use leptos::prelude::GetUntracked;
use leptos::prelude::RwSignal;
use leptos::prelude::Set;
use leptos::prelude::Update;
use leptos::prelude::on_cleanup;
use leptos::prelude::set_timeout;
use leptoskit::prelude::component;
use leptoskit::view;
use leptoskit_native::Element;
use leptoskit_native::components::*;
use leptoskit_testing_library::Finder;
use leptoskit_testing_library::NativeKind;
use leptoskit_testing_library::Queries;
use leptoskit_testing_library::Role;
use leptoskit_testing_library::TestError;
use leptoskit_testing_library::render;
use wasm_bindgen::JsCast;
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
async fn a_tap_re_renders_a_view_macro_component() -> Result<(), TestError> {
	let count = RwSignal::new(0);
	let screen = render(move || view! { <Counter count=count /> });

	screen.get_by_text("Count: 0")?;
	screen
		.tap(&Finder::aria_role(Role::Button, "Increment"))
		.await?;
	screen.get_by_text("Count: 1")?;
	assert!(screen.query_by_text("Count: 0")?.is_none());
	assert_eq!(count.get_untracked(), 1);
	Ok(())
}

fn inbox() -> Element {
	View()
		.child(Heading(1, "Inbox"))
		.child(Text("3 unread"))
		.child(Button("Archive").test_id("archive"))
		.child(Button("x").accessibility_label("Close"))
		.child(Button("Delete"))
		.child(Button("Delete"))
		.child(Switch(false).accessibility_label("Notifications"))
		.child(TextInput().accessibility_label("Search"))
		.child(Pressable("Open profile").child(Text("Profile")))
		.child(Text("Display none").class("hidden"))
		.child(Text("Muted").with_aria(|aria| aria.hidden = Some(true)))
}

#[wasm_bindgen_test]
fn queries_match_text_labels_roles_and_test_ids() -> Result<(), TestError> {
	let screen = render(inbox);

	let heading = screen.get_by_text("Inbox")?;
	assert_eq!(heading.local_name(), "h1");
	assert_eq!(screen.get_by_role(Role::Heading, "Inbox")?, heading);

	let archive = screen.get_by_test_id("archive")?;
	assert_eq!(archive.local_name(), "button");
	assert_eq!(screen.get_by_text("Archive")?, archive);
	assert_eq!(screen.get_by_role(Role::Button, "Archive")?, archive);
	assert_eq!(
		screen.get(&Finder::role(NativeKind::Button, "Archive"))?,
		archive
	);

	let close = screen.get_by_label("Close")?;
	assert_eq!(screen.get_by_role(Role::Button, "Close")?, close);
	assert!(
		screen.query_by_role(Role::Button, "x")?.is_none(),
		"a label replaces the text as the accessible name"
	);

	let switch = screen.get_by_role(Role::Switch, "Notifications")?;
	assert_eq!(
		screen.get(&Finder::role(NativeKind::Switch, "Notifications"))?,
		switch
	);
	assert!(
		screen
			.query(&Finder::role(NativeKind::Checkbox, "Notifications"))?
			.is_none()
	);
	assert_eq!(
		screen.get_by_role(Role::TextBox, "Search")?.local_name(),
		"input"
	);

	let profile = screen.get_by_role(Role::Button, "Open profile")?;
	let content = screen.get_by_text("Profile")?;
	assert_ne!(content, profile, "the deepest element owns the text");
	assert!(profile.contains(Some(&content)));

	assert!(
		screen.query(&Finder::role(NativeKind::View, ""))?.is_none(),
		"kinds without an implied role match nothing"
	);
	assert_eq!(screen.query_all_by_role(Role::Button, "Delete").len(), 2);
	Ok(())
}

#[wasm_bindgen_test]
fn matches_come_back_in_document_order() -> Result<(), TestError> {
	let screen = render(|| {
		View()
			.role(Role::List)
			.children(["Hi", "how", "are", "you?"].map(|word| {
				View()
					.role(Role::ListItem)
					.test_id("word")
					.child(Text(word))
			}))
	});

	let words = screen
		.get_all_by_test_id("word")?
		.iter()
		.map(web_sys::HtmlElement::inner_text)
		.collect::<Vec<_>>()
		.join(" ");
	assert_eq!(words, "Hi how are you?");
	assert_eq!(
		screen.get_by_role(Role::ListItem, "you?")?.local_name(),
		"li"
	);
	Ok(())
}

#[wasm_bindgen_test]
fn hidden_elements_never_match() -> Result<(), TestError> {
	let screen = render(inbox);

	assert!(screen.query_by_text("Display none")?.is_none());
	assert!(screen.query_by_text("Muted")?.is_none());
	assert!(screen.container().inner_html().contains("Display none"));
	Ok(())
}

#[wasm_bindgen_test]
fn failed_queries_explain_themselves_with_the_markup() {
	let screen = render(inbox);

	let missing = screen.get_by_text("Spam").unwrap_err();
	assert_eq!(
		missing.reason,
		"expected one match for Text(\"Spam\"), found 0"
	);
	assert!(missing.tree.contains("Inbox</h1>"), "{}", missing.tree);
	assert!(!missing.tree.contains("<style"), "stylesheets are noise");
	assert!(
		!missing.tree.contains("data-lk-style"),
		"style scopes are noise"
	);
	assert!(
		!missing.tree.contains("<!--"),
		"hydration markers are noise"
	);
	assert!(
		!missing.tree.contains("class=\"\""),
		"empty classes are noise"
	);

	let several = screen.get_by_text("Delete").unwrap_err();
	assert_eq!(
		several.reason,
		"expected one match for Text(\"Delete\"), found 2"
	);

	let ambiguous = screen.query_by_text("Delete").unwrap_err();
	assert_eq!(
		ambiguous.reason,
		"expected at most one match for Text(\"Delete\"), found 2"
	);

	let none = screen.get_all_by_label("Spam").unwrap_err();
	assert_eq!(
		none.reason,
		"expected at least one match for Label(\"Spam\"), found 0"
	);
	assert_eq!(
		screen.get_all_by_text("Delete").map(|all| all.len()).ok(),
		Some(2)
	);
	assert_eq!(
		screen.query_all_by_test_id("spam"),
		Vec::<web_sys::HtmlElement>::new()
	);
}

#[wasm_bindgen_test]
async fn find_by_waits_for_content_that_arrives_later() -> Result<(), TestError> {
	let loaded = RwSignal::new(false);
	let screen = render(move || {
		if loaded.get() {
			Text("Loaded")
		} else {
			Text("Loading")
		}
	});
	set_timeout(move || loaded.set(true), Duration::from_millis(50))
		.expect("the browser schedules the timeout");

	assert!(screen.query_by_text("Loaded")?.is_none());
	let element = screen.find_by_text("Loaded").await?;
	assert_eq!(element.text_content().as_deref(), Some("Loaded"));
	assert_eq!(screen.find_all_by(&Finder::text("Loaded")).await?.len(), 1);
	Ok(())
}

#[wasm_bindgen_test]
async fn find_by_times_out_and_rejects_duplicates_at_once() {
	let screen = render(inbox).with_timeout(Duration::from_millis(50));

	let timeout = screen.find_by_text("Never").await.unwrap_err();
	assert_eq!(
		timeout.reason,
		"Text(\"Never\") did not appear within 50 ms"
	);
	assert!(timeout.tree.contains("Inbox</h1>"));

	let several = screen
		.find_by_role(Role::Button, "Delete")
		.await
		.unwrap_err();
	assert!(several.reason.ends_with("found 2"), "{}", several.reason);
	assert_eq!(
		screen
			.find_all_by(&Finder::aria_role(Role::Button, "Delete"))
			.await
			.map(|all| all.len())
			.ok(),
		Some(2)
	);
	assert!(screen.find_by_label("Close").await.is_ok());
	assert!(screen.find_by_test_id("archive").await.is_ok());
}

#[wasm_bindgen_test]
async fn actions_refuse_disabled_controls_and_wrong_kinds() {
	let pressed = Arc::new(AtomicBool::new(false));
	let recorder = Arc::clone(&pressed);
	let screen = render(move || {
		let recorder = Arc::clone(&recorder);
		View()
			.child(
				Button("Save")
					.enabled(false)
					.on_press(move || recorder.store(true, Ordering::SeqCst)),
			)
			.child(
				View()
					.enabled(false)
					.child(Switch(false).accessibility_label("Locked")),
			)
			.child(Button("Plain"))
	});

	let save = screen.tap(&Finder::text("Save")).await.unwrap_err();
	assert_eq!(save.reason, "Text(\"Save\") is disabled");
	assert!(!pressed.load(Ordering::SeqCst));

	let locked = screen.toggle(&Finder::label("Locked")).await.unwrap_err();
	assert_eq!(locked.reason, "Label(\"Locked\") is disabled");

	let toggle = screen.toggle(&Finder::text("Plain")).await.unwrap_err();
	assert_eq!(
		toggle.reason,
		"Text(\"Plain\") is not a switch, checkbox, or radio"
	);

	let typing = screen
		.enter_text(&Finder::text("Plain"), "Hello")
		.await
		.unwrap_err();
	assert_eq!(typing.reason, "Text(\"Plain\") is not a text input");

	let missing = screen.tap(&Finder::text("Missing")).await.unwrap_err();
	assert!(missing.reason.ends_with("found 0"));
}

#[wasm_bindgen_test]
async fn enter_text_proposes_a_value_the_app_owns() -> Result<(), TestError> {
	let name = RwSignal::new(String::new());
	let screen = render(move || {
		View()
			.child(
				TextInput()
					.accessibility_label("Name")
					.value(name.get())
					.on_change(move |value| name.set(value.to_owned())),
			)
			.child(Text(format!("Hello, {}", name.get())))
			.child(
				TextInput()
					.accessibility_label("Fixed")
					.value("fixed")
					.on_change(|_| {}),
			)
	});

	screen.enter_text(&Finder::label("Name"), "Ada").await?;
	screen.get_by_text("Hello, Ada")?;
	let input = screen
		.get_by_label("Name")?
		.unchecked_into::<web_sys::HtmlInputElement>();
	assert_eq!(input.value(), "Ada");

	screen
		.enter_text(&Finder::label("Fixed"), "changed")
		.await?;
	let fixed = screen
		.get_by_label("Fixed")?
		.unchecked_into::<web_sys::HtmlInputElement>();
	assert_eq!(fixed.value(), "fixed", "a rejected proposal is restored");
	Ok(())
}

#[wasm_bindgen_test]
async fn toggle_proposes_the_opposite_state() -> Result<(), TestError> {
	let on = RwSignal::new(false);
	let screen = render(move || {
		View()
			.child(
				Switch(on.get())
					.accessibility_label("Wi-Fi")
					.on_toggle(move |checked| on.set(checked)),
			)
			.child(Text(if on.get() { "On" } else { "Off" }))
	});

	let wifi = Finder::aria_role(Role::Switch, "Wi-Fi");
	screen.toggle(&wifi).await?;
	screen.get_by_text("On")?;
	let input = screen
		.get(&wifi)?
		.unchecked_into::<web_sys::HtmlInputElement>();
	assert!(input.checked());

	screen.toggle(&wifi).await?;
	screen.get_by_text("Off")?;
	assert!(!on.get_untracked());
	Ok(())
}

#[wasm_bindgen_test]
fn dropping_a_screen_unmounts_it_and_removes_its_container() {
	let body = web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.body())
		.expect("a document body");
	let before = body.child_element_count();
	let disposed = Arc::new(AtomicBool::new(false));
	let cleanup = Arc::clone(&disposed);
	let screen = render(move || {
		let cleanup = Arc::clone(&cleanup);
		on_cleanup(move || cleanup.store(true, Ordering::SeqCst));
		Button("Temporary")
	});

	assert_eq!(body.child_element_count(), before + 1);
	assert!(!disposed.load(Ordering::SeqCst));
	drop(screen);
	assert_eq!(body.child_element_count(), before);
	assert!(disposed.load(Ordering::SeqCst));
}
