//! Renders small scenes on the GPU and checks their pixels. Machines with no
//! GPU adapter (some CI runners) skip these tests.
#![cfg(all(feature = "renderer", not(target_arch = "wasm32")))]

use leptoskit_canvas::*;

const RED: Color = Color::rgb8(255, 0, 0);
const BLUE: Color = Color::rgb8(0, 0, 255);

struct Image {
	pixels: Vec<u8>,
	width: u32,
}

impl Image {
	fn at(&self, x: u32, y: u32) -> [u8; 4] {
		let i = ((y * self.width + x) * 4) as usize;
		[
			self.pixels[i],
			self.pixels[i + 1],
			self.pixels[i + 2],
			self.pixels[i + 3],
		]
	}

	fn alpha(&self, x: u32, y: u32) -> u8 {
		self.at(x, y)[3]
	}
}

/// Renders at one pixel per point, or `None` when there is no GPU.
fn render(scene: &Scene, width: u32, height: u32) -> Option<Image> {
	render_scaled(scene, width, height, 1.0)
}

fn render_scaled(scene: &Scene, width: u32, height: u32, scale: f32) -> Option<Image> {
	match render_to_rgba(scene, width, height, scale) {
		Ok(pixels) => {
			assert_eq!(pixels.len(), (width * height * 4) as usize);
			Some(Image { pixels, width })
		}
		Err(error) if error.is_unavailable() => {
			eprintln!("skipping: {error}");
			None
		}
		Err(error) => panic!("{error}"),
	}
}

fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) -> bool {
	actual
		.iter()
		.zip(expected)
		.all(|(a, e)| a.abs_diff(e) <= tolerance)
}

#[test]
fn solid_rect_covers_exactly_its_pixels() {
	let scene = Scene::new().fill_rect(Rect::new(10.0, 10.0, 20.0, 20.0), RED);
	let Some(image) = render(&scene, 40, 40) else {
		return;
	};
	assert_eq!(image.at(20, 20), [255, 0, 0, 255]);
	assert_eq!(image.at(10, 10), [255, 0, 0, 255]);
	assert_eq!(image.at(29, 29), [255, 0, 0, 255]);
	assert_eq!(image.at(9, 20), [0; 4]);
	assert_eq!(image.at(30, 20), [0; 4]);
	assert_eq!(image.at(0, 0), [0; 4]);
}

#[test]
fn rounded_corners_are_transparent_and_smooth() {
	let scene = Scene::new().fill_rounded_rect(Rect::new(0.0, 0.0, 40.0, 40.0), 20.0, RED);
	let Some(image) = render(&scene, 40, 40) else {
		return;
	};
	assert_eq!(image.alpha(20, 20), 255);
	assert_eq!(image.alpha(20, 1), 255);
	assert_eq!(image.alpha(1, 1), 0);
	assert_eq!(image.alpha(38, 38), 0);
	assert_eq!(image.alpha(4, 4), 0);
	let partial = (0..40).any(|x| (1..255).contains(&image.alpha(x, 3)));
	assert!(partial, "the curved edge has partly covered pixels");
}

#[test]
fn border_is_drawn_inside_the_edge() {
	let scene = Scene::new().with(Command::FillRect {
		rect: Rect::new(10.0, 10.0, 40.0, 40.0),
		radii: CornerRadii::default(),
		paint: Color::WHITE.into(),
		border: Some(Border {
			width: 5.0,
			color: Color::BLACK,
		}),
	});
	let Some(image) = render(&scene, 60, 60) else {
		return;
	};
	assert_eq!(image.at(12, 30), [0, 0, 0, 255]);
	assert_eq!(image.at(47, 30), [0, 0, 0, 255]);
	assert_eq!(image.at(30, 30), [255, 255, 255, 255]);
	assert_eq!(image.at(16, 30), [255, 255, 255, 255]);
	assert_eq!(image.at(8, 30), [0; 4]);
}

