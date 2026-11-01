use leptoskit_appkit::AppKitBackend;
use leptoskit_appkit::AppKitTextMeasurer;
use leptoskit_appkit::MainThreadMarker;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use leptoskit_yoga::Yoga;
use objc2_app_kit::NSBitmapImageRep;
use objc2_app_kit::NSButton;
use objc2_app_kit::NSGraphicsContext;
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_core_graphics::CGContext;
use objc2_foundation::NSString;

pub(super) fn observe() {
	const SCALES: [f64; 2] = [2.0, 1.0];
	const FONT_SIZE: f32 = 10.0;
	let marker = MainThreadMarker::new().expect("native context requires main thread");

	for scale in SCALES {
		let bitmap = bitmap();
		bitmap.setSize(CGSize::new(
			bitmap.pixelsWide() as f64 / scale,
			bitmap.pixelsHigh() as f64 / scale,
		));
		let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap).unwrap();
		let previous = NSGraphicsContext::currentContext();
		let graphics = context.CGContext();
		let initial = CGContext::ctm(Some(&graphics));
		CGContext::scale_ctm(Some(&graphics), scale / initial.a, scale / initial.d);

		let matrix = CGContext::ctm(Some(&graphics));
		assert_eq!(matrix.a.to_bits(), scale.to_bits());
		assert_eq!(matrix.d.to_bits(), scale.to_bits());
		NSGraphicsContext::setCurrentContext(Some(&context));
		let mut backend = AppKitBackend::new(marker);
		let mut yoga = Yoga::new();
		let node = yoga.create_node();
		let props = NativeProps {
			text: Some(super::JOURNEY.to_owned()),
			font_size: Some(FONT_SIZE),
			..NativeProps::default()
		};
		backend.create(node, NativeKind::Button, &props);
		let view = backend.view(node).unwrap();
		let button = view.downcast_ref::<NSButton>().unwrap();
		let size = button.cell().unwrap().cellSize();
		let measured = AppKitTextMeasurer::new().measure_button(
			super::JOURNEY,
			TextStyle::new(FONT_SIZE, leptoskit_native::FontWeight::Regular),
			Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
			true,
		);
		eprintln!(
			"bitmap_scale={scale}, matrix={matrix:?}, native={size:?}, measured={measured:?}"
		);
		NSGraphicsContext::setCurrentContext(previous.as_deref());
		assert_eq!(f64::from(measured.width).to_bits(), size.width.to_bits());
	}
}

fn bitmap() -> objc2::rc::Retained<NSBitmapImageRep> {
	let image = NSImage::imageNamed(&NSString::from_str("NSCaution")).expect("system image");
	let bytes = image.TIFFRepresentation().expect("rasterized system image");
	NSBitmapImageRep::imageRepWithData(&bytes).expect("owned bitmap fixture")
}
