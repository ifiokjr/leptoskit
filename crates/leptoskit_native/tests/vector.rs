//! Path data parsing, canonical serialization, and the frame mapping every
//! backend draws through.
// The mappings here are exact in binary floating point.
#![allow(clippy::float_cmp)]

use leptoskit_native::Color;
use leptoskit_native::Drawing;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::LineCap;
use leptoskit_native::LineJoin;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::PathData;
use leptoskit_native::Stroke;
use leptoskit_native::VectorError;
use leptoskit_native::VectorPath;
use leptoskit_native::VectorScaling;
use leptoskit_native::ViewBox;
use leptoskit_native::components::Vector;
use leptoskit_native::components::View;
use leptoskit_native::controls::ControlTreeError;
use leptoskit_native::vector::PathCommand;
use leptoskit_native::vector::Point;

const INK: Color = Color::rgba(17, 17, 17, 255);

fn canonical(data: &str) -> String {
	PathData::parse(data)
		.unwrap_or_else(|error| panic!("`{data}` parses: {error}"))
		.to_string()
}

#[test]
fn absolute_commands_round_trip_canonically() {
	assert_eq!(
		canonical("M0 0 L10 10 Q 5 0 10 0 C1,2 3,4 5,6 Z"),
		"M 0 0 L 10 10 Q 5 0 10 0 C 1 2 3 4 5 6 Z"
	);
}

#[test]
fn relative_and_shorthand_commands_resolve_to_absolute() {
	assert_eq!(
		canonical("m10 10 l5 0 h5 v5 H0 V0 z"),
		"M 10 10 L 15 10 L 20 10 L 20 15 L 0 15 L 0 0 Z"
	);
	// `S` mirrors the previous cubic's second control point, `T` the
	// previous quadratic's control point.
	assert_eq!(
		canonical("M0 0 C0 10 10 10 10 0 S20 -10 20 0"),
		"M 0 0 C 0 10 10 10 10 0 C 10 -10 20 -10 20 0"
	);
	assert_eq!(
		canonical("M0 0 Q5 10 10 0 T20 0"),
		"M 0 0 Q 5 10 10 0 Q 15 -10 20 0"
	);
	// Without a preceding curve of the same kind, the control point is the
	// current point.
	assert_eq!(
		canonical("M0 0 L5 5 S10 10 15 5"),
		"M 0 0 L 5 5 C 5 5 10 10 15 5"
	);
	assert_eq!(canonical("M0 0 L5 5 T10 0"), "M 0 0 L 5 5 Q 5 5 10 0");
	assert_eq!(
		canonical("M1 1 c1 1 2 2 3 3 s1 1 2 2 q1 1 2 2 t2 2"),
		"M 1 1 C 2 2 3 3 4 4 C 5 5 5 5 6 6 Q 7 7 8 8 Q 9 9 10 10"
	);
}

#[test]
fn repeated_coordinates_repeat_the_command() {
	// After a move-to, extra pairs are line-tos, in the move's case.
	assert_eq!(canonical("M0 0 10 0 10 10"), "M 0 0 L 10 0 L 10 10");
	assert_eq!(canonical("m1 1 2 0 0 2"), "M 1 1 L 3 1 L 3 3");
	assert_eq!(canonical("M0 0 L1 1 2 2"), "M 0 0 L 1 1 L 2 2");
}

#[test]
fn compact_svg_numbers_parse() {
	assert_eq!(canonical("M.5.5L-1-1"), "M 0.5 0.5 L -1 -1");
	assert_eq!(canonical("M1e1,2E-1 L+3,4."), "M 10 0.2 L 3 4");
	assert_eq!(canonical("  M 0 0\n\tL 1 1  "), "M 0 0 L 1 1");
	assert_eq!(canonical(""), "");
}