#[test]
fn linear_gradient_runs_between_its_end_colours() {
	let scene = Scene::new().fill_rect(
		Rect::new(0.0, 0.0, 100.0, 10.0),
		Paint::LinearGradient {
			start: Point::new(0.0, 0.0),
			end: Point::new(100.0, 0.0),
			stops: vec![GradientStop::new(0.0, RED), GradientStop::new(1.0, BLUE)],
		},
	);
	let Some(image) = render(&scene, 100, 10) else {
		return;
	};
	assert!(
		near(image.at(0, 5), [255, 0, 0, 255], 3),
		"{:?}",
		image.at(0, 5)
	);
	assert!(
		near(image.at(99, 5), [0, 0, 255, 255], 3),
		"{:?}",
		image.at(99, 5)
	);
	assert!(
		near(image.at(50, 5), [126, 0, 129, 255], 3),
		"{:?}",
		image.at(50, 5)
	);
}

#[test]
fn radial_gradient_fades_from_the_centre() {
	let scene = Scene::new().fill_rect(
		Rect::new(0.0, 0.0, 100.0, 100.0),
		Paint::RadialGradient {
			center: Point::new(50.0, 50.0),
			radius: 50.0,
			stops: vec![
				GradientStop::new(0.0, Color::WHITE),
				GradientStop::new(1.0, Color::BLACK),
			],
		},
	);
	let Some(image) = render(&scene, 100, 100) else {
		return;
	};
	assert!(
		near(image.at(50, 50), [255, 255, 255, 255], 4),
		"{:?}",
		image.at(50, 50)
	);
	assert!(
		near(image.at(75, 50), [127, 127, 127, 255], 4),
		"{:?}",
		image.at(75, 50)
	);
	assert_eq!(image.at(1, 1), [0, 0, 0, 255]);
}

#[test]
fn box_shadow_falls_off_outside_the_box() {
	let scene = Scene::new().box_shadow(
		Rect::new(30.0, 30.0, 40.0, 40.0),
		0.0,
		BoxShadow {
			blur: 20.0,
			color: Color::BLACK,
			..BoxShadow::default()
		},
	);
	let Some(image) = render(&scene, 100, 100) else {
		return;
	};
	// The centre is two standard deviations from each edge, so the blur
	// keeps erf(√2)² ≈ 91% of it, as a browser draws it.
	let centre = image.alpha(50, 50);
	assert!((228..=237).contains(&centre), "{centre}");
	let edge = image.alpha(70, 50);
	assert!(
		(96..=160).contains(&edge),
		"half covered at the edge: {edge}"
	);
	let row: Vec<u8> = (60..100).map(|x| image.alpha(x, 50)).collect();
	assert!(row.windows(2).all(|pair| pair[0] >= pair[1]), "{row:?}");
	assert!(image.alpha(99, 50) <= 2, "{}", image.alpha(99, 50));
	assert!(image.alpha(80, 50) > 10, "{}", image.alpha(80, 50));
}

#[test]
fn box_shadow_moves_by_its_offset_and_grows_by_its_spread() {
	let scene = Scene::new().box_shadow(
		Rect::new(10.0, 10.0, 20.0, 20.0),
		0.0,
		BoxShadow {
			offset_x: 30.0,
			offset_y: 0.0,
			blur: 0.0,
			spread: 5.0,
			color: BLUE,
		},
	);
	let Some(image) = render(&scene, 80, 40) else {
		return;
	};
	assert_eq!(image.at(20, 20), [0; 4]);
	assert_eq!(image.at(36, 20), [0, 0, 255, 255]);
	assert_eq!(image.at(64, 20), [0, 0, 255, 255]);
	assert_eq!(image.at(66, 20), [0; 4]);
}

#[test]
fn even_odd_fill_cuts_a_hole() {
	let rings = Path::new()
		.rect(Rect::new(10.0, 10.0, 80.0, 80.0))
		.rect(Rect::new(30.0, 30.0, 40.0, 40.0));
	let even_odd = Scene::new().fill_path(rings.clone(), FillRule::EvenOdd, RED);
	let non_zero = Scene::new().fill_path(rings, FillRule::NonZero, RED);
	let (Some(even_odd), Some(non_zero)) =
		(render(&even_odd, 100, 100), render(&non_zero, 100, 100))
	else {
		return;
	};
	assert_eq!(even_odd.at(50, 50), [0; 4]);
	assert_eq!(even_odd.at(20, 50), [255, 0, 0, 255]);
	assert_eq!(even_odd.at(5, 50), [0; 4]);
	assert_eq!(non_zero.at(50, 50), [255, 0, 0, 255]);
}

