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
	let screen = gallery::find_screen("counter").expect("the counter exists");
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(screen.viewport.0, screen.viewport.1);
	let state = ScreenState::new(Assets::at("assets"));
	surface.set_root(&screen.render(&state));

	let button = |surface: &NativeSurface<HeadlessBackend>, title: &str| {
		find(surface, surface.root().expect("root"), title).expect("the button exists")
	};
	let add = button(&surface, "Add one");
	surface.dispatch_event(add, &leptoskit_native::NativeEvent::Press);
	assert_eq!(state.count(), ScreenState::INITIAL_COUNT + 1);

	let reset = button(&surface, "Reset");
	surface.dispatch_event(reset, &leptoskit_native::NativeEvent::Press);
	surface.set_root(&screen.render(&state));
	assert_eq!(state.count(), 0);

	// The count never goes negative.
	let minus = button(&surface, "−");
	surface.dispatch_event(minus, &leptoskit_native::NativeEvent::Press);
	assert_eq!(state.count(), 0);
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

fn find(surface: &NativeSurface<HeadlessBackend>, id: NodeId, text: &str) -> Option<NodeId> {
	if surface.props(id).text.as_deref() == Some(text) {
		return Some(id);
	}
	surface
		.children(id)
		.iter()
		.find_map(|&child| find(surface, child, text))
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
