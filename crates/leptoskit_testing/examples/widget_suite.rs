//! The same widget journey, compiled and run on each target OS.
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use leptoskit_testing::capture::LayoutImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let count = Arc::new(AtomicUsize::new(0));
	let mut tester = WidgetTester::new(
		move || {
			let action = Arc::clone(&count);
			Element::view()
				.child(Element::text(format!(
					"Count: {}",
					count.load(Ordering::SeqCst)
				)))
				.child(Element::button("Add").on_press(move || {
					action.fetch_add(1, Ordering::SeqCst);
				}))
		},
		390.0,
		844.0,
	)?;
	tester.find(&Finder::text("Count: 0"))?;
	tester.tap(&Finder::text("Add"))?;
	tester.find(&Finder::text("Count: 1"))?;
	tester.tap(&Finder::text("Add"))?;
	tester.find(&Finder::text("Count: 2"))?;
	// The flat-colour layout mode is computed from the shared layout tree, so
	// it must produce the same kind of image on every target OS: outlines and
	// distinct fills for the root and its controls, but never more colours
	// than its palette allows.
	let colors = tester
		.layout_image()?
		.distinct_colors(LayoutImage::MAX_COLORS)?;
	if !(3..=LayoutImage::MAX_COLORS).contains(&colors) {
		return Err(format!("layout image has {colors} colours, expected flat colour").into());
	}
	println!("widget suite: passed on {}", std::env::consts::OS);
	Ok(())
}