#[test]
fn drawing_after_close_restarts_at_the_subpath_start() {
	assert_eq!(
		canonical("M5 5 L10 5 L10 10 Z L0 0"),
		"M 5 5 L 10 5 L 10 10 Z M 5 5 L 0 0"
	);
	assert_eq!(canonical("M5 5 L10 5 z l1 1"), "M 5 5 L 10 5 Z M 5 5 L 6 6");
	assert_eq!(
		PathData::from_commands([
			PathCommand::MoveTo(Point::new(1.0, 1.0)),
			PathCommand::LineTo(Point::new(2.0, 1.0)),
			PathCommand::Close,
			PathCommand::LineTo(Point::new(3.0, 3.0)),
		])
		.map(|data| data.to_string()),
		Ok("M 1 1 L 2 1 Z M 1 1 L 3 3".to_owned())
	);
}

#[test]
fn malformed_path_data_reports_where() {
	assert_eq!(PathData::parse("L0 0"), Err(VectorError::MissingMoveTo));
	assert_eq!(
		PathData::parse("0 0"),
		Err(VectorError::PathData { offset: 0 })
	);
	assert_eq!(
		PathData::parse("M0 0 L1"),
		Err(VectorError::PathData { offset: 7 })
	);
	assert_eq!(
		PathData::parse("M0 0 X1 1"),
		Err(VectorError::PathData { offset: 5 })
	);
	assert_eq!(
		PathData::parse("M0 0 Z 1 1"),
		Err(VectorError::PathData { offset: 7 })
	);
	assert_eq!(
		PathData::parse("M0 0 A5 5 0 2 1 10 10"),
		Err(VectorError::PathData { offset: 12 })
	);
	assert_eq!(
		PathData::parse("M0 0 L1e39 0"),
		Err(VectorError::NonFiniteCoordinate)
	);
	assert_eq!(
		PathData::from_commands([PathCommand::MoveTo(Point::new(f32::NAN, 0.0))]),
		Err(VectorError::NonFiniteCoordinate)
	);

	assert_eq!(
		PathData::from_commands([PathCommand::Close]),
		Err(VectorError::MissingMoveTo)
	);
}

#[test]
fn values_validate() {
	assert!(ViewBox::new(0.0, 0.0, 0.0, 1.0).is_err());
	assert!(ViewBox::new(0.0, 0.0, 1.0, -1.0).is_err());
	assert!(ViewBox::new(f32::NAN, 0.0, 1.0, 1.0).is_err());
	assert_eq!(
		ViewBox::new(-1.0, 2.0, 3.0, 4.0).map(|view_box| view_box.to_string()),
		Ok("-1 2 3 4".to_owned())
	);
	assert_eq!(Stroke::new(INK, 0.0), Err(VectorError::StrokeWidth));
	assert_eq!(
		Stroke::new(INK, f32::INFINITY),
		Err(VectorError::StrokeWidth)
	);

	let stroke = Stroke::new(INK, 2.0).expect("a positive width").round();
	assert_eq!(stroke.line_cap(), LineCap::Round);
	assert_eq!(stroke.line_join(), LineJoin::Round);
}

#[test]
fn fit_scales_uniformly_and_centres() {
	let drawing = Drawing::new(ViewBox::new(10.0, 10.0, 20.0, 10.0).expect("a valid view box"));
	let transform = drawing.transform(100.0, 100.0);

	assert_eq!(transform.scale_x, 5.0);
	assert_eq!(transform.scale_y, 5.0);
	// The 100×50 drawing sits 25 units down the 100×100 frame.
	assert_eq!(
		transform.apply(Point::new(10.0, 10.0)),
		Point::new(0.0, 25.0)
	);
	assert_eq!(
		transform.apply(Point::new(30.0, 20.0)),
		Point::new(100.0, 75.0)
	);
}

#[test]
fn stretch_fills_both_axes() {
	let drawing = Drawing::new(ViewBox::UNIT).scaling(VectorScaling::Stretch);
	let transform = drawing.transform(200.0, 40.0);

	assert_eq!(transform.apply(Point::new(0.0, 0.0)), Point::new(0.0, 0.0));
	assert_eq!(
		transform.apply(Point::new(1.0, 1.0)),
		Point::new(200.0, 40.0)
	);

	let data = PathData::parse("M0 0.5 C0.25 0 0.75 1 1 0.5").expect("valid data");
	assert_eq!(
		data.transformed(transform).collect::<Vec<_>>(),
		vec![
			PathCommand::MoveTo(Point::new(0.0, 20.0)),
			PathCommand::CubicTo {
				control1: Point::new(50.0, 0.0),
				control2: Point::new(150.0, 40.0),
				to: Point::new(200.0, 20.0),
			},
		]
	);
}

