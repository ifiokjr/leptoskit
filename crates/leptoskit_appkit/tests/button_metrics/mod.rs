//! Actual native cells and arbitrary-thread measurement share button metrics.

mod context_probe;

use leptoskit_appkit::AppKitBackend;
use leptoskit_appkit::AppKitTextMeasurer;
use leptoskit_appkit::MainThreadMarker;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use leptoskit_yoga::Yoga;
use objc2_app_kit::NSAttributedStringNSExtendedStringDrawing;
use objc2_app_kit::NSAttributedStringNSStringDrawing;
use objc2_app_kit::NSButton;
use objc2_app_kit::NSScreen;
use objc2_app_kit::NSStringDrawingOptions;
use objc2_core_foundation::CGSize;
use objc2_core_graphics::CGDisplayCopyDisplayMode;
use objc2_core_graphics::CGDisplayMode;
use objc2_core_graphics::CGMainDisplayID;

const JOURNEY: &str = "Subscribe, transfer and unsubscribe";
const FONT_SIZES: [f32; 5] = [10.0, 14.0, 18.0, 28.0, 42.0];
const NATIVE_HEIGHT: f32 = 24.0;
const NARROW_WIDTH: f32 = 64.0;
const VIEWPORT: f32 = 712.0;
const BLACK: Color = Color::rgba(0, 0, 0, 255);

pub fn native_controls() {
	for font_size in FONT_SIZES {
		for font_weight in [FontWeight::Regular, FontWeight::Bold] {
			compare_titles(TextStyle::new(font_size, font_weight));
		}
	}

	if std::env::var_os("LEPTOSKIT_BUTTON_METRICS_DIAGNOSTICS").is_some() {
		context_probe::observe();
	}
}

fn compare_titles(style: TextStyle) {
	let marker = MainThreadMarker::new().expect("native cells require the main thread");
	let mut backend = AppKitBackend::new(marker);
	let mut ids = Yoga::new();

	for text in [
		None,
		Some(""),
		Some("Go"),
		Some(JOURNEY),
		Some("gypq ÅÉç façade naïve"),
	] {
		let id = ids.create_node();
		let props = NativeProps {
			text: text.map(str::to_owned),
			font_size: Some(style.font_size),
			font_weight: Some(style.font_weight),
			..NativeProps::default()
		};
		backend.create(id, NativeKind::Button, &props);

		let view = backend.view(id).expect("created button");
		let button = view.downcast_ref::<NSButton>().expect("native NSButton");
		let natural = button.cell().expect("native control cell").cellSize();
		let measured = AppKitTextMeasurer::new().measure_button(
			text.unwrap_or_default(),
			style,
			unbounded(),
			true,
		);
		let context = measurement_context(button, style);

		if std::env::var_os("LEPTOSKIT_BUTTON_METRICS_DIAGNOSTICS").is_some() {
			eprintln!("{text:?}: {context}");
		}

		assert_eq!(
			f64::from(measured.width).to_bits(),
			natural.width.to_bits(),
			"{text:?}; {context}"
		);
		assert_eq!(
			f64::from(measured.height).to_bits(),
			natural.height.to_bits()
		);
	}
}

fn measurement_context(button: &NSButton, style: TextStyle) -> String {
	const TITLE_BOUNDS: f64 = 1.0e7;
	let marker = MainThreadMarker::new().expect("native context requires the main thread");
	let title = button.attributedTitle();
	let fragment = title.boundingRectWithSize_options_context(
		CGSize::new(TITLE_BOUNDS, TITLE_BOUNDS),
		NSStringDrawingOptions::UsesLineFragmentOrigin,
		None,
	);
	let single_line = title.size();
	let scale = NSScreen::mainScreen(marker).map(|screen| screen.backingScaleFactor());
	let quartz = CGDisplayCopyDisplayMode(CGMainDisplayID()).map(|mode| {
		(
			CGDisplayMode::width(Some(&mode)),
			CGDisplayMode::pixel_width(Some(&mode)),
		)
	});

	format!(
		"style={style:?}, fragment={:?}, single={single_line:?}, scale={scale:?}, mode={quartz:?}",
		fragment.size,
	)
}

pub fn off_main_thread() {
	let worker = std::thread::spawn(|| {
		assert!(MainThreadMarker::new().is_none());
		let style = TextStyle::default();
		let measurer = AppKitTextMeasurer::new();
		let natural = measurer.measure_button(JOURNEY, style, unbounded(), true);
		assert_eq!(natural.height.to_bits(), NATIVE_HEIGHT.to_bits());
		let constrained = measurer.measure_button(
			JOURNEY,
			style,
			Size::new(
				AvailableSpace::Definite(NARROW_WIDTH),
				AvailableSpace::MaxContent,
			),
			true,
		);
		assert_eq!(constrained.width.to_bits(), NARROW_WIDTH.to_bits());
		assert_eq!(constrained.height.to_bits(), NATIVE_HEIGHT.to_bits());
		let custom = measurer.measure_button(JOURNEY, style, unbounded(), false);
		assert!(custom.width < natural.width);
	});
	worker
		.join()
		.expect("pure measurement needs no main-thread UI object");
}

fn unbounded() -> Size<AvailableSpace> {
	Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent)
}

pub fn updates() {
	let backend =
		HeadlessBackend::new().with_text_measurer(std::sync::Arc::new(AppKitTextMeasurer::new()));
	let mut surface = NativeSurface::new(backend);
	surface.set_viewport(VIEWPORT, VIEWPORT);
	let tree = |button| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(button)
	};
	surface.set_root(&tree(Element::button(JOURNEY)));
	let root = surface.root().unwrap();
	let button = surface.children(root)[0];

	let natural = surface.frame(button).unwrap();
	surface.set_root(&tree(Element::button("Go")));
	assert_eq!(surface.children(root), &[button]);
	assert!(surface.frame(button).unwrap().width < natural.width);
	surface.set_root(&tree(Element::button(JOURNEY).background(BLACK)));
	assert_eq!(surface.children(root), &[button]);
	assert!(surface.frame(button).unwrap().width < natural.width);
	surface.set_root(&tree(Element::button(JOURNEY)));
	assert_eq!(surface.children(root), &[button]);
	assert_eq!(surface.frame(button), Some(natural));
}
