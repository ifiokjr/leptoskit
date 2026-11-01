//! The rendered window over the real headless renderer: only the intersecting
//! items exist in the tree, scrolling swaps them, and each item is placed
//! and sized where the window math says it is.
//!
//! Frames come from explicit Tailwind bounds and dyadic sizes, so equality
//! asserts them directly.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::RwLock;

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_testing::Finder;
use leptoskit_testing::TestError;
use leptoskit_testing::WidgetTester;
use leptoskit_virtual::Axis;
use leptoskit_virtual::Estimate;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

/// A list screen whose scroll offset lives in shared state, the way a screen
/// owns the loop: the list reports offsets, the next build reads them.
fn screen(
	options: VirtualOptions,
	viewport: f64,
	axis: Axis,
	label: &'static str,
	class: &'static str,
) -> (Arc<RwLock<f64>>, impl FnMut() -> Element + 'static) {
	let offset = Arc::new(RwLock::new(0.0_f64));
	let render = {
		let offset = Arc::clone(&offset);
		move || {
			let mut virtualizer = Virtualizer::new(options.clone());
			virtualizer.set_viewport(viewport);
			virtualizer.scroll_to(*offset.read().expect("uncontended"));

			VirtualList(
				&virtualizer,
				axis,
				{
					let offset = Arc::clone(&offset);
					move |next| *offset.write().expect("uncontended") = next
				},
				|item| {
					// The item fills its slot, so its frame is the slot's.
					View()
						.class("flex-1")
						.aria_label(format!("{label} {}", item.index))
						.child(Text(format!("{label} {}", item.index)))
				},
			)
			.expect("the estimate is valid")
			.aria_label(label)
			.class(class)
		}
	};
	(offset, render)
}

/// The indexes of the `label` items committed to the surface, in index
/// order.
///
/// The headless tester dispatches scroll events without moving content, so a
/// deep window is committed but off-screen; this reads the committed tree
/// rather than what is visible.
fn committed(tester: &WidgetTester, label: &str) -> Vec<usize> {
	let surface = tester.surface();
	let prefix = format!("{label} ");
	let mut pending: Vec<_> = surface.root().into_iter().collect();
	let mut indexes = Vec::new();

	while let Some(id) = pending.pop() {
		let index = surface
			.props(id)
			.text
			.as_deref()
			.and_then(|text| text.strip_prefix(&prefix))
			.and_then(|index| index.parse::<usize>().ok());
		indexes.extend(index);
		pending.extend(surface.children(id).iter().copied());
	}

	indexes.sort_unstable();
	indexes
}

#[test]
fn a_scroll_container_renders_only_the_window() -> Result<(), TestError> {
	let (_, render) = screen(
		VirtualOptions::fixed(10_000, 56.0),
		644.0,
		Axis::Vertical,
		"Contact",
		"w-[390px] h-[644px]",
	);
	let mut tester = WidgetTester::new(render, 390.0, 844.0)?;

	// The mount renders the first window, not ten thousand rows.
	assert_eq!(committed(&tester, "Contact"), (0..12).collect::<Vec<_>>());

	// Scrolling deep into the list swaps the window wholesale.
	tester.scroll(&Finder::label("Contact"), 0.0, 280_000.0)?;
	assert_eq!(
		committed(&tester, "Contact"),
		(5_000..5_012).collect::<Vec<_>>()
	);

	// The list itself stays one viewport-sized committed container.
	let list = tester.find(&Finder::label("Contact"))?;
	assert_eq!(list.frame.height, 644.0);
	assert_eq!(list.frame.width, 390.0);
	Ok(())
}

#[test]
fn vertical_items_are_sized_to_their_estimate_and_span_the_width() -> Result<(), TestError> {
	// Every third row is twice as tall, so a row's frame must come from its
	// own size, not from what its content happens to measure.
	let options = VirtualOptions {
		estimate: Estimate::PerItem(Arc::new(|index| if index % 3 == 0 { 64.0 } else { 32.0 })),
		..VirtualOptions::fixed(300, 0.0)
	};
	let (_, render) = screen(options, 256.0, Axis::Vertical, "Row", "w-[200px] h-[256px]");
	let tester = WidgetTester::new(render, 390.0, 844.0)?;

	let first = tester.find(&Finder::label("Row 0"))?;
	let second = tester.find(&Finder::label("Row 1"))?;
	let fourth = tester.find(&Finder::label("Row 3"))?;
	assert_eq!(first.frame.height, 64.0);
	assert_eq!(second.frame.height, 32.0);
	assert_eq!(second.frame.y - first.frame.y, 64.0);
	assert_eq!(fourth.frame.y - first.frame.y, 128.0);
	assert_eq!(first.frame.width, 200.0, "rows stretch across the list");

	Ok(())
}

