//! Scenes from untrusted code come out bounded and balanced.

use leptoskit_canvas::*;

#[test]
fn numbers_become_finite_and_bounded() {
	let nan = f32::NAN;
	let scene = Scene::new()
		.with(Command::FillRect {
			rect: Rect::new(nan, f32::INFINITY, -5.0, 40.0),
			radii: CornerRadii::all(1e9),
			paint: Color::rgba(nan, 2.0, -1.0, 0.5).into(),
			border: Some(Border {
				width: 1e9,
				color: Color::BLACK,
			}),
		})
		.stroke_path(
			Path::new().line_to(nan, 1e30),
			Stroke::new(-1.0).miter_limit(nan),
			Color::BLACK,
		)
		.sanitized();
	let Command::FillRect {
		rect,
		radii,
		paint,
		border,
	} = &scene.commands[0]
	else {
		panic!("a rect");
	};
	assert_eq!(*rect, Rect::new(0.0, 0.0, 0.0, 40.0));
	assert_eq!(*radii, CornerRadii::all(0.0));
	assert_eq!(*paint, Paint::Solid(Color::rgba(0.0, 1.0, 0.0, 0.5)));
	assert_eq!(border.map(|border| border.width), Some(0.0));
	let Command::StrokePath { path, stroke, .. } = &scene.commands[1] else {
		panic!("a stroke");
	};
	assert_eq!(
		path.commands,
		[PathCommand::LineTo(Point::new(0.0, MAX_COORDINATE))]
	);
	assert_eq!((stroke.width, stroke.miter_limit), (0.0, 1.0));
}

#[test]
fn gradient_stops_are_ordered_and_capped() {
	let stop = |offset| GradientStop::new(offset, Color::WHITE);
	let mut stops = vec![stop(0.5), stop(0.2), stop(f32::NAN), stop(2.0)];
	stops.extend((0..40).map(|_| stop(1.0)));
	let paint = Paint::RadialGradient {
		center: Point::new(0.0, 0.0),
		radius: -4.0,
		stops,
	}
	.sanitized();
	let Paint::RadialGradient { radius, stops, .. } = paint else {
		panic!("a radial gradient");
	};
	assert_eq!(radius, 0.0);
	assert_eq!(stops.len(), MAX_GRADIENT_STOPS);
	let offsets: Vec<f32> = stops.iter().take(4).map(|stop| stop.offset).collect();
	assert_eq!(offsets, [0.5, 0.5, 0.5, 1.0]);
	let empty = Paint::LinearGradient {
		start: Point::default(),
		end: Point::default(),
		stops: Vec::new(),
	};
	assert_eq!(empty.sanitized(), Paint::Solid(Color::TRANSPARENT));
}

#[test]
fn pops_match_pushes_and_open_pushes_close() {
	let rect = Rect::new(0.0, 0.0, 1.0, 1.0);
	let scene = Scene::new()
		.pop_clip()
		.push_layer(2.0)
		.push_clip(rect, 0.0)
		.pop_layer()
		.pop_clip()
		.push_transform(Affine::IDENTITY)
		.sanitized();
	assert_eq!(
		scene.commands,
		[
			Command::PushLayer { opacity: 1.0 },
			Command::PushClip {
				rect,
				radii: CornerRadii::default()
			},
			Command::PopClip,
			Command::PushTransform(Affine::IDENTITY),
			Command::PopTransform,
			Command::PopLayer,
		]
	);
}

#[test]
fn deep_nesting_is_dropped_with_its_pops() {
	let mut scene = Scene::new();
	for _ in 0..MAX_DEPTH + 10 {
		scene = scene.push_transform(Affine::IDENTITY);
	}
	for _ in 0..MAX_DEPTH + 10 {
		scene = scene.pop_transform();
	}
	let kept = scene.sanitized().commands;
	assert_eq!(kept.len(), MAX_DEPTH * 2);

	let mut layers = Scene::new();
	for _ in 0..MAX_LAYER_DEPTH + 3 {
		layers = layers.push_layer(1.0);
	}
	let pushes = layers
		.sanitized()
		.commands
		.iter()
		.filter(|command| matches!(command, Command::PushLayer { .. }))
		.count();
	assert_eq!(pushes, MAX_LAYER_DEPTH);
}

#[test]
fn command_and_path_counts_are_capped() {
	let mut scene = Scene::new();
	for _ in 0..MAX_COMMANDS + 5 {
		scene = scene.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::BLACK);
	}
	assert_eq!(scene.sanitized().commands.len(), MAX_COMMANDS);

	let mut path = Path::new();
	for i in 0..MAX_PATH_COMMANDS + 5 {
		path = path.line_to(i as f32, 0.0);
	}
	let mut scene = Scene::new();
	for _ in 0..20 {
		scene = scene.fill_path(path.clone(), FillRule::NonZero, Color::BLACK);
	}
	let total: usize = scene
		.sanitized()
		.commands
		.iter()
		.map(|command| {
			match command {
				Command::FillPath { path, .. } => path.commands.len(),
				_ => 0,
			}
		})
		.sum();
	assert_eq!(total, MAX_SCENE_PATH_COMMANDS);
}

#[test]
fn gradients_past_the_limit_become_their_first_colour() {
	let gradient = Paint::LinearGradient {
		start: Point::new(0.0, 0.0),
		end: Point::new(1.0, 0.0),
		stops: vec![GradientStop::new(0.0, Color::WHITE)],
	};
	let mut scene = Scene::new();
	for _ in 0..=MAX_GRADIENTS {
		scene = scene.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), gradient.clone());
	}
	let scene = scene.sanitized();
	let Some(Command::FillRect { paint, .. }) = scene.commands.last() else {
		panic!("a rect");
	};
	assert_eq!(*paint, Paint::Solid(Color::WHITE));
}

#[test]
fn transforms_compose_in_canvas_order() {
	let transform = Affine::scale(2.0, 2.0).then(Affine::translate(10.0, 0.0));
	assert_eq!(transform.apply(Point::new(1.0, 1.0)), Point::new(12.0, 2.0));
}

#[cfg(feature = "serde")]
#[test]
fn scenes_round_trip_through_json() {
	let scene = Scene::new()
		.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), 2.0)
		.fill_path(
			Path::new()
				.move_to(0.0, 0.0)
				.quad_to(5.0, 5.0, 10.0, 0.0)
				.close(),
			FillRule::EvenOdd,
			Color::BLACK,
		)
		.pop_clip();
	let json = serde_json::to_value(&scene).expect("serialize");
	assert_eq!(json[1]["type"], "fill_path");
	assert_eq!(json[1]["rule"], "even_odd");
	assert_eq!(json[1]["path"][0]["move_to"]["x"], 0.0);
	let back: Scene = serde_json::from_value(json).expect("deserialize");
	assert_eq!(back, scene);
}
