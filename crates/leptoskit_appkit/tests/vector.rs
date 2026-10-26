//! Vector drawings paint filled and stroked paths into real AppKit views.
#![allow(unsafe_code)]

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = macos::trials();
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
mod macos {
	use leptoskit_appkit::AppKitBackend;
	use leptoskit_appkit::MainThreadMarker;
	use leptoskit_native::Color;
	use leptoskit_native::Element;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::PathData;
	use leptoskit_native::Stroke;
	use leptoskit_native::VectorPath;
	use leptoskit_native::VectorScaling;
	use leptoskit_native::ViewBox;
	use leptoskit_native::components::Vector;
	use leptoskit_native::components::View;
	use leptoskit_yoga::NodeId;
	use libtest_mimic::Trial;
	use objc2::rc::Retained;
	use objc2_app_kit::NSAccessibility;
	use objc2_app_kit::NSAccessibilityImageRole;
	use objc2_app_kit::NSBitmapImageRep;
	use objc2_app_kit::NSColorSpace;
	use objc2_foundation::NSData;

	const RED: Color = Color::rgba(220, 20, 20, 255);
	const BLUE: Color = Color::rgba(20, 20, 220, 255);
	const WHITE: Color = Color::rgba(255, 255, 255, 255);
	const SCALE: f64 = 2.0;

	pub fn trials() -> Vec<Trial> {
		[
			(
				"fills_and_strokes_paint_in_their_frames",
				fills_and_strokes_paint_in_their_frames as fn(),
			),
			(
				"stretched_strokes_keep_their_width_through_resizes",
				stretched_strokes_keep_their_width_through_resizes,
			),
			(
				"named_drawings_are_images_and_unnamed_ones_decoration",
				named_drawings_are_images_and_unnamed_ones_decoration,
			),
		]
		.into_iter()
		.map(|(name, test)| {
			Trial::test(name, move || {
				test();
				Ok(())
			})
		})
		.collect()
	}

	fn data(path: &str) -> PathData {
		PathData::parse(path).expect("literal path data is valid")
	}

	fn view_box(width: f32, height: f32) -> ViewBox {
		ViewBox::new(0.0, 0.0, width, height).expect("a literal view box is valid")
	}

	/// A 24-unit square filled red, fit into a wide frame so it is centred,
	/// over a stretched blue centreline.
	fn scene() -> Element {
		View()
			.class("w-[120px] h-[60px]")
			.background(WHITE)
			.child(
				Vector(view_box(24.0, 24.0))
					.class("w-[120px] h-[40px]")
					.path(VectorPath::new(data("M4 4 H20 V20 H4 Z")).fill(RED)),
			)
			.child(
				View().class("w-[120px] h-[20px]").child(
					Vector(ViewBox::UNIT)
						.class("absolute inset-0")
						.scaling(VectorScaling::Stretch)
						.path(
							VectorPath::new(data("M0 0.5 L1 0.5"))
								.stroke(Stroke::new(BLUE, 4.0).expect("a positive width").round()),
						),
				),
			)
	}

	fn mounted(tree: &Element) -> NativeSurface<AppKitBackend> {
		let mtm = MainThreadMarker::new().expect("tests run on the main thread");
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_viewport(400.0, 300.0);
		surface.set_root(tree);
		surface
	}