#[test]
fn a_horizontal_list_places_cards_along_x_and_fills_the_height() -> Result<(), TestError> {
	let (_, render) = screen(
		VirtualOptions::fixed(1_000, 60.0),
		300.0,
		Axis::Horizontal,
		"Card",
		"w-[300px] h-[120px]",
	);
	let mut tester = WidgetTester::new(render, 390.0, 844.0)?;

	// The first five cards fill the 300-unit strip, each 60 wide and as tall
	// as the strip: placed at the top, not pinned to the bottom edge.
	assert_eq!(committed(&tester, "Card"), (0..5).collect::<Vec<_>>());
	let deck = tester.find(&Finder::label("Card"))?;
	let first = tester.find(&Finder::label("Card 0"))?;
	let second = tester.find(&Finder::label("Card 1"))?;
	assert_eq!(first.frame.width, 60.0);
	assert_eq!(first.frame.height, 120.0);
	assert_eq!(first.frame.y, deck.frame.y);
	assert_eq!(second.frame.x - first.frame.x, 60.0);

	// Horizontal scrolling feeds the x offset back: the window moves.
	tester.scroll(&Finder::label("Card"), 6_000.0, 0.0)?;
	assert_eq!(committed(&tester, "Card"), (100..105).collect::<Vec<_>>());
	Ok(())
}

#[test]
fn a_list_that_shrank_under_its_offset_still_shows_its_end() -> Result<(), TestError> {
	let count = Arc::new(RwLock::new(10_000_usize));
	let offset = Arc::new(RwLock::new(0.0_f64));
	let render = {
		let count = Arc::clone(&count);
		let offset = Arc::clone(&offset);
		move || {
			let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 50.0));
			virtualizer.set_count(*count.read().expect("uncontended"));
			virtualizer.set_viewport(200.0);
			virtualizer.scroll_to(*offset.read().expect("uncontended"));
			VirtualList(
				&virtualizer,
				Axis::Vertical,
				|_| (),
				|item| Text(format!("Row {}", item.index)),
			)
			.expect("a fixed estimate is valid")
			.class("w-[200px] h-[200px]")
		}
	};
	let mut tester = WidgetTester::new(render, 390.0, 844.0)?;

	// Deep in the list, then the data shrinks to twenty rows before the
	// container reports a new offset: the last viewport still renders.
	*offset.write().expect("uncontended") = 400_000.0;
	*count.write().expect("uncontended") = 20;
	tester.pump();
	assert_eq!(committed(&tester, "Row"), (16..20).collect::<Vec<_>>());
	Ok(())
}

#[test]
fn every_item_is_a_listitem_that_knows_its_place_in_the_whole_list() {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 50.0));
	virtualizer.set_viewport(100.0);
	virtualizer.scroll_to(200_000.0);

	let list = VirtualList(
		&virtualizer,
		Axis::Vertical,
		|_| (),
		|item| View().child(Text(format!("Row {}", item.index))),
	)
	.expect("a fixed estimate is valid");

	assert_eq!(list.props.aria().role, Some(Role::List));
	let content = &list.children[0];
	assert_eq!(
		content.props.aria().role,
		None,
		"the content is plain layout"
	);
	let positions: Vec<_> = content
		.children
		.iter()
		.map(|item| {
			let aria = item.props.aria();
			(aria.role, aria.pos_in_set, aria.set_size)
		})
		.collect();
	assert_eq!(
		positions,
		[
			(Some(Role::ListItem), Some(4_001), Some(10_000)),
			(Some(Role::ListItem), Some(4_002), Some(10_000)),
		]
	);
}

#[test]
fn an_empty_list_renders_an_empty_list() {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(0, 50.0));
	virtualizer.set_viewport(400.0);
	let list = VirtualList(&virtualizer, Axis::Vertical, |_| (), |_| View())
		.expect("a fixed estimate is valid");
	assert!(list.children[0].children.is_empty());
}

#[test]
fn an_invalid_estimate_fails_the_list_instead_of_rendering_a_gap() {
	let options = VirtualOptions {
		estimate: Estimate::PerItem(Arc::new(|index| if index == 2 { -1.0 } else { 40.0 })),
		..VirtualOptions::fixed(10, 0.0)
	};
	let mut virtualizer = Virtualizer::new(options);
	virtualizer.set_viewport(400.0);
	let error = VirtualList(&virtualizer, Axis::Vertical, |_| (), |_| View()).unwrap_err();
	assert_eq!(error.index, 2);
}

#[test]
fn a_named_list_keeps_its_items_reachable() -> Result<(), TestError> {
	// Naming the list must not fold its items into one element, the way a
	// named plain container is.
	let (_, render) = screen(
		VirtualOptions::fixed(100, 40.0),
		200.0,
		Axis::Vertical,
		"Contact",
		"w-[200px] h-[200px]",
	);
	let tester = WidgetTester::new(render, 390.0, 844.0)?;
	tester.find(&Finder::aria_role(Role::List, "Contact"))?;
	let semantics = tester.semantics();
	assert!(semantics.contains("list \"Contact\""), "{semantics}");
	assert!(semantics.contains("listitem"), "{semantics}");
	assert!(semantics.contains("\"Contact 0\""), "{semantics}");
	Ok(())
}
