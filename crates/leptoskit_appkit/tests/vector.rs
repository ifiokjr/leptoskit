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
	use leptoskit_native::FillRule;
	use leptoskit_native::NativeBackend;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::PathData;
	use leptoskit_native::Stroke;
	use leptoskit_native::StrokeScaling;
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
	use objc2_quartz_core::CALayer;
	use objc2_quartz_core::CAShapeLayer;

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
			(
				"evenodd_paths_punch_holes_that_nonzero_paths_fill",
				evenodd_paths_punch_holes_that_nonzero_paths_fill,
			),
			(
				"dashed_strokes_leave_gaps_from_their_offset",
				dashed_strokes_leave_gaps_from_their_offset,
			),
			(
				"filled_drawings_cover_their_frame_and_crop_in_clipping_parents",
				filled_drawings_cover_their_frame_and_crop_in_clipping_parents,
			),
			(
				"drawing_scaled_strokes_thicken_with_the_frame",
				drawing_scaled_strokes_thicken_with_the_frame,
			),
			(
				"unchanged_drawings_keep_their_layers_and_paths",
				unchanged_drawings_keep_their_layers_and_paths,
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

	fn evenodd_paths_punch_holes_that_nonzero_paths_fill() {
		// Both subpaths wind the same way, so only the fill rule decides
		// whether the inner square is a hole.
		let ring = || data("M2 2 H22 V22 H2 Z M8 8 H16 V16 H8 Z");
		let tree = View()
			.class("flex-row w-[120px] h-[60px]")
			.background(WHITE)
			.child(
				Vector(view_box(24.0, 24.0))
					.class("w-[60px] h-[60px]")
					.path(
						VectorPath::new(ring())
							.fill(RED)
							.fill_rule(FillRule::Evenodd),
					),
			)
			.child(
				Vector(view_box(24.0, 24.0))
					.class("w-[60px] h-[60px]")
					.path(VectorPath::new(ring()).fill(BLUE)),
			);
		let bitmap = bitmap(&mounted(&tree));

		// Each 60-point frame draws the 24-unit box at 2.5 points per unit:
		// the ring spans 5..55 and the inner square 20..40.
		assert_color(&bitmap, 10.0, 30.0, RED, "the evenodd ring");
		assert_color(&bitmap, 30.0, 30.0, WHITE, "the evenodd hole");
		assert_color(&bitmap, 70.0, 30.0, BLUE, "the nonzero ring");
		assert_color(
			&bitmap,
			90.0,
			30.0,
			BLUE,
			"the nonzero square's filled centre",
		);
	}

	fn dashed_strokes_leave_gaps_from_their_offset() {
		let dashed = |offset: f32| {
			Vector(view_box(100.0, 10.0))
				.class("w-[100px] h-[10px]")
				.path(
					VectorPath::new(data("M0 5 H100")).stroke(
						Stroke::new(BLUE, 4.0)
							.expect("a positive width")
							.dashes(&[10.0, 10.0], offset)
							.expect("valid dashes"),
					),
				)
		};
		let tree = View()
			.class("w-[100px] h-[20px]")
			.background(WHITE)
			.child(dashed(0.0))
			.child(dashed(10.0));
		let bitmap = bitmap(&mounted(&tree));

		// Butt-capped 10-point dashes alternate with 10-point gaps along
		// y = 5; the second line starts one dash into the pattern.
		for x in [5.0, 25.0, 85.0] {
			assert_color(&bitmap, x, 5.0, BLUE, "a dash");
			assert_color(&bitmap, x, 15.0, WHITE, "an offset gap");
		}

		for x in [15.0, 35.0, 95.0] {
			assert_color(&bitmap, x, 5.0, WHITE, "a gap");
			assert_color(&bitmap, x, 15.0, BLUE, "an offset dash");
		}
	}

	fn filled_drawings_cover_their_frame_and_crop_in_clipping_parents() {
		// A square drawing, blue over red, filled into a 60×30 frame scales
		// by 3 to 60×60 and centres vertically, overflowing 15 points above
		// and below the frame.
		let tree = View()
			.class("w-[120px] h-[90px] p-[30px]")
			.background(WHITE)
			.child(
				View().class("w-[60px] h-[30px] overflow-hidden").child(
					Vector(view_box(20.0, 20.0))
						.class("absolute inset-0")
						.scaling(VectorScaling::Fill)
						.path(VectorPath::new(data("M0 0 H20 V10 H0 Z")).fill(BLUE))
						.path(VectorPath::new(data("M0 10 H20 V20 H0 Z")).fill(RED)),
				),
			);
		let bitmap = bitmap(&mounted(&tree));

		assert_color(&bitmap, 32.0, 37.0, BLUE, "the frame's top-left corner");
		assert_color(&bitmap, 88.0, 37.0, BLUE, "the frame's top-right corner");
		assert_color(&bitmap, 32.0, 53.0, RED, "the frame's bottom-left corner");
		assert_color(&bitmap, 88.0, 53.0, RED, "the frame's bottom-right corner");
		assert_color(&bitmap, 60.0, 22.0, WHITE, "the cropped overflow above");
		assert_color(&bitmap, 60.0, 68.0, WHITE, "the cropped overflow below");
	}

	fn drawing_scaled_strokes_thicken_with_the_frame() {
		let tree = |side: &str| {
			View().class("w-[80px] h-[80px]").background(WHITE).child(
				Vector(view_box(10.0, 10.0))
					.class(format!("w-[{side}px] h-[{side}px]"))
					.path(
						VectorPath::new(data("M0 5 H10")).stroke(
							Stroke::new(BLUE, 1.0)
								.expect("a positive width")
								.scaling(StrokeScaling::Drawing),
						),
					),
			)
		};
		let mut surface = mounted(&tree("40"));
		let bitmap = bitmap(&surface);
		// At 4 points per unit the 1-unit stroke is 4 points wide, y 18..22.
		assert_color(&bitmap, 20.0, 20.0, BLUE, "the scaled stroke");
		assert_color(&bitmap, 20.0, 16.0, WHITE, "above the 4-point stroke");
		assert_color(&bitmap, 20.0, 24.0, WHITE, "below the 4-point stroke");

		// Only the frame grows; the drawing is unchanged, and the resize
		// alone redraws it 8 points wide, y 36..44.
		surface.set_root(&tree("80"));
		let bitmap = self::bitmap(&surface);
		assert_color(&bitmap, 40.0, 37.0, BLUE, "the thickened stroke's top");
		assert_color(&bitmap, 40.0, 43.0, BLUE, "the thickened stroke's bottom");
		assert_color(&bitmap, 40.0, 33.0, WHITE, "above the 8-point stroke");
		assert_color(&bitmap, 40.0, 47.0, WHITE, "below the 8-point stroke");
	}

	/// The vector's shape layers and the address of each one's path.
	fn shapes(surface: &NativeSurface<AppKitBackend>) -> Vec<(Retained<CAShapeLayer>, usize)> {
		let node = vector_nodes(surface)[0];
		let view = surface.backend().view(node).expect("a live view");
		let layer = view.layer().expect("vector views are layer-backed");
		// SAFETY: reading a live layer's sublayers on the main thread.
		unsafe { layer.sublayers() }
			.map(|layers| layers.to_vec())
			.unwrap_or_default()
			.into_iter()
			.map(|layer: Retained<CALayer>| {
				let shape = layer
					.downcast::<CAShapeLayer>()
					.expect("vector views hold only shape layers");
				let path = shape
					.path()
					.map_or(0, |path| std::ptr::from_ref(&*path) as usize);
				(shape, path)
			})
			.collect()
	}

	fn unchanged_drawings_keep_their_layers_and_paths() {
		let tree = |background: Color, fill: Color, width: &str| {
			View()
				.class("w-[100px] h-[40px]")
				.background(background)
				.child(
					Vector(view_box(24.0, 24.0))
						.class(format!("w-[{width}px] h-[40px]"))
						.path(VectorPath::new(data("M4 4 H20 V20 H4 Z")).fill(fill))
						.path(
							VectorPath::new(data("M4 12 H20"))
								.stroke(Stroke::new(BLUE, 2.0).expect("a positive width")),
						),
				)
		};
		let mut surface = mounted(&tree(WHITE, RED, "40"));
		let drawn = shapes(&surface);
		assert_eq!(drawn.len(), 2, "one shape layer per path");

		// Props carrying an equal drawing, such as after a restyle, leave the
		// layers alone: a width set behind the backend's back survives, where
		// a redraw would restore the stroke's 2 points.
		drawn[1].0.setLineWidth(7.0);
		let node = vector_nodes(&surface)[0];
		let props = surface.props(node).clone();
		surface.backend_mut().update_props(node, &props);
		let kept = shapes(&surface);
		assert!(
			drawn.iter().zip(&kept).all(|((before, _), (after, _))| {
				Retained::as_ptr(before) == Retained::as_ptr(after)
			}),
			"an unchanged drawing keeps its layers"
		);
		assert!(
			(kept[1].0.lineWidth() - 7.0).abs() < f64::EPSILON,
			"an unchanged drawing is not redrawn"
		);

		// A new fill reuses the same layers rather than replacing them, and
		// redraws every one of them.
		surface.set_root(&tree(BLUE, BLUE, "40"));
		let refilled = shapes(&surface);
		assert!(
			drawn
				.iter()
				.zip(&refilled)
				.all(|((before, _), (after, _))| {
					Retained::as_ptr(before) == Retained::as_ptr(after)
				}),
			"a changed drawing reuses its layers"
		);
		assert!((refilled[1].0.lineWidth() - 2.0).abs() < f64::EPSILON);

		// A resize rebuilds the paths on the same layers.
		surface.set_root(&tree(BLUE, BLUE, "80"));
		let resized = shapes(&surface);
		assert!(
			refilled
				.iter()
				.zip(&resized)
				.all(|((before, before_path), (after, after_path))| {
					Retained::as_ptr(before) == Retained::as_ptr(after) && before_path != after_path
				}),
			"a resize redraws the same layers"
		);
	}
}
