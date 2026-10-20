#![cfg(all(feature = "web", target_arch = "wasm32"))]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptos::mount::hydrate_from;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::task::tick;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::web::render;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::NodeId;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn overflowing_decoration_does_not_extend_browser_pointer_hit_bounds() {
	let parent = container();
	parent.style().set_property("position", "fixed").unwrap();
	parent.style().set_property("top", "0").unwrap();
	parent.style().set_property("left", "0").unwrap();
	parent.style().set_property("z-index", "9999").unwrap();
	let mount = mount_to(parent.clone(), || {
		render(
			View().class("w-80 h-32").child(
				Pressable("Action")
					.class("w-20 h-10 overflow-visible")
					.child(Text("Overflow").class("absolute left-24 top-0 w-20 h-10")),
			),
		)
	});
	let button: web_sys::HtmlElement = find(&parent, "button");
	let frame = button.get_bounding_client_rect();
	let inside = document()
		.element_from_point((frame.x() + 20.0) as f32, (frame.y() + 20.0) as f32)
		.unwrap();
	let outside = document()
		.element_from_point((frame.x() + 120.0) as f32, (frame.y() + 20.0) as f32)
		.unwrap();
	assert_eq!(inside, button.clone().into());
	assert!(!button.contains(Some(&outside)));
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn rerenders_refresh_callbacks_and_slider_configuration_without_replacing_controls() {
	use leptoskit_native::SliderState;
	use leptoskit_native::components::Button;
	use leptoskit_native::components::Checkbox;
	use leptoskit_native::components::Slider;

	let parent = container();
	let revision = ArcRwSignal::new(0);
	let app_revision = revision.clone();
	let received = Arc::new(Mutex::new(Vec::new()));
	let app_received = Arc::clone(&received);
	let mount = mount_to(parent.clone(), move || {
		move || {
			let current = app_revision.get();
			let press = Arc::clone(&app_received);
			let toggle = Arc::clone(&app_received);
			let value = Arc::clone(&app_received);
			let submit = Arc::clone(&app_received);
			let state = if current == 0 {
				SliderState::new(0.0, 10.0, 4.0, Some(2.0))
			} else {
				SliderState::new(10.0, 20.0, 10.0, Some(5.0))
			}
			.unwrap();
			render(
				View().children([
					Button("Callback")
						.enabled(current > 0)
						.on_press(move || press.lock().unwrap().push(("press", current, 0.0))),
					Checkbox(false)
						.on_toggle(move |_| toggle.lock().unwrap().push(("toggle", current, 0.0))),
					Slider(state).on_value_change(move |proposal| {
						value.lock().unwrap().push(("value", current, proposal));
					}),
					Element::text_input().on(NativeEventSlot::Submit, move |_| {
						submit.lock().unwrap().push(("submit", current, 0.0));
						EventOutcome::Handled
					}),
				]),
			)
		}
	});
	tick().await;
	let button: web_sys::HtmlElement = find(&parent, "button");
	let checkbox: web_sys::HtmlInputElement = find(&parent, "input[type=checkbox]");
	let slider: web_sys::HtmlInputElement = find(&parent, "input[type=range]");
	assert!(button.has_attribute("disabled"));
	slider.focus().unwrap();
	revision.set(1);
	tick().await;
	assert_eq!(find::<web_sys::HtmlElement>(&parent, "button"), button);
	assert_eq!(
		find::<web_sys::HtmlInputElement>(&parent, "input[type=checkbox]"),
		checkbox
	);
	assert_eq!(
		find::<web_sys::HtmlInputElement>(&parent, "input[type=range]"),
		slider
	);
	assert!(!button.has_attribute("disabled"));
	assert_eq!(document().active_element(), Some(slider.clone().into()));
	button.click();
	checkbox.click();
	slider.set_value("17");
	let options = web_sys::EventInit::new();
	options.set_bubbles(true);
	slider
		.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &options).unwrap())
		.unwrap();
	assert_eq!(
		*received.lock().unwrap(),
		[("press", 1, 0.0), ("toggle", 1, 0.0), ("value", 1, 15.0)]
	);
	let input: web_sys::HtmlInputElement = find(&parent, "input[type=text]");
	let key = web_sys::KeyboardEventInit::new();
	key.set_key("Enter");
	key.set_bubbles(true);
	input
		.dispatch_event(
			&web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &key).unwrap(),
		)
		.unwrap();
	assert_eq!(received.lock().unwrap().last(), Some(&("submit", 1, 0.0)));
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn rich_pressables_keep_native_button_semantics_and_focus_during_updates() {
	let parent = container();
	let count = ArcRwSignal::new(0);
	let enabled = ArcRwSignal::new(true);
	let outer = Arc::new(AtomicUsize::new(0));
	let app_count = count.clone();
	let app_enabled = enabled.clone();
	let outer_presses = Arc::clone(&outer);
	let mount = mount_to(parent.clone(), move || {
		move || {
			let increment = app_count.clone();
			let outer_presses = Arc::clone(&outer_presses);
			render(
				View()
					.on_press(move || {
						outer_presses.fetch_add(1, Ordering::Relaxed);
					})
					.child(
						Pressable("Open profile")
							.class("w-60 h-20 p-4 rounded-xl")
							.enabled(app_enabled.get())
							.child(View().child(Text(format!("Activated {}", app_count.get()))))
							.on_press(move || increment.update(|value| *value += 1)),
					),
			)
		}
	});
	tick().await;
	let button: web_sys::HtmlElement = find(&parent, "button");
	assert_eq!(button.get_attribute("type").as_deref(), Some("button"));
	assert_eq!(
		button.get_attribute("aria-label").as_deref(),
		Some("Open profile")
	);
	assert_eq!(button.tab_index(), 0);
	button.focus().expect("focus rich button");
	let decoration: web_sys::HtmlElement = find(&button, "span");
	decoration.click();
	tick().await;
	assert_eq!(count.get_untracked(), 1);
	assert_eq!(outer.load(Ordering::Relaxed), 0);
	assert!(button.text_content().unwrap().contains("Activated 1"));
	assert_eq!(find::<web_sys::HtmlElement>(&parent, "button"), button);
	assert_eq!(document().active_element(), Some(button.clone().into()));
	count.set(4);
	tick().await;
	assert!(button.text_content().unwrap().contains("Activated 4"));
	assert_eq!(document().active_element(), Some(button.clone().into()));
	enabled.set(false);
	tick().await;
	assert!(button.has_attribute("disabled"));
	decoration.click();
	button.click();
	tick().await;
	assert_eq!(count.get_untracked(), 4);
	enabled.set(true);
	tick().await;
	decoration.click();
	tick().await;
	assert_eq!(count.get_untracked(), 5);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
fn hydrates_existing_server_controls_and_attaches_handlers() {
	let parent = container();
	parent.set_inner_html(include_str!("fixtures/web_hydration.html"));
	let original: web_sys::HtmlElement = find(&parent, "button");
	let presses = Arc::new(AtomicUsize::new(0));
	let count = Arc::clone(&presses);
	let mount = hydrate_from(parent.clone(), move || {
		render(
			Element::view()
				.class("p-4")
				.child(Element::text("Hydrated"))
				.child(Element::button("Activate").on_press(move || {
					count.fetch_add(1, Ordering::SeqCst);
				})),
		)
	});
	let hydrated: web_sys::HtmlElement = find(&parent, "button");

	assert_eq!(
		original, hydrated,
		"hydration must preserve server DOM nodes"
	);
	hydrated.click();
	assert_eq!(presses.load(Ordering::SeqCst), 1);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn reactive_controls_preserve_focus_and_deliver_native_events() {
	let parent = container();
	let value = ArcRwSignal::new("First".to_owned());
	let count = ArcRwSignal::new(0);
	let submissions = Arc::new(AtomicUsize::new(0));
	let app_value = value.clone();
	let app_count = count.clone();
	let app_submissions = Arc::clone(&submissions);
	let mount = mount_to(parent.clone(), move || {
		move || {
			let change = app_value.clone();
			let increment = app_count.clone();
			let submitted = Arc::clone(&app_submissions);
			let input = Element::text_input()
				.with_props(|props| {
					props.value = Some(app_value.get());
					props.accessibility_label = Some("Name".to_owned());
				})
				.on(NativeEventSlot::Change, move |event| {
					if let NativeEvent::Change { value } = event {
						change.set(value.clone());
					}

					EventOutcome::Handled
				})
				.on(NativeEventSlot::Submit, move |_| {
					submitted.fetch_add(1, Ordering::SeqCst);
					EventOutcome::Handled
				});

			render(
				Element::view()
					.class("gap-4 p-4")
					.child(input)
					.child(Element::text(format!("Count: {}", app_count.get())))
					.child(
						Element::button("Increment")
							.on_press(move || increment.update(|n| *n += 1)),
					),
			)
		}
	});
	tick().await;
	let input: web_sys::HtmlInputElement = find(&parent, "input");
	assert_eq!(input.value(), "First");
	input.focus().expect("focus input");
	input.set_value("Edited");
	input.set_selection_range(2, 2).expect("set selection");
	let options = web_sys::EventInit::new();
	options.set_bubbles(true);
	input
		.dispatch_event(
			&web_sys::Event::new_with_event_init_dict("input", &options).expect("input event"),
		)
		.expect("dispatch input");
	tick().await;

	assert_eq!(value.get_untracked(), "Edited");
	let current: web_sys::HtmlInputElement = find(&parent, "input");
	assert_eq!(input, current, "Leptos must reuse the input DOM node");
	assert_eq!(input.selection_start().expect("selection"), Some(2));
	assert_eq!(document().active_element(), Some(input.clone().into()));

	let button: web_sys::HtmlElement = find(&parent, "button");
	button.click();
	tick().await;
	assert_eq!(count.get_untracked(), 1);
	assert!(parent.text_content().expect("text").contains("Count: 1"));

	let options = web_sys::KeyboardEventInit::new();
	options.set_key("Enter");
	options.set_bubbles(true);
	input
		.dispatch_event(
			&web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &options)
				.expect("enter event"),
		)
		.expect("dispatch enter");
	assert_eq!(submissions.load(Ordering::SeqCst), 1);

	value.set("Externally updated".to_owned());
	tick().await;
	assert_eq!(input.value(), "Externally updated");
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn handled_events_stop_bubbling_and_disabled_buttons_do_not_press() {
	let parent = container();
	let outer = Arc::new(AtomicUsize::new(0));
	let inner = Arc::new(AtomicUsize::new(0));
	let parent_count = Arc::clone(&outer);
	let child_count = Arc::clone(&inner);
	let blocked_count = Arc::clone(&inner);
	let mount = mount_to(parent.clone(), move || {
		render(
			Element::view()
				.on_press(move || {
					parent_count.fetch_add(1, Ordering::SeqCst);
				})
				.child(Element::button("Handled").on_press(move || {
					child_count.fetch_add(1, Ordering::SeqCst);
				}))
				.child(Element::button("Disabled").with_props(|props| props.enabled = Some(false)))
				.child(
					Element::view()
						.with_props(|props| {
							props.enabled = Some(false);
							props.accessibility_label = Some("Disabled parent".into());
						})
						.child(
							Element::button("Nested")
								.with_props(|props| {
									props.accessibility_label = Some("Nested".into());
								})
								.on_press(move || {
									blocked_count.fetch_add(1, Ordering::SeqCst);
								}),
						),
				),
		)
	});
	let handled: web_sys::HtmlElement = find(&parent, "button");
	let disabled: web_sys::HtmlElement = find(&parent, "button[disabled]");
	handled.click();
	disabled.click();
	let nested: web_sys::HtmlElement = find(&parent, "[aria-label=Nested]");
	let disabled_parent: web_sys::HtmlElement = find(&parent, "[aria-label=\"Disabled parent\"]");
	nested.click();
	disabled_parent.click();
	tick().await;

	assert_eq!(inner.load(Ordering::SeqCst), 1);
	assert_eq!(outer.load(Ordering::SeqCst), 0);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn responsive_styles_update_and_independent_trees_remain_isolated() {
	let frame = document()
		.create_element("iframe")
		.expect("iframe")
		.dyn_into::<web_sys::HtmlIFrameElement>()
		.expect("iframe type");
	frame.set_width("600");
	frame.set_height("400");
	document()
		.body()
		.expect("body")
		.append_child(&frame)
		.expect("attach iframe");
	let parent = frame
		.content_document()
		.expect("frame document")
		.body()
		.expect("frame body");
	let window = frame.content_window().expect("frame window");
	let mount = mount_to(parent.clone(), || {
		(
			render(
				Element::view()
					.class("p-4 md:p-8 md:flex-row")
					.with_props(|props| props.accessibility_label = Some("First".into())),
			),
			render(
				Element::view()
					.class("p-4")
					.with_props(|props| props.accessibility_label = Some("Second".into())),
			),
		)
	});
	tick().await;
	// query_selector already returns an Element. A second instanceof cast uses
	// the parent JavaScript realm and fails for Firefox's adopted iframe nodes.
	let first = parent
		.query_selector("[aria-label=First]")
		.expect("selector")
		.expect("first view");
	let second = parent
		.query_selector("[aria-label=Second]")
		.expect("selector")
		.expect("second view");
	let computed = || {
		window
			.get_computed_style(&first)
			.expect("computed style")
			.expect("style")
	};
	assert_eq!(
		computed()
			.get_property_value("padding-top")
			.expect("padding"),
		"16px"
	);
	assert_eq!(
		computed()
			.get_property_value("flex-direction")
			.expect("direction"),
		"column"
	);

	frame.set_width("900");
	tick().await;
	assert_eq!(
		computed()
			.get_property_value("padding-top")
			.expect("padding"),
		"32px"
	);
	assert_eq!(
		computed()
			.get_property_value("flex-direction")
			.expect("direction"),
		"row"
	);
	assert_eq!(
		window
			.get_computed_style(&second)
			.expect("computed style")
			.expect("style")
			.get_property_value("padding-top")
			.expect("padding"),
		"16px"
	);

	drop(mount);
	frame.remove();
}

#[wasm_bindgen_test]
fn inherits_typography_and_preserves_border_and_scroll_semantics() {
	let parent = container();
	let scrolls = Arc::new(AtomicUsize::new(0));
	let ancestor_scrolls = Arc::clone(&scrolls);
	let mount = mount_to(parent.clone(), move || {
		render(
			Element::view()
				.class("text-end")
				.on(NativeEventSlot::Scroll, move |_| {
					ancestor_scrolls.fetch_add(1, Ordering::SeqCst);
					EventOutcome::Handled
				})
				.child(Element::button("Inherited"))
				.child(
					Element::scroll_view()
						.class("rounded-lg")
						.with_style(|style| {
							style.width = Dimension::Points(80.0);
							style.height = Dimension::Points(80.0);
						})
						.child(
							Element::view()
								.with_style(|style| style.height = Dimension::Points(200.0)),
						),
				)
				.child(
					Element::view()
						.border(2.0, Color::rgba(255, 0, 0, 255))
						.class("border-8 border-[#0000ff]")
						.with_props(|props| props.accessibility_label = Some("Painted".into())),
				)
				.child(
					Element::view()
						.with_style(|style| style.border.top = 2.0)
						.with_props(|props| props.accessibility_label = Some("Layout only".into())),
				),
		)
	});
	let window = web_sys::window().expect("window");
	let button: web_sys::Element = find(&parent, "button");
	let style = window
		.get_computed_style(&button)
		.expect("computed style")
		.expect("style");
	assert_eq!(
		style.get_property_value("text-align").expect("alignment"),
		"end"
	);

	let scroll: web_sys::Element = find(&parent, ".rounded-lg");
	let style = window
		.get_computed_style(&scroll)
		.expect("computed style")
		.expect("style");
	assert_eq!(
		style.get_property_value("overflow-y").expect("overflow"),
		"auto"
	);
	assert_ne!(
		style
			.get_property_value("clip-path")
			.expect("rounded clipping"),
		"none"
	);
	scroll.set_scroll_top(20);
	assert_eq!(scroll.scroll_top(), 20);
	scroll
		.dispatch_event(&web_sys::Event::new("scroll").expect("scroll event"))
		.expect("dispatch scroll");
	assert_eq!(
		scrolls.load(Ordering::SeqCst),
		1,
		"nonbubbling DOM scroll must reach the native ancestor"
	);

	let painted: web_sys::Element = find(&parent, "[aria-label=Painted]");
	let style = window
		.get_computed_style(&painted)
		.expect("computed style")
		.expect("style");
	assert_eq!(
		style
			.get_property_value("border-top-width")
			.expect("border width"),
		"8px"
	);
	assert_eq!(
		style
			.get_property_value("border-top-color")
			.expect("border color"),
		"rgb(0, 0, 255)"
	);
	assert_eq!(
		style
			.get_property_value("box-shadow")
			.expect("legacy border override"),
		"none"
	);

	let layout: web_sys::Element = find(&parent, "[aria-label=\"Layout only\"]");
	let style = window
		.get_computed_style(&layout)
		.expect("computed style")
		.expect("style");
	assert_eq!(
		style
			.get_property_value("border-top-width")
			.expect("layout border width"),
		"2px"
	);
	assert_eq!(
		style
			.get_property_value("border-top-color")
			.expect("layout border color"),
		"rgba(0, 0, 0, 0)"
	);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
fn native_and_browser_share_box_flex_and_grid_geometry() {
	let mut fixtures = vec![
		Element::view()
			.class("flex-row w-[400px] h-[160px] p-4 gap-4 border-4")
			.children((0..2).map(|_| Element::view().class("flex-1 h-10"))),
		Element::view()
			.class("box-content w-[200px] h-[100px] p-4 border-4")
			.child(Element::view().class("w-20 h-10")),
		Element::view()
			.class("grid grid-cols-3 grid-rows-2 w-[400px] h-[200px] p-4 gap-4")
			.children((0..6).map(|_| Element::view())),
	];
	for wrap in [FlexWrap::Wrap, FlexWrap::WrapReverse] {
		for direction in [Direction::Ltr, Direction::Rtl] {
			fixtures.push(
				Element::view()
					.class("w-[100px] h-[100px] gap-[10px] content-start items-start")
					.with_style(|style| {
						style.direction = direction;
						style.flex_wrap = wrap;
					})
					.children((0..3).map(|_| Element::view().class("w-5 h-[60px] shrink-0"))),
			);
		}
	}
	fixtures.push(
		Element::view()
			.class("w-[100px] h-[100px] gap-[10px] content-start items-start")
			.with_style(|style| {
				style.flex_direction = FlexDirection::Row;
				style.flex_wrap = FlexWrap::WrapReverse;
			})
			.children((0..3).map(|_| Element::view().class("w-[60px] h-5 shrink-0"))),
	);
	fixtures.push(
		Element::view()
			.class("w-[100px] h-[100px]")
			.with_style(|style| style.direction = Direction::Rtl)
			.child(Element::view().class("max-w-10 h-5")),
	);

	for (index, element) in fixtures.into_iter().enumerate() {
		let parent = container();
		let mut native = NativeSurface::new(HeadlessBackend::new());
		native.set_viewport(400.0, 300.0);
		native.set_root(&element);
		let mount = mount_to(parent.clone(), move || render(element));
		let browser: web_sys::Element = find(&parent, "div[data-lk-style]");

		assert_geometry(
			&native,
			native.root().expect("native root"),
			&browser,
			index,
		);
		drop(mount);
		parent.remove();
	}
}

#[wasm_bindgen_test]
fn button_labels_are_vertically_centered() {
	let parent = container();
	let mount = mount_to(parent.clone(), || {
		render(Element::button("Centered").class("w-32 h-16"))
	});
	let button: web_sys::Element = find(&parent, "button");
	let range = document().create_range().expect("text range");
	range
		.select_node_contents(&button)
		.expect("button contents");
	let control = button.get_bounding_client_rect();
	let label = range.get_bounding_client_rect();

	close(control.height(), 64.0, 0, "button height");
	assert!(
		((control.top() + control.height() / 2.0) - (label.top() + label.height() / 2.0)).abs()
			< 2.0,
		"button label must be centered within its explicit height"
	);
	drop(mount);
	parent.remove();
}

fn assert_geometry(
	native: &NativeSurface<HeadlessBackend>,
	id: NodeId,
	browser: &web_sys::Element,
	fixture: usize,
) {
	let frame = native.frame(id).expect("native frame");
	let rect = browser.get_bounding_client_rect();
	close(rect.width(), frame.width, fixture, "width");
	close(rect.height(), frame.height, fixture, "height");
	let children = browser.children();

	assert_eq!(children.length() as usize, native.children(id).len());

	for (index, child) in native.children(id).iter().enumerate() {
		let browser_child = children.item(index as u32).expect("browser child");
		let child_rect = browser_child.get_bounding_client_rect();
		let frame = native.frame(*child).expect("child frame");
		close(child_rect.left() - rect.left(), frame.x, fixture, "x");
		close(child_rect.top() - rect.top(), frame.y, fixture, "y");
		assert_geometry(native, *child, &browser_child, fixture);
	}
}

fn close(browser: f64, native: f32, fixture: usize, property: &str) {
	assert!(
		(browser - f64::from(native)).abs() < 0.1,
		"fixture {fixture} {property}: browser={browser}, native={native}"
	);
}

fn document() -> web_sys::Document {
	web_sys::window()
		.expect("window")
		.document()
		.expect("document")
}

fn container() -> web_sys::HtmlElement {
	let parent = document()
		.create_element("div")
		.expect("container")
		.dyn_into::<web_sys::HtmlElement>()
		.expect("HTML container");
	document()
		.body()
		.expect("body")
		.append_child(&parent)
		.expect("attach container");
	parent
}

fn find<T: JsCast>(parent: &web_sys::HtmlElement, selector: &str) -> T {
	parent
		.query_selector(selector)
		.expect("selector")
		.expect("matching control")
		.dyn_into::<T>()
		.unwrap_or_else(|_| panic!("wrong DOM type for {selector}"))
}

#[wasm_bindgen_test]
async fn controlled_toggles_propose_once_bubble_typed_events_and_restore_rejection() {
	use leptoskit_native::components::Checkbox;
	use leptoskit_native::components::Switch;
	let parent = container();
	let proposals = Arc::new(AtomicUsize::new(0));
	let bubbled = Arc::new(AtomicUsize::new(0));
	let wrong_events = Arc::new(AtomicUsize::new(0));
	let child_count = Arc::clone(&proposals);
	let parent_count = Arc::clone(&bubbled);
	let change_count = Arc::clone(&wrong_events);
	let press_count = Arc::clone(&wrong_events);
	let checked = ArcRwSignal::new(false);
	let app_checked = checked.clone();
	let mount = mount_to(parent.clone(), move || {
		move || {
			let child_count = Arc::clone(&child_count);
			let parent_count = Arc::clone(&parent_count);
			let change_count = Arc::clone(&change_count);
			let press_count = Arc::clone(&press_count);
			render(
				Element::view()
					.on(NativeEventSlot::Toggle, move |_| {
						parent_count.fetch_add(1, Ordering::SeqCst);
						EventOutcome::Unhandled
					})
					.on_change(move |_| {
						change_count.fetch_add(1, Ordering::SeqCst);
					})
					.on_press(move || {
						press_count.fetch_add(1, Ordering::SeqCst);
					})
					.child(
						Checkbox(app_checked.get())
							.accessibility_label("Rejected")
							.on(NativeEventSlot::Toggle, move |_| {
								child_count.fetch_add(1, Ordering::SeqCst);
								EventOutcome::Unhandled
							}),
					)
					.child(Switch(true).accessibility_label("Switch")),
			)
		}
	});
	tick().await;
	let input: web_sys::HtmlInputElement = find(&parent, "[aria-label=Rejected]");
	input.focus().expect("focus checkbox");
	input.click();
	tick().await;
	assert!(
		!input.checked(),
		"rejected proposal must reset without a signal update"
	);
	assert_eq!(proposals.load(Ordering::SeqCst), 1);
	assert_eq!(bubbled.load(Ordering::SeqCst), 1);
	assert_eq!(wrong_events.load(Ordering::SeqCst), 0);
	checked.set(true);
	tick().await;
	let current: web_sys::HtmlInputElement = find(&parent, "[aria-label=Rejected]");
	assert_eq!(input, current);
	assert!(input.checked());
	assert_eq!(document().active_element(), Some(input.clone().into()));
	let switch: web_sys::HtmlInputElement = find(&parent, "[role=switch]");
	assert!(switch.checked());
	assert_eq!(
		web_sys::window()
			.expect("window")
			.get_computed_style(&input)
			.expect("style")
			.expect("computed")
			.get_property_value("appearance")
			.expect("appearance"),
		"auto"
	);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn sliders_and_passwords_keep_owner_state_and_dom_identity() {
	use leptoskit_native::components::*;
	let parent = container();
	let value = ArcRwSignal::new(4.0);
	let app_value = value.clone();
	let mount = mount_to(parent.clone(), move || {
		move || {
			let proposed = app_value.clone();
			render(
				View()
					.child(
						Slider(
							SliderState::new(0.0, 10.0, app_value.get(), Some(2.0))
								.expect("slider"),
						)
						.on_value_change(move |value| {
							if value <= 6.0 {
								proposed.set(value);
							}
						}),
					)
					.child(
						PasswordInput()
							.value("committed")
							.accessibility_label("Password"),
					),
			)
		}
	});
	tick().await;
	let slider: web_sys::HtmlInputElement = find(&parent, "input[type=range]");
	let password: web_sys::HtmlInputElement = find(&parent, "input[type=password]");
	slider.set_value("6");
	input_event(&slider);
	tick().await;
	assert!((value.get_untracked() - 6.0).abs() < f64::EPSILON);
	assert_eq!(slider.value(), "6");
	slider.set_value("10");
	input_event(&slider);
	tick().await;
	assert_eq!(slider.value(), "6", "rejected range proposal must reset");
	password.focus().expect("focus password");
	password.set_value("rejected");
	input_event(&password);
	tick().await;
	assert_eq!(password.value(), "committed");
	value.set(2.0);
	tick().await;
	assert_eq!(slider.value(), "2");
	assert_eq!(
		slider,
		find::<web_sys::HtmlInputElement>(&parent, "input[type=range]")
	);
	assert_eq!(
		password,
		find::<web_sys::HtmlInputElement>(&parent, "input[type=password]")
	);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn radio_groups_restore_rejection_support_arrows_and_isolate_render_roots() {
	use leptoskit_native::components::*;
	let parent = container();
	let independent = container();
	let selected = ArcRwSignal::new(0);
	let app_selected = selected.clone();
	let mount = mount_to(parent.clone(), move || {
		move || {
			let group = RadioGroup::new("choice").expect("group");
			let first = app_selected.clone();
			let second = app_selected.clone();
			render(
				View()
					.child(
						Radio(group.clone(), app_selected.get() == 0)
							.accessibility_label("First")
							.on_toggle(move |_| first.set(0)),
					)
					.child(
						Radio(group.clone(), false)
							.accessibility_label("Disabled")
							.enabled(false),
					)
					.child(
						Radio(group.clone(), app_selected.get() == 1)
							.accessibility_label("Second")
							.on_toggle(move |_| second.set(1)),
					)
					.child(Radio(group, false).accessibility_label("Rejected")),
			)
		}
	});
	let other_mount = mount_to(independent.clone(), move || {
		render(Radio(RadioGroup::new("choice").expect("group"), true))
	});
	tick().await;
	let first: web_sys::HtmlInputElement = find(&parent, "[aria-label=First]");
	let second: web_sys::HtmlInputElement = find(&parent, "[aria-label=Second]");
	let rejected: web_sys::HtmlInputElement = find(&parent, "[aria-label=Rejected]");
	let other: web_sys::HtmlInputElement = find(&independent, "input");
	assert_ne!(first.name(), other.name());
	assert!(first.checked() && other.checked());
	rejected.click();
	tick().await;
	assert!(first.checked());
	assert!(!rejected.checked());
	first.focus().expect("focus first");
	let options = web_sys::KeyboardEventInit::new();
	options.set_key("ArrowRight");
	options.set_bubbles(true);
	options.set_cancelable(true);
	first
		.dispatch_event(
			&web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &options)
				.expect("arrow event"),
		)
		.expect("dispatch arrow");
	tick().await;
	assert_eq!(selected.get_untracked(), 1);
	assert!(!first.checked() && second.checked() && other.checked());
	assert_eq!(document().active_element(), Some(second.clone().into()));
	let old_name = first.name();
	selected.set(0);
	tick().await;
	assert_eq!(
		first.name(),
		old_name,
		"root scope must survive a reactive rebuild"
	);
	assert!(first.checked() && !second.checked() && other.checked());
	drop(other_mount);
	drop(mount);
	independent.remove();
	parent.remove();
}

#[wasm_bindgen_test]
async fn disabled_hidden_controls_reject_synthetic_input_and_spinners_stop() {
	use leptoskit_native::components::*;
	let parent = container();
	let proposals = Arc::new(AtomicUsize::new(0));
	let counts = Arc::clone(&proposals);
	let running = ArcRwSignal::new(true);
	let app_running = running.clone();
	let mount =
		mount_to(parent.clone(), move || {
			move || {
				let counts = Arc::clone(&counts);
				let stopped_count = Arc::clone(&counts);
				render(
					View()
						.child(
							View().class("hidden").child(
								Checkbox(false).accessibility_label("Hidden").on_toggle(
									move |_| {
										counts.fetch_add(1, Ordering::SeqCst);
									},
								),
							),
						)
						.child(
							View()
								.enabled(false)
								.child(Switch(false).accessibility_label("Disabled")),
						)
						.child(View().enabled(false).child(
							ActivityIndicator(app_running.get()).accessibility_label("Loading"),
						))
						.child(
							ActivityIndicator(false)
								.accessibility_label("Stopped")
								.on_press(move || {
									stopped_count.fetch_add(1, Ordering::SeqCst);
								}),
						)
						.child(LinearProgressIndicator(Progress::Determinate(
							ProgressValue::new(0.25).expect("progress"),
						))),
				)
			}
		});
	tick().await;
	let hidden: web_sys::HtmlInputElement = find(&parent, "[aria-label=Hidden]");
	hidden.set_checked(true);
	input_event(&hidden);
	tick().await;
	assert!(!hidden.checked());
	assert_eq!(proposals.load(Ordering::SeqCst), 0);
	let disabled: web_sys::HtmlInputElement = find(&parent, "[aria-label=Disabled]");
	assert!(disabled.disabled());
	let stopped: web_sys::HtmlElement = find(&parent, "[aria-label=Stopped]");
	stopped.click();
	assert_eq!(proposals.load(Ordering::SeqCst), 0);
	let spinner: web_sys::HtmlElement = find(&parent, "[aria-label=Loading]");
	let animation = || {
		web_sys::window()
			.expect("window")
			.get_computed_style_with_pseudo_elt(&spinner, "::before")
			.expect("style")
			.expect("computed")
			.get_property_value("animation-name")
			.expect("animation")
	};
	assert_eq!(animation(), "lk-spinner");
	assert_eq!(
		spinner.get_attribute("aria-busy").as_deref(),
		Some("true"),
		"disabled ancestors must not stop loading"
	);
	let before = spinner.get_bounding_client_rect();
	let progress: web_sys::HtmlElement = find(&parent, "progress");
	let progress_y = progress.get_bounding_client_rect().y();
	running.set(false);
	tick().await;
	assert_eq!(animation(), "none");
	assert_eq!(spinner.get_attribute("aria-busy").as_deref(), Some("false"));
	assert_eq!(
		spinner.get_attribute("aria-hidden").as_deref(),
		Some("true")
	);
	assert_eq!(
		web_sys::window()
			.expect("window")
			.get_computed_style(&spinner)
			.expect("style")
			.expect("computed")
			.get_property_value("visibility")
			.expect("visibility"),
		"hidden"
	);
	let after = spinner.get_bounding_client_rect();
	assert!((before.width() - after.width()).abs() < 0.01);
	assert!((before.height() - after.height()).abs() < 0.01);
	assert!(
		(progress_y - progress.get_bounding_client_rect().y()).abs() < 0.01,
		"stopping a spinner retains its layout box"
	);
	assert_eq!(progress.get_attribute("value").as_deref(), Some("0.25"));
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
fn new_controls_hydrate_without_replacing_server_nodes() {
	use leptoskit_native::components::*;
	let parent = container();
	parent.set_inner_html(include_str!("fixtures/web_controls_hydration.html"));
	let radio: web_sys::HtmlInputElement = find(&parent, "input[type=radio]");
	let slider: web_sys::HtmlInputElement = find(&parent, "input[type=range]");
	assert!(radio.name().is_empty());
	let mount = hydrate_from(parent.clone(), move || {
		render(
			View()
				.child(Switch(true).accessibility_label("Switch"))
				.child(Checkbox(false).accessibility_label("Checkbox"))
				.child(
					Radio(RadioGroup::new("choice").expect("group"), true)
						.accessibility_label("Radio"),
				)
				.child(
					Slider(SliderState::new(0.0, 10.0, 4.0, Some(2.0)).expect("slider"))
						.accessibility_label("Slider"),
				)
				.child(
					LinearProgressIndicator(Progress::Determinate(
						ProgressValue::new(0.4).expect("progress"),
					))
					.accessibility_label("Progress"),
				)
				.child(ActivityIndicator(true).accessibility_label("Loading"))
				.child(
					PasswordInput()
						.value("secret")
						.accessibility_label("Password"),
				),
		)
	});
	assert_eq!(
		radio,
		find::<web_sys::HtmlInputElement>(&parent, "input[type=radio]")
	);
	assert_eq!(
		slider,
		find::<web_sys::HtmlInputElement>(&parent, "input[type=range]")
	);
	assert!(!radio.name().is_empty());
	assert!(radio.checked());
	drop(mount);
	parent.remove();
}

fn input_event(input: &web_sys::HtmlInputElement) {
	let options = web_sys::EventInit::new();
	options.set_bubbles(true);
	input
		.dispatch_event(
			&web_sys::Event::new_with_event_init_dict("input", &options).expect("input event"),
		)
		.expect("dispatch input");
}

#[wasm_bindgen_test]
fn visible_utilities_override_base_hidden_while_disabled_and_hidden_ancestors_block() {
	use leptoskit_native::components::*;
	let parent = container();
	let proposals = Arc::new(AtomicUsize::new(0));
	let app_proposals = Arc::clone(&proposals);
	let mount = mount_to(parent.clone(), move || {
		let checkbox = |label: &str| {
			let proposals = Arc::clone(&app_proposals);
			Checkbox(false)
				.accessibility_label(label)
				.on_toggle(move |_| {
					proposals.fetch_add(1, Ordering::SeqCst);
				})
		};
		render(
			View()
				.child(
					checkbox("Visible")
						.with_props(|props| props.hidden = Some(true))
						.class("flex"),
				)
				.child(
					View()
						.with_props(|props| props.hidden = Some(true))
						.class("flex")
						.child(checkbox("Visible descendant")),
				)
				.child(
					View()
						.with_props(|props| props.hidden = Some(true))
						.child(checkbox("Hidden ancestor").class("flex")),
				)
				.child(
					View()
						.class("hidden")
						.child(checkbox("Hidden utility ancestor").class("flex")),
				)
				.child(
					View()
						.enabled(false)
						.with_props(|props| props.hidden = Some(true))
						.class("flex")
						.child(checkbox("Disabled ancestor").enabled(true)),
				),
		)
	});

	for label in ["Visible", "Visible descendant"] {
		let input: web_sys::HtmlInputElement = find(&parent, &format!("[aria-label=\"{label}\"]"));
		assert!(!input.disabled());
		assert!(input.get_attribute("aria-disabled").is_none());
		assert!(input.get_attribute("data-lk-hidden").is_none());
		input.click();
		assert!(!input.checked(), "uncommitted proposals are restored");
	}

	assert_eq!(proposals.load(Ordering::SeqCst), 2);

	for label in [
		"Hidden ancestor",
		"Hidden utility ancestor",
		"Disabled ancestor",
	] {
		let input: web_sys::HtmlInputElement = find(&parent, &format!("[aria-label=\"{label}\"]"));
		input.set_checked(true);
		input_event(&input);
		assert!(!input.checked());
	}

	let disabled: web_sys::HtmlInputElement = find(&parent, "[aria-label=\"Disabled ancestor\"]");
	assert!(disabled.disabled());
	assert_eq!(proposals.load(Ordering::SeqCst), 2);
	drop(mount);
	parent.remove();
}

#[wasm_bindgen_test]
async fn responsive_visibility_uses_css_without_disabling_the_control() {
	use leptoskit_native::StyleContext;
	use leptoskit_native::components::Checkbox;
	use leptoskit_yoga::Size;
	let frame: web_sys::HtmlIFrameElement = document()
		.create_element("iframe")
		.expect("iframe")
		.unchecked_into();
	frame.set_width("600");
	frame.set_height("400");
	document()
		.body()
		.expect("body")
		.append_child(&frame)
		.expect("attach iframe");
	let parent = frame
		.content_document()
		.expect("frame document")
		.body()
		.expect("frame body");
	let window = frame.content_window().expect("frame window");
	let element = Checkbox(false)
		.with_props(|props| props.hidden = Some(true))
		.class("md:flex");
	let narrow = StyleContext {
		viewport: Size::new(600.0, 400.0),
		..StyleContext::default()
	};
	let wide = StyleContext {
		viewport: Size::new(900.0, 400.0),
		..StyleContext::default()
	};
	assert_eq!(element.resolved(&narrow).props.hidden, Some(true));
	assert_eq!(element.resolved(&wide).props.hidden, Some(false));
	let mount = mount_to(parent.clone(), move || render(element));
	tick().await;
	// The selector guarantees the input type across JavaScript iframe realms.
	let input: web_sys::HtmlInputElement = parent
		.query_selector("input[type=checkbox]")
		.expect("selector")
		.expect("checkbox")
		.unchecked_into();
	let display = || {
		window
			.get_computed_style(&input)
			.expect("style")
			.expect("computed")
			.get_property_value("display")
			.expect("display")
	};
	assert_eq!(display(), "none");
	assert!(!input.disabled());

	frame.set_width("900");
	tick().await;
	assert_eq!(display(), "flex");
	assert!(!input.disabled());
	assert!(input.get_attribute("aria-disabled").is_none());

	frame.set_width("600");
	tick().await;
	assert_eq!(display(), "none");
	assert!(!input.disabled());
	drop(mount);
	frame.remove();
}

#[wasm_bindgen_test]
fn radio_group_unicode_names_survive_hydration_without_merging_selection() {
	use leptoskit_native::components::*;

	for invalid in ["a\0b", "a\rb", "a\nb", "a\r\nb", "a\tb"] {
		assert!(RadioGroup::new(invalid).is_err());
	}

	let parent = container();
	parent.set_inner_html(include_str!("fixtures/web_radio_unicode_hydration.html"));
	let first: web_sys::HtmlInputElement = find(&parent, "[aria-label=Replacement]");
	let second: web_sys::HtmlInputElement = find(&parent, "[aria-label=Unicode]");
	assert!(first.checked() && second.checked());
	let mount = hydrate_from(parent.clone(), move || {
		render(View().children([
			Radio(RadioGroup::new("a�b").unwrap(), true).accessibility_label("Replacement"),
			Radio(RadioGroup::new("aéb").unwrap(), true).accessibility_label("Unicode"),
		]))
	});
	assert_eq!(
		first,
		find::<web_sys::HtmlInputElement>(&parent, "[aria-label=Replacement]")
	);
	assert_eq!(
		second,
		find::<web_sys::HtmlInputElement>(&parent, "[aria-label=Unicode]")
	);
	assert_ne!(first.name(), second.name());
	assert!(first.checked() && second.checked());
	drop(mount);
	parent.remove();
}
