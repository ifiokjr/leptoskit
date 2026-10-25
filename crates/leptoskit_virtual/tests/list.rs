//! The rendered window over the real headless renderer: only the intersecting
//! items exist in the tree, and scrolling swaps them.
//!
//! Frames come from explicit Tailwind bounds, so equality asserts them
//! directly.
#![allow(clippy::float_cmp)]

use std::sync::Arc;

use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

fn rendered_items(tester: &WidgetTester) -> Vec<String> {
	tester
		.tree()
		.lines()
		.filter(|line| line.contains("Contact "))
		.filter_map(|line| line.split("Some(\"").nth(1))
		.map(|rest| rest.split('"').next().unwrap_or_default().to_owned())
		.collect()
}

#[test]
fn a_scroll_container_renders_only_the_window() -> Result<(), leptoskit_testing::TestError> {
	const VIEWPORT: f64 = 644.0;
	let offset = Arc::new(std::sync::RwLock::new(0.0_f64));

	let mut tester = WidgetTester::new(
		{
			let offset = Arc::clone(&offset);
			move || {
				let scroll = *offset.read().expect("uncontended");
				let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 56.0));
				virtualizer.set_viewport(VIEWPORT);
				virtualizer.scroll_to(scroll);
				let items = virtualizer.items().expect("a fixed estimate is valid");
				let total = virtualizer.total_size().expect("a fixed estimate is valid");

				VirtualList(
					&items,
					total,
					false,
					{
						let offset = Arc::clone(&offset);
						move |next| {
							*offset.write().expect("uncontended") = next;
						}
					},
					|item| View().child(Text(format!("Contact {}", item.index))),
				)
				.aria_label("Contacts")
				.class("w-[390px] h-[644px]")
			}
		},
		390.0,
		844.0,
	)?;

	// The mount renders the first window, not ten thousand rows.
	let first = rendered_items(&tester);
	eprintln!(
		"TREE
{}",
		tester.tree()
	);
	assert_eq!(first.len(), 12);
	assert_eq!(first.first().map(String::as_str), Some("Contact 0"));
	assert_eq!(first.last().map(String::as_str), Some("Contact 11"));

	// Scrolling deep into the list swaps the window wholesale.
	let list = tester.find(&Finder::label("Contacts"))?;
	tester.scroll(&Finder::label("Contacts"), 0.0, 280_000.0)?;

	let deep = rendered_items(&tester);
	assert_eq!(deep.len(), 12);
	assert_eq!(deep.first().map(String::as_str), Some("Contact 5000"));
	assert_eq!(deep.last().map(String::as_str), Some("Contact 5011"));

	// The list itself stays one viewport-sized committed container.
	assert_eq!(list.frame.height, 644.0);
	assert_eq!(list.frame.width, 390.0);
	Ok(())
}
