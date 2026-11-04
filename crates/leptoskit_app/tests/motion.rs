#![cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use leptoskit::rive::native::Options;
use leptoskit::rive::native::Player;

#[test]
fn the_original_brand_scene_renders_moves_and_stops_for_reduced_motion() {
	let playback = leptoskit_app::motion::brand_playback();
	let mut player = Player::new(
		playback.asset(),
		&Options {
			scene: playback.selected_scene().clone(),
			autoplay: true,
			..Options::default()
		},
	)
	.unwrap();
	let initial = player
		.render(Duration::ZERO, 128, 128)
		.unwrap()
		.unwrap()
		.bytes()
		.to_vec();
	assert!(
		initial
			.as_chunks::<4>()
			.0
			.iter()
			.filter(|pixel| pixel[3] > 0)
			.count()
			> 4000
	);
	let moved = player
		.render(Duration::from_millis(100), 128, 128)
		.unwrap()
		.unwrap()
		.bytes()
		.to_vec();
	assert_ne!(initial, moved);
	player.set_reduced_motion(true);
	assert!(!player.needs_frame());
	assert!(
		player
			.render(Duration::from_millis(100), 128, 128)
			.unwrap()
			.is_none()
	);
	player.set_reduced_motion(false);
	for _ in 0..10 {
		let _ = player.render(Duration::from_millis(100), 128, 128).unwrap();
	}
	assert!(
		!player.needs_frame(),
		"the one-shot timeline must not keep requesting frames"
	);
}
