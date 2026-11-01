#![cfg(all(feature = "native", not(target_arch = "wasm32")))]

use std::time::Duration;

use leptoskit_rive::Asset;
use leptoskit_rive::Error;
use leptoskit_rive::Input;
use leptoskit_rive::Name;
use leptoskit_rive::Scene;
use leptoskit_rive::native::Options;
use leptoskit_rive::native::Player;

fn asset() -> Asset {
	Asset::new(include_bytes!(
		"fixtures/scene/build/leptoskit-rive-test.riv"
	))
	.unwrap()
}

#[test]
fn actual_native_pixels_advance_pause_and_reuse_the_backing_buffer() {
	let mut player = Player::new(&asset(), &Options::default()).unwrap();
	let frame = player.render(Duration::ZERO, 128, 128).unwrap().unwrap();
	assert!(
		frame
			.bytes()
			.as_chunks::<4>()
			.0
			.iter()
			.any(|pixel| pixel[3] != 0)
	);
	assert!(
		frame
			.bytes()
			.as_chunks::<4>()
			.0
			.contains(&[0x57, 0xa5, 0xe0, 0xff])
	);
	let initial = frame.bytes().to_vec();
	let pointer = frame.bytes().as_ptr();
	assert!(!player.needs_frame());
	assert!(
		player
			.render(Duration::from_secs(1), 128, 128)
			.unwrap()
			.is_none()
	);

	player.play();
	let frame = player
		.render(Duration::from_millis(100), 128, 128)
		.unwrap()
		.unwrap();
	assert_eq!(pointer, frame.bytes().as_ptr());
	assert_ne!(initial, frame.bytes());
	player.pause();
	assert!(
		player
			.render(Duration::from_millis(100), 128, 128)
			.unwrap()
			.is_none()
	);
}

#[test]
fn native_visibility_motion_and_size_bounds_stop_work() {
	let mut player = Player::new(
		&asset(),
		&Options {
			autoplay: true,
			..Options::default()
		},
	)
	.unwrap();
	player.set_visible(false);
	assert!(!player.needs_frame());
	assert!(player.render(Duration::ZERO, 128, 128).unwrap().is_none());
	player.set_visible(true);
	player.set_reduced_motion(true);
	player.render(Duration::ZERO, 128, 128).unwrap().unwrap();
	assert!(!player.needs_frame());

	player.set_reduced_motion(false);
	assert!(player.needs_frame());
	assert_eq!(
		player.render(Duration::ZERO, 0, 128).unwrap_err(),
		Error::InvalidFrame
	);
	assert_eq!(
		player.render(Duration::ZERO, 4097, 1).unwrap_err(),
		Error::InvalidFrame
	);
	assert_eq!(
		player.render(Duration::ZERO, 4096, 4096).unwrap_err(),
		Error::InvalidFrame
	);
	assert!(player.render(Duration::ZERO, 256, 128).unwrap().is_some());
}

#[test]
fn native_scene_and_input_failures_are_reported() {
	assert!(
		Player::new(
			&asset(),
			&Options {
				scene: Scene::Animation(Name::new("missing").unwrap()),
				..Options::default()
			}
		)
		.is_err()
	);
	assert!(
		Player::new(
			&asset(),
			&Options {
				artboard: Some(Name::new("missing").unwrap()),
				..Options::default()
			}
		)
		.is_err()
	);
	let mut player = Player::new(
		&asset(),
		&Options {
			scene: Scene::StateMachine(Name::new("Machine").unwrap()),
			..Options::default()
		},
	)
	.unwrap();
	player
		.set_input(&Name::new("amount").unwrap(), Input::Number(0.5))
		.unwrap();
	player
		.set_input(&Name::new("enabled").unwrap(), Input::Boolean(true))
		.unwrap();
	player
		.set_input(&Name::new("activate").unwrap(), Input::Trigger)
		.unwrap();
	assert!(
		player
			.set_input(&Name::new("amount").unwrap(), Input::Boolean(true))
			.is_err()
	);
	assert!(
		player
			.set_input(&Name::new("missing").unwrap(), Input::Trigger)
			.is_err()
	);

	assert_eq!(
		player.set_input(&Name::new("amount").unwrap(), Input::Number(f64::NAN)),
		Err(Error::InvalidNumber)
	);
	assert!(player.render(Duration::ZERO, 128, 128).unwrap().is_some());
}

