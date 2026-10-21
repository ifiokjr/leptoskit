//! Shared host behavior; platform widget suites separately exercise real input.

use gallery::Assets;
use gallery::ScreenState;
use gallery::native::NativeGallery;
use gallery::native::screen;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::NodeId;

fn text(surface: &NativeSurface<HeadlessBackend>, node: NodeId, expected: &str) -> Option<NodeId> {
	if surface.props(node).text.as_deref() == Some(expected) {
		return Some(node);
	}

	surface
		.children(node)
		.iter()
		.find_map(|child| text(surface, *child, expected))
}

#[test]
fn native_host_dispatch_commits_the_shared_counter_and_preserves_control_identity() {
	let screen = screen("counter").unwrap();
	let mut gallery = NativeGallery::new(
		HeadlessBackend::new(),
		screen,
		Assets::at("assets"),
		screen.viewport,
	);
	let root = gallery.surface.root().unwrap();
	let button = text(&gallery.surface, root, "Add one").unwrap();
	gallery.dispatch(button, &NativeEvent::Press);
	assert_eq!(gallery.state.count(), ScreenState::INITIAL_COUNT + 1);
	assert_eq!(text(&gallery.surface, root, "Add one"), Some(button));
	assert!(text(&gallery.surface, root, "13").is_some());

	gallery.surface.set_viewport(780.0, 480.0);
	let frame = gallery.surface.frame(root).unwrap();
	assert_eq!((frame.width, frame.height), (780.0, 480.0));
	assert_eq!(text(&gallery.surface, root, "Add one"), Some(button));
	assert_eq!(gallery.state.count(), 13);
}

#[test]
fn unknown_screen_lists_choices_without_creating_native_widgets() {
	let error = screen("missing").unwrap_err();
	assert!(error.contains("no screen named `missing`"));
	assert!(error.contains("counter, profile, settings, photos, chat, dashboard, components"));
}

#[test]
fn phone_studio_keeps_readable_cards_in_a_scrolling_document_after_rotation() {
	let screen = screen("components").unwrap();
	let mut gallery = NativeGallery::new(
		HeadlessBackend::new(),
		screen,
		Assets::at("assets"),
		(390.0, 844.0),
	);
	let root = gallery.surface.root().unwrap();

	for viewport in [(390.0, 844.0), (844.0, 390.0)] {
		gallery.surface.set_viewport(viewport.0, viewport.1);
		assert_eq!(gallery.surface.root(), Some(root));
		assert_eq!(gallery.surface.kind(root), NativeKind::ScrollView);
		let cards = gallery.surface.children(root)[1];
		let cards_frame = gallery.surface.frame(cards).unwrap();
		assert!(cards_frame.height > viewport.1);

		for heading in [
			"Make it yours",
			"Small pieces, more room",
			"A place for every idea",
		] {
			let heading = text(&gallery.surface, root, heading).unwrap();
			assert!(gallery.surface.frame(heading).unwrap().width > 250.0);
		}
	}
}
