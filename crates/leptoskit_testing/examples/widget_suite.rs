//! The same widget journey, compiled and run on each target OS.
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;

fn main() -> Result<(), TestError> {
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
	println!("widget suite: passed on {}", std::env::consts::OS);
	Ok(())
}