#[test]
fn vector_elements_carry_their_drawing() {
	let element = Vector(ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("a valid view box"))
		.scaling(VectorScaling::Stretch)
		.path(
			VectorPath::new(PathData::parse("M0 0 L24 24").expect("valid data"))
				.stroke(Stroke::new(INK, 2.0).expect("a positive width")),
		)
		.path(VectorPath::new(PathData::parse("M0 0 H24 V24 Z").expect("valid data")).fill(INK));
	let drawing = element
		.props
		.drawing
		.as_ref()
		.expect("a vector has a drawing");

	assert_eq!(element.kind, NativeKind::Vector);
	assert_eq!(drawing.vector_scaling(), VectorScaling::Stretch);
	assert_eq!(drawing.paths().len(), 2);
	assert_eq!(drawing.paths()[1].fill_color(), Some(INK));
	assert_eq!(
		Vector(ViewBox::UNIT).child(View()).validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::Vector
		})
	);
}

#[test]
#[should_panic(expected = "apply only to Vector elements")]
fn paths_on_other_kinds_panic() {
	let _ = Element::view().path(VectorPath::new(PathData::default()));
}

#[test]
fn surfaces_lay_out_drawings_from_classes() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(200.0, 200.0);
	surface.set_root(
		&View().class("w-20 h-10").child(
			Vector(ViewBox::UNIT)
				.class("absolute inset-0")
				.path(VectorPath::new(PathData::parse("M0 0 L1 1").expect("valid data")).fill(INK)),
		),
	);

	let created = surface
		.backend()
		.log()
		.iter()
		.find_map(|mutation| {
			match mutation {
				Mutation::Create {
					id,
					kind: NativeKind::Vector,
					props,
				} => Some((*id, props.drawing.clone())),
				_ => None,
			}
		})
		.expect("the vector is created");
	assert_eq!(
		created.1.map(|drawing| drawing.paths().len()),
		Some(1),
		"the backend receives the drawing with its props"
	);
	assert!(
		surface.backend().log().iter().any(|mutation| {
			matches!(
				mutation,
				Mutation::UpdateLayout { id, width, height, .. }
					if *id == created.0 && *width == 80.0 && *height == 40.0
			)
		}),
		"the drawing covers its parent"
	);
}

fn commands(data: &str) -> Vec<PathCommand> {
	PathData::parse(data)
		.unwrap_or_else(|error| panic!("`{data}` parses: {error}"))
		.commands()
		.to_vec()
}

fn close(left: Point, right: Point) -> bool {
	(left.x - right.x).abs() < 1e-3 && (left.y - right.y).abs() < 1e-3
}

#[test]
fn arcs_become_quarter_turn_cubics_through_the_spec_centre() {
	// A half circle of radius 5 from (0, 0) to (10, 0). A positive sweep
	// turns clockwise in a y-down space, over the top through (5, -5).
	let half = commands("M0 0 A5 5 0 0 1 10 0");
	assert_eq!(half.len(), 3, "two quarter turns after the move: {half:?}");
	let PathCommand::CubicTo { to: middle, .. } = half[1] else {
		panic!("arcs become cubics: {half:?}");
	};
	assert!(close(middle, Point::new(5.0, -5.0)), "{middle:?}");
	assert!(
		matches!(half[2], PathCommand::CubicTo { to, .. } if to == Point::new(10.0, 0.0)),
		"the last segment ends exactly on the end point"
	);

	// The other sweep goes the other way round.
	let PathCommand::CubicTo { to: below, .. } = commands("M0 0 A5 5 0 0 0 10 0")[1] else {
		panic!("arcs become cubics");
	};
	assert!(close(below, Point::new(5.0, 5.0)), "{below:?}");

	// Radii too small to reach scale up to a half turn; relative arcs and
	// packed flags parse.
	assert_eq!(commands("M0 0 a1 1 0 0110 0").len(), 3);
	assert_eq!(
		commands("M0 0 A1 1 0 0 1 10 0"),
		commands("M0 0 A5 5 0 0 1 10 0")
	);

	// A zero radius is a straight line, and a zero-length arc draws nothing.
	assert_eq!(
		commands("M0 0 A0 5 0 0 1 10 0"),
		vec![
			PathCommand::MoveTo(Point::new(0.0, 0.0)),
			PathCommand::LineTo(Point::new(10.0, 0.0)),
		]
	);
	assert_eq!(commands("M3 3 A5 5 0 0 1 3 3").len(), 1);

	// A large arc takes the long way: three quarter turns or more.
	assert!(commands("M0 0 A10 10 0 1 1 10 0").len() >= 4);
}

