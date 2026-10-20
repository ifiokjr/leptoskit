//! Every gallery screen lays out cleanly under the headless backend: the
//! root fills the viewport, no node collapses to nothing, and nothing spills
//! out of its parent except the content of a scroll view.

use gallery::Assets;
use gallery::SCREENS;
use gallery::Screen;
use gallery::ScreenState;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_yoga::NodeId;

/// Layout is float arithmetic; a hundredth of a point is far below a pixel.
const TOLERANCE: f32 = 0.01;

fn lay_out(
	screen: &Screen,
	measurer: Option<SharedTextMeasurer>,
) -> NativeSurface<HeadlessBackend> {
	let backend = match measurer {
		Some(measurer) => HeadlessBackend::new().with_text_measurer(measurer),
		None => HeadlessBackend::new(),
	};
	let mut surface = NativeSurface::new(backend);
	let (width, height) = screen.viewport;
	surface.set_viewport(width, height);
	// The trees only carry image paths, so nothing needs to exist on disk.
	let state = ScreenState::new(Assets::at("assets"));
	surface.set_root(&screen.render(&state));
	surface
}

/// Describes a node for failure messages: its kind and text, if any.
fn describe(surface: &NativeSurface<HeadlessBackend>, id: NodeId) -> String {
	let props = surface.props(id);
	match props.text.as_deref().or(props.source.as_deref()) {
		Some(content) => format!("{} {content:?}", surface.kind(id)),
		None => surface.kind(id).to_string(),
	}
}

/// Every problem found under `id`.
fn problems(surface: &NativeSurface<HeadlessBackend>, id: NodeId, found: &mut Vec<String>) {
	let frame = surface.frame(id).expect("every committed node has a frame");
	// An empty, unstyled view (a spacer) draws nothing, so it may collapse.
	let props = surface.props(id);
	let invisible = surface.kind(id) == NativeKind::View
		&& surface.children(id).is_empty()
		&& props.background.is_none()
		&& props.border_width.is_none();
	if !invisible && (frame.width <= 0.0 || frame.height <= 0.0) {
		found.push(format!(
			"{} is {}×{}",
			describe(surface, id),
			frame.width,
			frame.height
		));
	}
	let scrolls = surface.kind(id) == NativeKind::ScrollView;
	for &child in surface.children(id) {
		let inner = surface
			.frame(child)
			.expect("every committed node has a frame");
		let inside = inner.x >= -TOLERANCE
			&& inner.y >= -TOLERANCE
			&& inner.x + inner.width <= frame.width + TOLERANCE
			&& inner.y + inner.height <= frame.height + TOLERANCE;
		// Overflowing a scroll view along its axis is what scrolling is for.
		let allowed = scrolls && inner.x + inner.width <= frame.width + TOLERANCE;
		if !inside && !allowed {
			found.push(format!(
				"{} at ({}, {}) {}×{} overflows {} ({}×{})",
				describe(surface, child),
				inner.x,
				inner.y,
				inner.width,
				inner.height,
				describe(surface, id),
				frame.width,
				frame.height
			));
		}
		problems(surface, child, found);
	}
}

fn assert_clean(measurer: impl Fn() -> Option<SharedTextMeasurer>) {
	for screen in &SCREENS {
		let surface = lay_out(screen, measurer());
		let root = surface.root().expect("the screen commits a root");
		let frame = surface.frame(root).expect("the root has a frame");
		assert_eq!(
			(frame.width, frame.height),
			screen.viewport,
			"`{}` does not fill its viewport",
			screen.slug
		);
		let mut found = Vec::new();
		problems(&surface, root, &mut found);
		assert!(
			found.is_empty(),
			"`{}` lays out badly:\n{}",
			screen.slug,
			found.join("\n")
		);
	}
}

#[test]
fn every_screen_lays_out_with_the_approximate_measurer() {
	assert_clean(|| None);
}

#[cfg(target_os = "macos")]
#[test]
fn every_screen_lays_out_with_appkit_font_metrics() {
	assert_clean(|| {
		Some(std::sync::Arc::new(
			leptoskit_appkit::AppKitTextMeasurer::new(),
		))
	});
}

#[test]
fn slugs_are_unique_and_found() {
	for screen in &SCREENS {
		let found = gallery::find_screen(screen.slug).expect("every slug resolves");
		assert_eq!(found.title, screen.title);
	}
	assert!(gallery::find_screen("missing").is_none());
}

