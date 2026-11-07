//! The display list's plain values: colours convert, transforms compose,
//! and every builder appends the command it promises.

use leptoskit_canvas::Affine;
use leptoskit_canvas::BoxShadow;
use leptoskit_canvas::Color;
use leptoskit_canvas::Command;
use leptoskit_canvas::FillRule;
use leptoskit_canvas::LineCap;
use leptoskit_canvas::LineJoin;
use leptoskit_canvas::Paint;
use leptoskit_canvas::Path;
use leptoskit_canvas::PathCommand;
use leptoskit_canvas::Point;
use leptoskit_canvas::Rect;
use leptoskit_canvas::Scene;
use leptoskit_canvas::Stroke;

/// A value the compiler cannot fold into a constant, so `const fn` bodies
/// really run and count as covered code.
fn runtime(value: &str) -> f32 {
	value.parse().expect("a float")
}

#[test]
fn colors_convert_from_8_bit_channels_and_opacity() {
	let red = Color::rgb8(runtime("255") as u8, 0, 0);
	assert_eq!(red, Color::rgba(1.0, 0.0, 0.0, 1.0));
	let translucent = Color::rgba8(0, 0, 255, runtime("128") as u8);
	assert!((translucent.a - 128.0 / 255.0).abs() < 1.0e-6);
	assert_eq!(translucent.with_opacity(0.5).a, translucent.a * 0.5);
	assert_eq!(translucent.with_opacity(0.5).b, translucent.b);
	assert_eq!(Color::WHITE.premultiplied(), [1.0, 1.0, 1.0, 1.0]);
	assert_eq!(Color::TRANSPARENT.premultiplied(), [0.0; 4]);
	assert_eq!(translucent.premultiplied()[3], translucent.a);
}

#[test]
fn transforms_have_a_default_rotate_and_compose() {
	assert_eq!(Affine::default(), Affine::IDENTITY);
	let quarter_turn = runtime("1.5707964");
	let turned = Affine::rotate(quarter_turn).apply(Point::new(1.0, 0.0));
	assert!(turned.x.abs() < 1.0e-5, "{turned:?}");
	assert!((turned.y - 1.0).abs() < 1.0e-5, "{turned:?}");
	let composed = Affine::scale(2.0, 2.0).then(Affine::translate(runtime("10.0"), 0.0));
	assert_eq!(composed.apply(Point::new(1.0, 1.0)), Point::new(12.0, 2.0));
}

#[test]
fn strokes_build_beyond_the_defaults() {
	let stroke = Stroke::new(runtime("3.0"))
		.cap(LineCap::Square)
		.join(LineJoin::Bevel)
		.miter_limit(runtime("4.0"));
	assert_eq!(stroke.width, 3.0);
	assert_eq!(stroke.cap, LineCap::Square);
	assert_eq!(stroke.join, LineJoin::Bevel);
	assert_eq!(stroke.miter_limit, 4.0);
	assert_eq!(Stroke::default(), Stroke::new(1.0));
}

#[test]
fn paths_and_scenes_build_from_their_builders() {
	let radius = runtime("6.0");
	let rect = Rect::new(0.0, 0.0, 10.0, 5.0);
	let outline = Path::new().rect(rect);
	assert_eq!(outline.commands.len(), 5);
	assert_eq!(outline.commands[4], PathCommand::Close);
	let circle = Path::new().circle(5.0, 5.0, radius);
	assert_eq!(circle.commands.len(), 6);
	assert_eq!(
		circle.commands[0],
		PathCommand::MoveTo(Point::new(11.0, 5.0))
	);
	assert!(matches!(circle.commands[1], PathCommand::CubicTo(_, _, _)));
	let scene = Scene::new()
		.push_clip(rect, radius)
		.push_clip_path(
			Path::new()
				.move_to(0.0, 0.0)
				.quad_to(5.0, 5.0, 10.0, 0.0)
				.close(),
			FillRule::EvenOdd,
		)
		.fill_rounded_rect(rect, radius, Paint::Solid(Color::BLACK))
		.box_shadow(rect, radius, BoxShadow::default())
		.stroke_path(outline, Stroke::new(1.0), Color::BLACK)
		.pop_clip()
		.pop_clip();
	assert_eq!(scene.commands.len(), 7);
	assert_eq!(
		scene.commands[1],
		Command::PushClipPath {
			path: Path::new()
				.move_to(0.0, 0.0)
				.quad_to(5.0, 5.0, 10.0, 0.0)
				.close(),
			rule: FillRule::EvenOdd,
		}
	);
	assert!(matches!(scene.commands[2], Command::FillRect { .. }));
	assert!(matches!(scene.commands[3], Command::BoxShadow { .. }));
	assert!(matches!(scene.commands[4], Command::StrokePath { .. }));
}
