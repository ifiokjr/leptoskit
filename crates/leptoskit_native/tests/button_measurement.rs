//! Button-specific metrics retain generic measurers and existing field sizing.

use std::sync::Arc;

use leptoskit_native::ApproxTextMeasurer;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Size;

const VIEWPORT: f32 = 320.0;
const BUTTON_WIDTH: f32 = 80.0;
const BUTTON_HEIGHT: f32 = 24.0;
const LABEL_WIDTH: f32 = 16.0;
const LABEL_HEIGHT: f32 = 16.0;
const FIELD_HEIGHT: f32 = 28.0;
const CONSTRAINED_WIDTH: f32 = 40.0;
const BLACK: Color = Color::rgba(0, 0, 0, 255);

struct ButtonMeasurer;

impl TextMeasurer for ButtonMeasurer {
	fn measure(&self, _: &str, _: TextStyle, _: Size<AvailableSpace>) -> Size<f32> {
		Size::new(LABEL_WIDTH, LABEL_HEIGHT)
	}

	fn measure_button(
		&self,
		_: &str,
		_: TextStyle,
		_: Size<AvailableSpace>,
		native_chrome: bool,
	) -> Size<f32> {
		if native_chrome {
			return Size::new(BUTTON_WIDTH, BUTTON_HEIGHT);
		}

		Size::new(LABEL_WIDTH, LABEL_HEIGHT)
	}
}

#[test]
fn default_button_measurement_preserves_existing_implementors() {
	let measurer = ApproxTextMeasurer::new();
	let available = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);

	for chrome in [false, true] {
		assert_eq!(
			measurer.measure_button("Go", TextStyle::default(), available, chrome),
			measurer.measure("Go", TextStyle::default(), available),
		);
	}
}

#[test]
fn only_buttons_use_button_metrics_and_chrome_updates_remeasure_the_same_node() {
	let backend = HeadlessBackend::new().with_text_measurer(Arc::new(ButtonMeasurer));
	let mut surface = NativeSurface::new(backend);
	surface.set_viewport(VIEWPORT, VIEWPORT);
	let tree = |button| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(button)
			.child(Element::text("Go"))
			.child(Element::text_input().aria_label("Action").value("Go"))
	};
	surface.set_root(&tree(Element::button("Go")));
	let root = surface.root().unwrap();
	let ids = surface.children(root).to_vec();
	assert_size(&surface, ids[0], BUTTON_WIDTH, BUTTON_HEIGHT);
	assert_size(&surface, ids[1], LABEL_WIDTH, LABEL_HEIGHT);
	assert_size(&surface, ids[2], LABEL_WIDTH, FIELD_HEIGHT);

	for button in [
		Element::button("Updated").background(BLACK),
		Element::button("Updated").corner_radius(0.0),
		Element::button("Updated").border(0.0, BLACK),
		Element::button("Updated"),
	] {
		let native = !button.props.is_custom_drawn();
		surface.set_root(&tree(button));
		assert_eq!(surface.children(root), ids.as_slice());
		let expected = if native {
			(BUTTON_WIDTH, BUTTON_HEIGHT)
		} else {
			(LABEL_WIDTH, LABEL_HEIGHT)
		};
		assert_size(&surface, ids[0], expected.0, expected.1);
	}
}

#[test]
fn absent_and_empty_titles_remeasure_the_same_button() {
	let backend = HeadlessBackend::new().with_text_measurer(Arc::new(ButtonMeasurer));
	let mut surface = NativeSurface::new(backend);
	let tree = |button| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(button)
	};
	surface.set_viewport(VIEWPORT, VIEWPORT);
	surface.set_root(&tree(Element::button("Go")));
	let root = surface.root().unwrap();
	let button = surface.children(root)[0];

	for title in [Some(""), None, Some("Go"), None] {
		let mut element = Element::button("");
		element.props.text = title.map(str::to_owned);
		surface.set_root(&tree(element));
		assert_eq!(surface.children(root), &[button]);
		assert_size(&surface, button, BUTTON_WIDTH, BUTTON_HEIGHT);
	}
}

#[test]
fn default_backend_preserves_absent_and_empty_title_layout_and_text_height() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(VIEWPORT, VIEWPORT);
	let tree = |title: Option<&str>| {
		let mut button = Element::button("");
		button.props.text = title.map(str::to_owned);
		Element::view().child(Element::text("Go")).child(button)
	};
	surface.set_root(&tree(None));
	let root = surface.root().unwrap();
	let ids = surface.children(root).to_vec();

	for title in [Some(""), None] {
		surface.set_root(&tree(title));
		assert_eq!(surface.children(root), ids.as_slice());
		// The root fills the viewport's width and stretches its children.
		assert_size(&surface, root, VIEWPORT, LABEL_HEIGHT * 2.0);
		assert_size(&surface, ids[0], VIEWPORT, LABEL_HEIGHT);
		assert_size(&surface, ids[1], VIEWPORT, LABEL_HEIGHT);
		assert_eq!(
			surface.frame(ids[0]).unwrap().y.to_bits(),
			0.0_f32.to_bits()
		);
		assert_eq!(
			surface.frame(ids[1]).unwrap().y.to_bits(),
			LABEL_HEIGHT.to_bits()
		);
	}
}

#[test]
fn explicit_button_width_remains_owned_by_layout() {
	let backend = HeadlessBackend::new().with_text_measurer(Arc::new(ButtonMeasurer));
	let mut surface = NativeSurface::new(backend);
	surface.set_viewport(VIEWPORT, VIEWPORT);
	surface.set_root(
		&Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(Element::button("Go").with_style(|style| {
				style.width = Dimension::Points(CONSTRAINED_WIDTH);
			})),
	);
	let root = surface.root().unwrap();
	let button = surface.children(root)[0];
	assert_size(&surface, button, CONSTRAINED_WIDTH, BUTTON_HEIGHT);
}

fn assert_size(
	surface: &NativeSurface<HeadlessBackend>,
	id: leptoskit_yoga::NodeId,
	width: f32,
	height: f32,
) {
	let frame = surface.frame(id).unwrap();
	assert_eq!(frame.width.to_bits(), width.to_bits());
	assert_eq!(frame.height.to_bits(), height.to_bits());
}