	fn bitmap(surface: &NativeSurface<AppKitBackend>) -> Retained<NSBitmapImageRep> {
		let png = surface
			.backend()
			.snapshot_png(surface.root().expect("committed"), SCALE)
			.expect("the tree snapshots");

		if let Some(directory) = std::env::var_os("LEPTOSKIT_TEST_ARTIFACTS") {
			std::fs::create_dir_all(&directory).expect("the artifact directory exists");
			std::fs::write(std::path::PathBuf::from(directory).join("vector.png"), &png)
				.expect("the snapshot is written");
		}

		NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(&png)).expect("the snapshot decodes")
	}

	/// The sRGB color at a point, in layout units from the root's top-left.
	fn color_at(bitmap: &NSBitmapImageRep, x: f64, y: f64) -> [u8; 3] {
		let color = bitmap
			.colorAtX_y((x * SCALE) as isize, (y * SCALE) as isize)
			.expect("the point is inside the snapshot");
		let srgb = color
			.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
			.expect("snapshot colors convert to sRGB");
		[
			srgb.redComponent(),
			srgb.greenComponent(),
			srgb.blueComponent(),
		]
		.map(|channel| (channel * 255.0).round() as u8)
	}

	fn assert_color(bitmap: &NSBitmapImageRep, x: f64, y: f64, expected: Color, what: &str) {
		let actual = color_at(bitmap, x, y);
		let expected = [expected.red(), expected.green(), expected.blue()];
		// The snapshot is color-managed, so saturated colors shift a little;
		// the checks only need to tell ink from paper.
		let close = actual
			.iter()
			.zip(expected)
			.all(|(actual, expected)| actual.abs_diff(expected) <= 48);
		assert!(
			close,
			"{what} at ({x}, {y}): expected {expected:?}, got {actual:?}"
		);
	}

	fn vector_nodes(surface: &NativeSurface<AppKitBackend>) -> Vec<NodeId> {
		let mut pending = vec![surface.root().expect("committed")];
		let mut found = Vec::new();

		while let Some(id) = pending.pop() {
			if surface.kind(id) == NativeKind::Vector {
				found.push(id);
			}
			pending.extend_from_slice(surface.children(id));
		}

		found.sort_by_key(|id| id.index());
		found
	}

	fn fills_and_strokes_paint_in_their_frames() {
		let surface = mounted(&scene());
		let bitmap = bitmap(&surface);

		// The 120×40 frame fits the square at 40/24 scale, centred: its
		// 4..20 interior spans x 46.7..73.3 and y 6.7..33.3.
		assert_color(&bitmap, 60.0, 20.0, RED, "the filled square's centre");
		assert_color(&bitmap, 40.0, 20.0, WHITE, "left of the centred square");
		assert_color(&bitmap, 80.0, 20.0, WHITE, "right of the centred square");
		// The flipped geometry puts the square in the top half of its frame,
		// not mirrored below it.
		assert_color(&bitmap, 60.0, 3.0, WHITE, "above the square");

		// The stroke runs along y = 50 across the whole width.
		assert_color(&bitmap, 4.0, 50.0, BLUE, "the stroke's left end");
		assert_color(&bitmap, 116.0, 50.0, BLUE, "the stroke's right end");
	}

	fn stretched_strokes_keep_their_width_through_resizes() {
		let mut surface = mounted(&scene());
		let bitmap = bitmap(&surface);
		// A 4-point stroke: had it scaled with the 20-point-tall stretch, it
		// would cover the whole row.
		assert_color(&bitmap, 60.0, 44.0, WHITE, "above the 4-point stroke");
		assert_color(&bitmap, 60.0, 56.0, WHITE, "below the 4-point stroke");

		let wider = View().class("w-[200px] h-[20px]").background(WHITE).child(
			Vector(ViewBox::UNIT)
				.class("absolute inset-0")
				.scaling(VectorScaling::Stretch)
				.path(
					VectorPath::new(data("M0 0.5 L1 0.5"))
						.stroke(Stroke::new(BLUE, 4.0).expect("a positive width").round()),
				),
		);
		surface.set_root(&wider);
		let bitmap = self::bitmap(&surface);
		assert_color(
			&bitmap,
			190.0,
			10.0,
			BLUE,
			"the redrawn stroke's new right end",
		);
		assert_color(&bitmap, 100.0, 3.0, WHITE, "above the redrawn stroke");
	}

	fn named_drawings_are_images_and_unnamed_ones_decoration() {
		let tree = View()
			.child(
				Vector(view_box(24.0, 24.0))
					.class("w-6 h-6")
					.aria_label("Done")
					.path(
						VectorPath::new(data("M4 13 L9 18 L20 6"))
							.stroke(Stroke::new(BLUE, 2.0).expect("a positive width")),
					),
			)
			.child(
				Vector(view_box(24.0, 24.0))
					.class("w-6 h-6")
					.path(VectorPath::new(data("M0 0 H24")).fill(RED)),
			);
		let surface = mounted(&tree);
		let nodes = vector_nodes(&surface);
		let named = surface.backend().view(nodes[0]).expect("a live view");
		let unnamed = surface.backend().view(nodes[1]).expect("a live view");

		assert!(named.isAccessibilityElement());
		assert_eq!(
			named.accessibilityRole().map(|role| role.to_string()),
			// SAFETY: AppKit exports this immutable role-name static.
			Some(unsafe { NSAccessibilityImageRole }.to_string())
		);
		assert_eq!(
			named.accessibilityLabel().map(|label| label.to_string()),
			Some("Done".to_owned())
		);
		assert!(!unnamed.isAccessibilityElement());
	}
}
