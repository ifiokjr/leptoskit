#![cfg(all(feature = "web", target_arch = "wasm32"))]

use std::sync::Arc;
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
	let first: web_sys::Element = find(&parent, "[aria-label=First]");
	let second: web_sys::Element = find(&parent, "[aria-label=Second]");
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
