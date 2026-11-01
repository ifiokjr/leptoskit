//! Explicit line boundaries remain visible through measurement and Yoga layout.
#![allow(clippy::float_cmp)]

use leptoskit_native::ApproxTextMeasurer;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Size;

const CHARACTER_WIDTH: f32 = 8.0;
const LINE_HEIGHT: f32 = 16.0;
const FIVE_LETTER_WIDTH: f32 = 40.0;
const WRAP_WIDTH: f32 = 48.0;
const WIDE_BOUND: f32 = 80.0;
const THREE_LINE_HEIGHT: f32 = 48.0;
const FOUR_LINE_HEIGHT: f32 = 64.0;
const DOUBLE_FONT_FOUR_LINE_HEIGHT: f32 = 128.0;

const CONTENT_WIDTH: f32 = 200.0;
const VIEWPORT_WIDTH: f32 = 320.0;
const VIEWPORT_HEIGHT: f32 = 240.0;

#[test]
fn explicit_lines_use_the_widest_line_without_a_width_bound() {
	for width in [AvailableSpace::MaxContent, AvailableSpace::MinContent] {
		assert_eq!(
			measure("hello\nworld!", width),
			Size::new(WRAP_WIDTH, LINE_HEIGHT * 2.0)
		);
	}
}

#[test]
fn consecutive_and_trailing_line_breaks_reserve_empty_lines() {
	for width in [
		AvailableSpace::MaxContent,
		AvailableSpace::Definite(WIDE_BOUND),
	] {
		for (text, expected) in [
			("", Size::new(0.0, LINE_HEIGHT)),
			("\n", Size::new(0.0, LINE_HEIGHT * 2.0)),
			("a\n\nb\n", Size::new(CHARACTER_WIDTH, FOUR_LINE_HEIGHT)),
		] {
			assert_eq!(measure(text, width), expected, "{text:?} at {width:?}");
		}
	}
}

#[test]
fn bounded_width_wraps_words_within_each_explicit_line() {
	assert_eq!(
		measure("hello world\nx y z", AvailableSpace::Definite(WRAP_WIDTH)),
		Size::new(FIVE_LETTER_WIDTH, THREE_LINE_HEIGHT),
	);
	assert_eq!(
		measure(
			"abcdefghijk\nb abcdefghijk",
			AvailableSpace::Definite(WIDE_BOUND)
		),
		Size::new(WIDE_BOUND, THREE_LINE_HEIGHT),
	);
}

#[test]
fn multiline_font_scaling_preserves_line_count_and_empty_lines() {
	let measured = ApproxTextMeasurer::new().measure(
		"hello\n\nworld\n",
		TextStyle::new(TextStyle::DEFAULT_FONT_SIZE * 2.0, FontWeight::Bold),
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
	);
	assert_eq!(
		measured,
		Size::new(WIDE_BOUND, DOUBLE_FONT_FOUR_LINE_HEIGHT)
	);
}

#[test]
fn yoga_allocates_all_lines_and_remeasures_text_without_replacing_widgets() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
	let tree = |text| {
		Element::view().child(
			Element::text(text).with_style(|style| style.width = Dimension::Points(CONTENT_WIDTH)),
		)
	};
	surface.set_root(&tree(
		"Solana client journey\nconfirmed\naccount-notification\nquiet-unsubscribe",
	));
	let root = surface.root().expect("root exists");
	let text = surface.children(root)[0];
	let frame = surface.frame(text).expect("text frame");
	assert_eq!(
		(frame.width, frame.height),
		(CONTENT_WIDTH, FOUR_LINE_HEIGHT)
	);

	surface.set_root(&tree("ready\n"));
	assert_eq!(surface.children(root), &[text]);
	assert_eq!(
		surface.frame(text).expect("updated frame").height,
		LINE_HEIGHT * 2.0
	);
}

fn measure(text: &str, width: AvailableSpace) -> Size<f32> {
	ApproxTextMeasurer::new().measure(
		text,
		TextStyle::default(),
		Size::new(width, AvailableSpace::MaxContent),
	)
}
