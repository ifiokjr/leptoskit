//! The counter example: a native interface built with the leptoskit renderer,
//! run headless, and shipped as a code push release.
//!
//! This is the whole stack in one binary: the element tree, the yoga layout,
//! the mutation diffing, and the release pipeline over the wasm bundle the
//! shell would otherwise carry.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::FlexDirection;

/// Builds the counter interface for a count.
fn counter_ui(count: u32, presses: &Arc<AtomicU32>) -> Element {
	Element::view()
		.with_style(|style| {
			style.flex_direction = FlexDirection::Column;
			style.width = Dimension::Points(390.0);
			style.padding = leptoskit_yoga::Rect {
				top: Dimension::Points(16.0),
				right: Dimension::Points(16.0),
				bottom: Dimension::Points(16.0),
				left: Dimension::Points(16.0),
			};
			style.gap = leptoskit_yoga::Rect {
				top: Dimension::Points(8.0),
				right: Dimension::Points(0.0),
				bottom: Dimension::Points(8.0),
				left: Dimension::Points(0.0),
			};
		})
		.child(Element::text(format!("Count: {count}")).with_style(|style| style.flex_grow = 1.0))
		.child(Element::button("Increment").on(NativeEventSlot::Press, {
			let presses = Arc::clone(presses);
			move |_| {
				presses.fetch_add(1, Ordering::SeqCst);
				EventOutcome::Handled
			}
		}))
}

fn main() {
	let presses = Arc::new(AtomicU32::new(0));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);

	for count in 0..3 {
		surface.set_root(&counter_ui(count, &presses));
	}
	let root = surface.root().expect("root committed");
	println!("root frame: {:?}", surface.frame(root));
	println!(
		"mutations after two rebuilds: {}",
		surface.backend().log().len()
	);

	// Press the button: the headless surface dispatches like a platform shell.
	let button = surface.children(root)[1];
	surface.dispatch_event(button, &leptoskit_native::NativeEvent::Press);
	println!("presses after dispatch: {}", presses.load(Ordering::SeqCst));

	// Ship it: pack this example's own source into a code push release.
	if let Some(temp) = std::env::temp_dir().to_str() {
		let store_root = format!("{temp}/leptoskit_counter_example_releases");
		let scratch = format!("{temp}/leptoskit_counter_example_bundle");
		let _ = std::fs::remove_dir_all(&scratch);
		std::fs::create_dir_all(format!("{scratch}/assets")).expect("scratch dir");
		std::fs::write(
			format!("{scratch}/app.wasm"),
			b"\0asm the compiled interface bundle",
		)
		.expect("write bundle");
		match leptoskit_codepush::ReleaseStore::open(&store_root) {
			Ok(store) => {
				match store.ingest_directory(
					"counter",
					"production",
					&semver::Version::new(0, 1, 0),
					1,
					scratch.as_ref(),
					false,
				) {
					Ok(record) => {
						println!(
							"released counter 0.1.0+1: {} files, {:.1} KiB container",
							record.manifest.files.len(),
							record.container.len() as f64 / 1024.0
						);
					}
					Err(error) => println!("release failed: {error}"),
				}
			}
			Err(error) => println!("store failed: {error}"),
		}
	}
}
