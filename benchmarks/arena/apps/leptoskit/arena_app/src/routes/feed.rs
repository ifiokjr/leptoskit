//! The `autoscroll` and `busy` feed: 2,000 cards in a virtual list.

use leptoskit::prelude::Color;
use leptoskit::prelude::Dimension;
use leptoskit::prelude::Rect;
use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit_virtual::Axis;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

/// Items in the feed.
pub const ITEMS: usize = 2_000;
/// Height of one card, in layout units.
pub const ITEM_HEIGHT: f64 = 72.0;
/// Side of an item's colour square, in layout units.
const SQUARE: f32 = 40.0;
/// Height of the arena's header row, in layout units.
pub const HEADER_HEIGHT: f64 = 48.0;

/// The list scrolled to `offset`, reporting every scroll the platform makes.
///
/// The window is rebuilt from `offset`, so the offset the app asked for and
/// the offset the platform reports are the same number: `on_scroll` stores
/// what the user scrolled to, and the next frame lays the window out there.
pub fn feed(
	offset: f64,
	viewport: f64,
	on_scroll: impl Fn(f64) + Send + Sync + 'static,
) -> Element {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(ITEMS, ITEM_HEIGHT));
	virtualizer.set_viewport(viewport);
	virtualizer.scroll_to(offset);

	VirtualList(&virtualizer, Axis::Vertical, on_scroll, |item| {
		let index = item.index;

		View()
			.with_style(|style| {
				style.width = Dimension::Percent(1.0);
				style.height = Dimension::Percent(1.0);
				style.flex_direction = leptoskit::prelude::FlexDirection::Row;
				style.padding = Rect {
					top: Dimension::Points(8.0),
					right: Dimension::Points(16.0),
					bottom: Dimension::Points(8.0),
					left: Dimension::Points(16.0),
				};
				style.gap = Rect {
					right: Dimension::Points(12.0),
					..Rect::default()
				};
			})
			.children([
				View()
					.with_style(|style| {
						style.width = Dimension::Points(SQUARE);
						style.height = Dimension::Points(SQUARE);
					})
					.background(hsl(index)),
				View().class("flex-1").children([
					Text(format!("Item {index}")),
					Text(format!(
						"This is the body of item {index}. This is the body of item {index}."
					))
					.class("text-sm"),
				]),
			])
	})
	.expect("a fixed estimate is a valid one")
}

/// The feed with its scroll command: snapped, or animated by the platform.
pub fn scrolled(
	offset: f64,
	viewport: f64,
	animated: bool,
	report: impl Fn(f64) + Send + Sync + 'static,
) -> Element {
	let list = feed(offset, viewport, report);

	if animated {
		list.scroll_animated(0.0, offset as f32)
	} else {
		list.scroll(0.0, offset as f32)
	}
}

/// The offset that puts item `index` at the top of the list.
pub fn item_offset(index: usize) -> f64 {
	index as f64 * ITEM_HEIGHT
}

/// `hsl(i * 37 % 360, 60%, 50%)`, converted the way CSS does.
fn hsl(index: usize) -> Color {
	let hue = (index as f64 * 37.0) % 360.0;
	let saturation = 0.6_f64;
	let lightness = 0.5_f64;
	let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
	let sector = hue / 60.0;
	let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
	let (red, green, blue) = match sector as u32 {
		0 => (chroma, second, 0.0),
		1 => (second, chroma, 0.0),
		2 => (0.0, chroma, second),
		3 => (0.0, second, chroma),
		4 => (second, 0.0, chroma),
		_ => (chroma, 0.0, second),
	};
	let offset = lightness - chroma / 2.0;
	let channel = |value: f64| ((value + offset) * 255.0).round().clamp(0.0, 255.0) as u8;

	Color::rgba(channel(red), channel(green), channel(blue), 255)
}