#[test]
fn shapes_build_closed_paths() {
	let rect = PathData::rect(1.0, 2.0, 3.0, 4.0).expect("a valid rect");
	assert_eq!(rect.to_string(), "M 1 2 L 4 2 L 4 6 L 1 6 Z");

	let circle = PathData::circle(10.0, 10.0, 5.0).expect("a valid circle");
	// A move, four quarter arcs, and a close.
	assert_eq!(circle.commands().len(), 6);
	assert!(
		circle.commands().iter().all(|command| {
			match command {
				PathCommand::MoveTo(point) | PathCommand::CubicTo { to: point, .. } => {
					((point.x - 10.0).hypot(point.y - 10.0) - 5.0).abs() < 1e-4
				}
				_ => true,
			}
		}),
		"every on-curve point lies on the circle"
	);

	// Corner radii clamp to half the side.
	let rounded = PathData::rounded_rect(0.0, 0.0, 10.0, 4.0, 3.0, 3.0).expect("valid");
	assert_eq!(
		rounded.commands()[0],
		PathCommand::MoveTo(Point::new(3.0, 0.0))
	);
	assert!(
		rounded
			.commands()
			.contains(&PathCommand::LineTo(Point::new(10.0, 2.0)))
	);

	assert_eq!(
		PathData::polygon([
			Point::new(0.0, 0.0),
			Point::new(4.0, 0.0),
			Point::new(2.0, 3.0)
		])
		.map(|data| data.to_string()),
		Ok("M 0 0 L 4 0 L 2 3 Z".to_owned())
	);
	assert_eq!(
		PathData::circle(0.0, 0.0, -1.0),
		Err(VectorError::ShapeSize)
	);
	assert_eq!(
		PathData::rect(f32::NAN, 0.0, 1.0, 1.0),
		Err(VectorError::ShapeSize)
	);
}

#[test]
fn transform_lists_compose_like_svg() {
	use leptoskit_native::Affine;

	let rotated = Affine::parse("rotate(90, 5, 5)").expect("a valid transform");
	assert!(close(
		rotated.apply(Point::new(10.0, 5.0)),
		Point::new(5.0, 10.0)
	));

	// Right to left: scale first, then translate.
	let list = Affine::parse("translate(10 0) scale(2)").expect("a valid list");
	assert_eq!(list.apply(Point::new(1.0, 1.0)), Point::new(12.0, 2.0));
	assert_eq!(list.length_scale(), 2.0);

	let matrix = Affine::parse("matrix(1,0,0,1,3,4)").expect("a valid matrix");
	assert_eq!(matrix, Affine::translate(3.0, 4.0));

	let skewed = Affine::parse("skewX(45)").expect("a valid skew");
	assert!(close(
		skewed.apply(Point::new(0.0, 1.0)),
		Point::new(1.0, 1.0)
	));

	assert_eq!(Affine::parse("spin(3)"), Err(VectorError::Transform));
	assert_eq!(Affine::parse("scale(1 2 3)"), Err(VectorError::Transform));

	let moved = PathData::parse("M0 0 L1 0")
		.expect("valid data")
		.transform(Affine::translate(2.0, 3.0));
	assert_eq!(moved.to_string(), "M 2 3 L 3 3");
}