#[test]
fn curved_path_edges_are_smoothed() {
	let scene =
		Scene::new().fill_path(Path::new().circle(50.0, 50.0, 40.0), FillRule::NonZero, RED);
	let Some(image) = render(&scene, 100, 100) else {
		return;
	};
	assert_eq!(image.alpha(50, 50), 255);
	assert_eq!(image.alpha(5, 5), 0);
	let partial = (0..100)
		.filter(|&x| (1..255).contains(&image.alpha(x, 30)))
		.count();
	assert!(partial >= 2, "multisampling softens the curve");
}

#[test]
fn stroke_covers_its_width_and_caps() {
	let line = Path::new().move_to(10.0, 50.0).line_to(90.0, 50.0);
	let butt = Scene::new().stroke_path(line.clone(), Stroke::new(10.0), RED);
	let round = Scene::new().stroke_path(line, Stroke::new(10.0).cap(LineCap::Round), RED);
	let (Some(butt), Some(round)) = (render(&butt, 100, 100), render(&round, 100, 100)) else {
		return;
	};
	assert_eq!(butt.at(50, 45), [255, 0, 0, 255]);
	assert_eq!(butt.at(50, 54), [255, 0, 0, 255]);
	assert_eq!(butt.at(50, 44), [0; 4]);
	assert_eq!(butt.at(50, 55), [0; 4]);
	assert_eq!(butt.at(92, 50), [0; 4]);
	assert_eq!(round.at(92, 50), [255, 0, 0, 255]);
	assert_eq!(round.at(96, 50), [0; 4]);
}

#[test]
fn clips_limit_drawing_and_pop_restores_it() {
	let everything = Rect::new(0.0, 0.0, 100.0, 100.0);
	let scene = Scene::new()
		.push_clip(Rect::new(20.0, 20.0, 40.0, 40.0), 0.0)
		.fill_rect(everything, RED)
		.pop_clip()
		.fill_rect(Rect::new(80.0, 80.0, 10.0, 10.0), BLUE);
	let Some(image) = render(&scene, 100, 100) else {
		return;
	};
	assert_eq!(image.at(10, 10), [0; 4]);
	assert_eq!(image.at(20, 20), [255, 0, 0, 255]);
	assert_eq!(image.at(59, 59), [255, 0, 0, 255]);
	assert_eq!(image.at(60, 40), [0; 4]);
	assert_eq!(image.at(85, 85), [0, 0, 255, 255]);
}

#[test]
fn rounded_and_nested_clips_intersect() {
	let scene = Scene::new()
		.push_clip(Rect::new(0.0, 0.0, 60.0, 60.0), 30.0)
		.push_clip(Rect::new(30.0, 0.0, 60.0, 60.0), 0.0)
		.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), RED)
		.pop_clip()
		.pop_clip();
	let Some(image) = render(&scene, 100, 100) else {
		return;
	};
	assert_eq!(image.alpha(45, 30), 255);
	assert_eq!(image.alpha(15, 30), 0, "outside the inner clip");
	assert_eq!(image.alpha(57, 2), 0, "in the outer clip's rounded corner");
	assert_eq!(image.alpha(70, 30), 0, "outside the outer clip");
}

#[test]
fn layer_fades_its_contents_as_one() {
	let scene = Scene::new()
		.push_layer(0.5)
		.fill_rect(Rect::new(0.0, 0.0, 30.0, 20.0), RED)
		.fill_rect(Rect::new(10.0, 0.0, 30.0, 20.0), RED)
		.pop_layer()
		.fill_rect(Rect::new(0.0, 30.0, 10.0, 10.0), BLUE);
	let Some(image) = render(&scene, 40, 40) else {
		return;
	};
	for x in [5, 20, 35] {
		let [r, g, b, a] = image.at(x, 10);
		assert!(
			(126..=129).contains(&a) && r == a && g == 0 && b == 0,
			"{:?}",
			image.at(x, 10)
		);
	}
	assert_eq!(image.at(5, 35), [0, 0, 255, 255]);
}

#[test]
fn layers_respect_the_clip_around_them() {
	let scene = Scene::new()
		.push_clip(Rect::new(0.0, 0.0, 20.0, 40.0), 0.0)
		.push_layer(1.0)
		.fill_rect(Rect::new(0.0, 0.0, 40.0, 40.0), RED)
		.pop_layer()
		.pop_clip();
	let Some(image) = render(&scene, 40, 40) else {
		return;
	};
	assert_eq!(image.at(10, 10), [255, 0, 0, 255]);
	assert_eq!(image.at(30, 10), [0; 4]);
}

