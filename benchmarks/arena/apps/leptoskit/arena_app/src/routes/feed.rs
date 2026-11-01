//! The `autoscroll` and `busy` feed: 2,000 cards in leptoskit's virtual list.

use leptoskit::prelude::Color;
use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit::prelude::component;
use leptoskit::virtual_list::Axis;
use leptoskit::virtual_list::VirtualItem;
use leptoskit::virtual_list::VirtualList;
use leptoskit::virtual_list::VirtualOptions;
use leptoskit::virtual_list::Virtualizer;

use crate::services::Arena;
use crate::services::Feed;
use crate::services::Screen;

/// Items in the feed.
pub const LENGTH: usize = 2_000;
/// Each item's height, in points.
pub const ITEM_HEIGHT: f64 = 72.0;
/// The height the window covers: taller than the list on any phone the
/// arena runs on, so the window always fills the screen.
const VIEWPORT_HEIGHT: f64 = 1_000.0;
/// Items built past each edge of the viewport, about FlashList's default
/// 250 pt draw distance.
const OVERSCAN: usize = 4;

/// The feed's window at the current offset, scrolled to the scenario's
/// target when it sets one.
#[component]
pub fn FeedList(arena: &Arena, feed: &Feed) -> Element {
	let mut virtualizer =
		Virtualizer::new(VirtualOptions::fixed(LENGTH, ITEM_HEIGHT).with_overscan(OVERSCAN));
	virtualizer.set_viewport(VIEWPORT_HEIGHT);
	virtualizer.scroll_to(feed.offset);
	let arena = arena.clone();
	let list = VirtualList(
		&virtualizer,
		Axis::Vertical,
		move |offset| scrolled(&arena, offset),
		item,
	)
	.expect("a fixed estimate is valid")
	.aria_label("Feed");

	match feed.target {
		Some(target) => list.scroll_offset(target),
		None => list,
	}
}

/// Records where the list scrolled. The window is rebuilt only when the
/// offset moved from the one it was built around, so the report of an
/// offset the scenario set itself costs no render.
fn scrolled(arena: &Arena, offset: f64) {
	let moved = {
		let mut model = arena.read();
		let Screen::Feed(feed) = &mut model.screen else {
			return;
		};
		feed.reported = offset;
		(feed.offset - offset).abs() >= 0.5
	};

	if moved {
		arena.update(|model| {
			if let Screen::Feed(feed) = &mut model.screen {
				feed.offset = offset;
			}
		});
	}
}

/// Item `i`: a 40 pt square in `hsl(i * 37 % 360, 60%, 50%)`, its title, and
/// its body twice.
fn item(item: &VirtualItem) -> Element {
	let index = item.index;
	let body = format!("This is the body of item {index}.");

	View()
		.class("h-[72px] flex-row items-center px-4 overflow-hidden")
		.children([
			View()
				.class("w-10 h-10 mr-3 shrink-0")
				.background(swatch(index)),
			View().class("flex-1").children([
				Text(format!("Item {index}")).class("text-[15px] font-semibold"),
				Text(format!("{body} {body}")).class("text-xs"),
			]),
		])
}

/// `hsl(index * 37 % 360, 60%, 50%)` in RGB.
fn swatch(index: usize) -> Color {
	let hue = (index * 37 % 360) as f64 / 60.0;
	let (lightness, saturation) = (0.5, 0.6);
	let chroma = (1.0 - (2.0 * lightness - 1.0_f64).abs()) * saturation;
	let second = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
	let (red, green, blue) = match hue as u32 {
		0 => (chroma, second, 0.0),
		1 => (second, chroma, 0.0),
		2 => (0.0, chroma, second),
		3 => (0.0, second, chroma),
		4 => (second, 0.0, chroma),
		_ => (chroma, 0.0, second),
	};
	let base = lightness - chroma / 2.0;
	let channel = |value: f64| ((value + base) * 255.0).round() as u8;

	Color::rgba(channel(red), channel(green), channel(blue), 255)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn swatches_match_css_hsl() {
		// hsl(0, 60%, 50%), hsl(37, 60%, 50%), and hsl(222, 60%, 50%).
		assert_eq!(swatch(0), Color::rgba(204, 51, 51, 255));
		assert_eq!(swatch(1), Color::rgba(204, 145, 51, 255));
		assert_eq!(swatch(6), Color::rgba(51, 97, 204, 255));
	}
}