#[test]
fn dashes_normalize_like_svg() {
	let stroke = Stroke::new(INK, 1.0).expect("a positive width");

	let odd = stroke
		.clone()
		.dashes(&[4.0, 2.0, 1.0], 3.0)
		.expect("valid dashes");
	assert_eq!(odd.dash_lengths(), &[4.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
	assert_eq!(odd.dash_offset(), 3.0);

	let solid = stroke
		.clone()
		.dashes(&[0.0, 0.0], 5.0)
		.expect("valid dashes");
	assert_eq!(solid.dash_lengths(), [] as [f32; 0]);
	assert_eq!(solid.dash_offset(), 0.0, "a solid line has no offset");

	assert_eq!(
		stroke.clone().dashes(&[-1.0], 0.0),
		Err(VectorError::DashArray)
	);
	assert_eq!(
		stroke.clone().miter_limit(0.5),
		Err(VectorError::MiterLimit)
	);
	assert_eq!(
		stroke.miter_limit(10.0).map(|stroke| stroke.miter()),
		Ok(10.0)
	);
}

#[test]
fn fill_and_alignment_place_uniform_scaling() {
	use leptoskit_native::VectorAlign;
	use leptoskit_native::VectorAlignment;

	let wide = ViewBox::new(0.0, 0.0, 20.0, 10.0).expect("a valid view box");

	// Fill covers the 100×100 frame: scale 10, cropping 50 units each side.
	let filled = Drawing::new(wide).scaling(VectorScaling::Fill);
	let transform = filled.transform(100.0, 100.0);
	assert_eq!(transform.scale_x, 10.0);
	assert_eq!(
		transform.apply(Point::new(0.0, 0.0)),
		Point::new(-50.0, 0.0)
	);

	// Fit aligned to the end sits at the bottom of the frame.
	let end = Drawing::new(wide).align(VectorAlignment::new(VectorAlign::Start, VectorAlign::End));
	let transform = end.transform(100.0, 100.0);
	assert_eq!(transform.apply(Point::new(0.0, 0.0)), Point::new(0.0, 50.0));
	assert_eq!(
		VectorAlignment::new(VectorAlign::Start, VectorAlign::End).svg_keyword(),
		"xMinYMax"
	);
}

#[test]
fn scaling_strokes_follow_the_drawing() {
	use leptoskit_native::StrokeScaling;

	let drawing = Drawing::new(ViewBox::new(0.0, 0.0, 10.0, 10.0).expect("valid"));
	let transform = drawing.transform(40.0, 40.0);
	let fixed = Stroke::new(INK, 2.0)
		.expect("a positive width")
		.dashes(&[1.0, 1.0], 0.5)
		.expect("valid dashes");
	let scaling = fixed.clone().scaling(StrokeScaling::Drawing);

	assert_eq!(fixed.frame_width(transform), 2.0);
	assert_eq!(scaling.frame_width(transform), 8.0);
	assert_eq!(
		scaling.frame_dashes(transform).collect::<Vec<_>>(),
		vec![4.0, 4.0]
	);
	assert_eq!(scaling.frame_dash_offset(transform), 2.0);

	// Uneven stretching scales by the square root of the area scale.
	let stretched = drawing
		.scaling(VectorScaling::Stretch)
		.transform(40.0, 10.0);
	assert_eq!(scaling.frame_width(stretched), 4.0);
}

#[test]
fn current_color_resolves_to_the_inherited_text_color() {
	use leptoskit_native::Paint;
	use leptoskit_native::StyleContext;

	let accent = Color::rgba(0x4f, 0x46, 0xe5, 0xff);
	let icon = Vector(ViewBox::UNIT).path(
		VectorPath::new(PathData::parse("M0 0 L1 1").expect("valid data"))
			.fill(Paint::CurrentColor)
			.stroke(Stroke::new(Paint::CurrentColor, 1.0).expect("a positive width")),
	);
	let resolved = View()
		.color(accent)
		.child(icon.clone())
		.resolved(&StyleContext::default());
	let path = &resolved.children[0]
		.props
		.drawing
		.as_ref()
		.expect("a drawing")
		.paths()[0];

	assert_eq!(path.fill_paint(), Some(Paint::Color(accent)));
	assert_eq!(
		path.stroke_style().map(Stroke::paint),
		Some(Paint::Color(accent))
	);
	// Unresolved, the keyword stays for the web, and native falls back to
	// CSS's initial black.
	let unresolved = &icon.props.drawing.as_ref().expect("a drawing").paths()[0];
	assert_eq!(unresolved.fill_paint(), Some(Paint::CurrentColor));
	assert_eq!(unresolved.fill_color(), Some(Color::rgba(0, 0, 0, 255)));
}

#[test]
fn shared_paths_and_drawings_clone_by_pointer() {
	let data = PathData::parse("M0 0 L10 10 C1 2 3 4 5 6").expect("valid data");
	let copy = data.clone();
	assert!(std::ptr::eq(data.commands(), copy.commands()));
	assert!(std::ptr::eq(data.canonical(), copy.canonical()));

	let drawing = Drawing::new(ViewBox::UNIT).path(VectorPath::new(data).fill(INK));
	let shared = drawing.clone();
	assert!(std::ptr::eq(drawing.paths(), shared.paths()));
	assert_eq!(drawing, shared);

	// Editing a shared drawing copies it first; the original is untouched.
	let edited = shared.path(VectorPath::new(PathData::default()));
	assert_eq!(drawing.paths().len(), 1);
	assert_eq!(edited.paths().len(), 2);
}

#[test]
fn errors_explain_themselves() {
	let messages = [
		VectorError::ViewBox,
		VectorError::StrokeWidth,
		VectorError::NonFiniteCoordinate,
		VectorError::MissingMoveTo,
		VectorError::PathData { offset: 3 },
		VectorError::MiterLimit,
		VectorError::DashArray,
		VectorError::ShapeSize,
		VectorError::Transform,
	]
	.map(|error| error.to_string());

	assert!(messages[0].contains("view box"));
	assert!(messages[1].contains("stroke width"));
	assert!(messages[2].contains("finite"));
	assert!(messages[3].contains("move-to"));
	assert_eq!(messages[4], "invalid path data at byte 3");
	assert!(messages[5].contains("miter limit"));
	assert!(messages[6].contains("dash"));
	assert!(messages[7].contains("shapes"));

	assert!(messages[8].contains("transform list"));
}

#[test]
fn paths_parse_from_strings_and_map_every_command() {
	let data: PathData = "M0 0 Q1 2 3 4 C1 1 2 2 3 3 Z".parse().expect("valid data");
	assert!(!data.is_empty());
	assert!(PathData::default().is_empty());

	let transform = Drawing::new(ViewBox::new(0.0, 0.0, 10.0, 10.0).expect("valid"))
		.scaling(VectorScaling::Stretch)
		.transform(20.0, 30.0);
	assert_eq!(
		data.transformed(transform).collect::<Vec<_>>(),
		vec![
			PathCommand::MoveTo(Point::new(0.0, 0.0)),
			PathCommand::QuadTo {
				control: Point::new(2.0, 6.0),
				to: Point::new(6.0, 12.0),
			},
			PathCommand::CubicTo {
				control1: Point::new(2.0, 3.0),
				control2: Point::new(4.0, 6.0),
				to: Point::new(6.0, 9.0),
			},
			PathCommand::Close,
		]
	);

	// An exponent marker without digits ends the number before it.
	assert_eq!(
		PathData::parse("M1E"),
		Err(VectorError::PathData { offset: 2 })
	);
}

#[test]
fn view_boxes_expose_their_edges() {
	let view_box = ViewBox::new(-1.0, 2.0, 3.0, 4.0).expect("a valid view box");

	assert_eq!(
		[
			view_box.min_x(),
			view_box.min_y(),
			view_box.width(),
			view_box.height()
		],
		[-1.0, 2.0, 3.0, 4.0]
	);
	assert_eq!(ViewBox::default(), ViewBox::UNIT);
	assert_eq!(NativeKind::Vector.wire_name(), "vector");
}