#[test]
fn transforms_move_rects_and_paths() {
	let scene = Scene::new()
		.push_transform(Affine::translate(50.0, 0.0))
		.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), RED)
		.push_transform(Affine::translate(0.0, 20.0))
		.fill_path(
			Path::new().rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
			FillRule::NonZero,
			BLUE,
		)
		.pop_transform()
		.pop_transform()
		.push_transform(
			Affine::rotate(std::f32::consts::FRAC_PI_4).then(Affine::translate(20.0, 60.0)),
		)
		.fill_rect(Rect::new(-5.0, -5.0, 10.0, 10.0), RED)
		.pop_transform();
	let Some(image) = render(&scene, 80, 80) else {
		return;
	};
	assert_eq!(image.at(5, 5), [0; 4]);
	assert_eq!(image.at(55, 5), [255, 0, 0, 255]);
	assert_eq!(image.at(55, 25), [0, 0, 255, 255]);
	// A square turned 45° reaches √50 ≈ 7.07 along each axis, so its
	// corners are clear while its points stick out past the unturned edge.
	assert_eq!(image.alpha(20, 65), 255);
	assert_eq!(image.alpha(14, 54), 0);
}

#[test]
fn scale_maps_points_to_pixels() {
	let scene = Scene::new().fill_rect(Rect::new(10.0, 10.0, 10.0, 10.0), RED);
	let Some(image) = render_scaled(&scene, 60, 60, 2.0) else {
		return;
	};
	assert_eq!(image.at(20, 20), [255, 0, 0, 255]);
	assert_eq!(image.at(39, 39), [255, 0, 0, 255]);
	assert_eq!(image.at(40, 40), [0; 4]);
	assert_eq!(image.at(19, 19), [0; 4]);
}

#[test]
fn hostile_scenes_render_without_panicking() {
	let nan = f32::NAN;
	let mut scene = Scene::new()
		.fill_rect(
			Rect::new(nan, f32::INFINITY, -5.0, 1e30),
			Color::rgba(nan, 2.0, -1.0, 1.0),
		)
		.push_transform(Affine([nan, 1e30, 0.0, 0.0, f32::INFINITY, 0.0]))
		.stroke_path(
			Path::new().move_to(0.0, 0.0).cubic_to(
				Point::new(1e30, -1e30),
				Point::new(nan, 0.0),
				Point::new(5.0, 5.0),
			),
			Stroke::new(f32::INFINITY).miter_limit(-3.0),
			Paint::LinearGradient {
				start: Point::new(0.0, 0.0),
				end: Point::new(0.0, 0.0),
				stops: Vec::new(),
			},
		)
		.pop_layer()
		.pop_clip();
	for _ in 0..100 {
		scene = scene
			.push_layer(0.5)
			.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), 99.0);
	}
	let scene = scene.box_shadow(
		Rect::new(0.0, 0.0, 10.0, 10.0),
		1e9,
		BoxShadow {
			blur: 1e9,
			spread: -1e9,
			..BoxShadow::default()
		},
	);
	render(&scene, 16, 16);
}

#[test]
fn renderer_draws_repeatedly_into_a_bgra_target() {
	let Ok((device, queue)) = headless_device() else {
		eprintln!("skipping: no GPU");
		return;
	};
	let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Bgra8Unorm);
	let small = Scene::new().fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), RED);
	let gradient = Scene::new().push_layer(1.0).fill_rect(
		Rect::new(0.0, 0.0, 8.0, 8.0),
		Paint::LinearGradient {
			start: Point::new(0.0, 0.0),
			end: Point::new(8.0, 0.0),
			stops: vec![GradientStop::new(0.0, BLUE), GradientStop::new(1.0, BLUE)],
		},
	);
	let first = renderer.render_to_rgba(&small, 8, 8, 1.0).expect("render");
	let second = renderer
		.render_to_rgba(&gradient, 16, 16, 1.0)
		.expect("render");
	assert_eq!(&first[..4], &[255, 0, 0, 255]);
	assert_eq!(&second[..4], &[0, 0, 255, 255]);
	assert_eq!(&second[(9 * 4)..(9 * 4 + 4)], &[0; 4]);
}
