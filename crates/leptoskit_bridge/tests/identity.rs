//! Transport identity remains usable independently of its owner's lifetime.

use leptoskit_bridge::Bridge;

#[test]
fn identity_survives_moves_and_retained_handles_prevent_reuse() {
	let bridge = Bridge::default();
	let identity = bridge.identity();
	let moved = Box::new(bridge);

	assert_eq!(identity, moved.identity());
	assert_eq!(identity, identity.clone());

	drop(moved);

	for _ in 0..128 {
		assert_ne!(identity, Bridge::default().identity());
	}
}

#[test]
fn replacement_has_a_distinct_identity() {
	let mut bridge = Bridge::default();
	let original = bridge.identity();
	let previous = std::mem::take(&mut bridge);

	assert_eq!(original, previous.identity());
	assert_ne!(original, bridge.identity());
}
