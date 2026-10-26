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
		PathData::parse("M0 0 A5 5 0 0 1 10 10"),
		Err(VectorError::UnsupportedCommand {
			command: 'A',
			offset: 5
		})
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
		.as_deref()
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