#[test]
fn unsupported_objects_on_unused_artboards_reject_the_whole_file() {
	let asset = Asset::new(include_bytes!(
		"fixtures/unsupported/build/leptoskit-rive-unsupported-test.riv"
	))
	.unwrap();
	assert!(matches!(
		Player::new(&asset, &Options::default()),
		Err(Error::UnsupportedFeature)
	));
}

#[test]
fn duplicate_input_names_cannot_select_different_inputs_by_type() {
	let asset = Asset::new(include_bytes!(
		"fixtures/ambiguous/build/leptoskit-rive-ambiguous-test.riv"
	))
	.unwrap();
	let result = Player::new(
		&asset,
		&Options {
			scene: Scene::StateMachine(Name::new("Ambiguous").unwrap()),
			..Options::default()
		},
	);

	assert!(matches!(result, Err(Error::Runtime(message)) if message.contains("input")));
}

#[test]
fn native_players_have_independent_lifetimes_and_import_errors_release_resources() {
	let mut first = Player::new(&asset(), &Options::default()).unwrap();
	let mut second = Player::new(&asset(), &Options::default()).unwrap();
	let initial = second
		.render(Duration::ZERO, 128, 128)
		.unwrap()
		.unwrap()
		.bytes()
		.to_vec();
	first.render(Duration::ZERO, 128, 128).unwrap();
	first.play();
	assert_ne!(
		first
			.render(Duration::from_millis(100), 128, 128)
			.unwrap()
			.unwrap()
			.bytes(),
		initial
	);
	drop(first);
	second.pause();

	assert!(second.render(Duration::ZERO, 128, 128).unwrap().is_none());
	let malformed = Asset::new(b"RIVE\x07\x00\x00\x00").unwrap();

	for _ in 0..100 {
		assert!(Player::new(&malformed, &Options::default()).is_err());
	}

	second.play();
	assert!(
		second
			.render(Duration::from_millis(16), 128, 128)
			.unwrap()
			.is_some()
	);
}

#[test]
fn vector_paints_strokes_clipping_and_premultiplied_alpha_reach_real_pixels() {
	let asset = Asset::new(include_bytes!(
		"fixtures/native/build/leptoskit-rive-native-test.riv"
	))
	.unwrap();
	let mut player = Player::new(&asset, &Options::default()).unwrap();
	let frame = player.render(Duration::ZERO, 128, 128).unwrap().unwrap();
	let pixel = |x: usize, y: usize| &frame.bytes()[(y * 128 + x) * 4..(y * 128 + x + 1) * 4];
	assert!(pixel(16, 32)[0] > 200 && pixel(16, 32)[2] < 60);
	assert!(pixel(48, 32)[2] > 200 && pixel(48, 32)[0] < 60);
	assert!(pixel(96, 32)[3] > 230 && pixel(116, 32)[3] < 60);
	assert!(
		pixel(116, 32)
			.iter()
			.all(|value| *value == pixel(116, 32)[3]),
		"white gradients must remain premultiplied"
	);

	assert_eq!(pixel(32, 96), [0, 128, 0, 128]);
	assert_eq!(pixel(10, 96), [255, 128, 0, 255]);
	assert_eq!(pixel(96, 96), [0, 255, 255, 255]);
	assert_eq!(pixel(80, 80), [0, 0, 0, 0]);
	assert_eq!(pixel(2, 2), [0, 0, 0, 0]);
	// Keep the exact native output reviewable next to the CLI reference image.
	let output =
		std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/rive-native");
	std::fs::create_dir_all(&output).unwrap();
	let pixels = tiny_skia::Pixmap::from_vec(
		frame.bytes().to_vec(),
		tiny_skia::IntSize::from_wh(128, 128).unwrap(),
	)
	.unwrap();
	pixels.save_png(output.join("vectors.png")).unwrap();
	let mut preview = tiny_skia::Pixmap::new(128, 128).unwrap();
	preview.fill(tiny_skia::Color::BLACK);
	preview.draw_pixmap(
		0,
		0,
		pixels.as_ref(),
		&tiny_skia::PixmapPaint::default(),
		tiny_skia::Transform::identity(),
		None,
	);
	preview.save_png(output.join("vectors-black.png")).unwrap();
	assert!(
		!player.needs_frame(),
		"a static artboard has no idle render work"
	);
}
