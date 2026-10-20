//! The same widget journey, compiled and run on each target OS.
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

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
	// it must produce the same kind of image on every target OS.
	let layout = tester.layout_image()?;
	let mut reader = png::Decoder::new(std::io::Cursor::new(layout.png())).read_info()?;
	let (width, height) = (reader.info().width, reader.info().height);
	let mut pixels = vec![0; reader.output_buffer_size()];
	let frame = reader.next_frame(&mut pixels)?;
	let colors: std::collections::HashSet<&[u8]> = pixels[..frame.buffer_size()]
		.chunks_exact(frame.color_type.samples())
		.collect();
	// Background, outlines, and at least one widget fill; at most the five
	// palette fills besides those. Anything else is not flat colour.
	if (width, height) != (390, 844) || !(3..=7).contains(&colors.len()) {
		return Err(format!(
			"layout image is {width}x{height} with {} colours, expected flat 390x844",
			colors.len()
		)
		.into());
	}
	println!("widget suite: passed on {}", std::env::consts::OS);
	Ok(())
}