#[test]
fn counter_buttons_change_the_count() {
	use std::sync::Arc;
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	let screen = gallery::find_screen("counter").expect("the counter exists");
	let changes = Arc::new(AtomicUsize::new(0));
	let notifications = Arc::clone(&changes);
	let state = ScreenState::new(Assets::at("assets")).on_change(move || {
		notifications.fetch_add(1, Ordering::SeqCst);
	});
	let mut tester = leptoskit_testing::WidgetTester::new(
		move || screen.render(&state),
		screen.viewport.0,
		screen.viewport.1,
	)
	.unwrap();
	let text = |value: &str| leptoskit_testing::Finder::text(value);
	tester.tap(&text("Add one")).unwrap();
	tester
		.find(&text(&(ScreenState::INITIAL_COUNT + 1).to_string()))
		.unwrap();
	tester.tap(&text("Reset")).unwrap();
	tester.find(&text("0")).unwrap();
	tester.tap(&text("−")).unwrap();
	tester.find(&text("0")).unwrap();
	assert_eq!(changes.load(Ordering::SeqCst), 3);
}

#[test]
fn settings_switches_toggle() {
	let state = ScreenState::new(Assets::at("assets"));
	assert!(state.is_on("Haptics"));
	state.toggle("Haptics")();
	assert!(!state.is_on("Haptics"));
	state.toggle("Haptics")();
	assert!(state.is_on("Haptics"));
}

#[test]
fn settings_button_updates_its_track_and_knob() {
	let screen = gallery::find_screen("settings").expect("the settings screen exists");
	let state = ScreenState::new(Assets::at("assets"));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(screen.viewport.0, screen.viewport.1);
	surface.set_root(&screen.render(&state));
	let root = surface.root().expect("the screen commits a root");
	let switch =
		find_accessibility_label(&surface, root, "Haptics: on").expect("the switch exists");
	let knob = surface.children(switch)[0];
	let before = surface.frame(knob).expect("the knob has a frame");
	let track = surface.props(switch).background;

	surface.dispatch_event(switch, &leptoskit_native::NativeEvent::Press);
	surface.set_root(&screen.render(&state));

	assert!(!state.is_on("Haptics"));
	assert_eq!(
		surface.props(switch).accessibility_label.as_deref(),
		Some("Haptics: off")
	);
	assert_ne!(surface.props(switch).background, track);
	let after = surface.frame(knob).expect("the knob keeps a frame");
	assert!(after.x < before.x, "the off knob moves to the left");
}

fn find_accessibility_label(
	surface: &NativeSurface<HeadlessBackend>,
	id: NodeId,
	label: &str,
) -> Option<NodeId> {
	if surface.props(id).accessibility_label.as_deref() == Some(label) {
		return Some(id);
	}
	surface
		.children(id)
		.iter()
		.find_map(|&child| find_accessibility_label(surface, child, label))
}

#[test]
fn generated_assets_are_bitmaps() {
	let dir = std::env::temp_dir().join(format!("leptoskit-gallery-test-{}", std::process::id()));
	let assets = Assets::generate(&dir).expect("assets generate");
	for path in [assets.photo(0), assets.avatar(0), assets.cover()] {
		let bytes = std::fs::read(&path).expect("the asset exists");
		assert_eq!(&bytes[..2], b"BM", "{path} is a BMP");
		let declared = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
		assert_eq!(
			declared as usize,
			bytes.len(),
			"{path} declares its own size"
		);
	}
	std::fs::remove_dir_all(&dir).expect("clean up");
}

#[cfg(feature = "web")]
#[test]
fn browser_assets_embed_the_same_bitmap_format() {
	use base64::Engine;
	use base64::engine::general_purpose::STANDARD;

	let assets = Assets::for_browser().expect("the browser artwork generates");
	for source in [assets.photo(0), assets.avatar(0), assets.cover()] {
		let encoded = source
			.strip_prefix("data:image/bmp;base64,")
			.expect("a browser image is a self-contained bitmap");
		let bytes = STANDARD.decode(encoded).expect("the bitmap is base64");
		assert_eq!(&bytes[..2], b"BM");
		let declared = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
		assert_eq!(declared as usize, bytes.len());
	}
}
