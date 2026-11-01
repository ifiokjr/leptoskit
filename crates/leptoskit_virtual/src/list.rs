//! The scroll container and content window around a virtualizer's items.

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::Role;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::View;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;

use crate::InvalidEstimate;
use crate::VirtualItem;
use crate::Virtualizer;

/// The axis a [`VirtualList`] scrolls along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
	/// Items stack top to bottom and the list scrolls vertically.
	#[default]
	Vertical,
	/// Items run start to end and the list scrolls horizontally.
	Horizontal,
}

/// A scroll container that renders one virtualized window of items.
///
/// The list is the renderer side of a [`Virtualizer`]: a
/// [`ScrollView`](leptoskit_native::components::ScrollView) whose content is
/// sized to the virtualizer's total, with each item of the current window
/// placed at its offset and sized to its size along the axis. Build it from
/// the current state; `on_scroll` receives the container's offset along the
/// axis whenever it scrolls, which the screen feeds back into the virtualizer
/// before rebuilding with the new window.
///
/// The container is a `list` and every item a `listitem` that carries its
/// position in the whole list (`aria-posinset` and `aria-setsize`), so a
/// screen reader announces "item 4,001 of 10,000" rather than the size of
/// the window. Name the list with `aria_label` on the returned element, as
/// with any list.
///
/// Items arrive positionally: give each one its own stable identity in the
/// item builder, because a window can start anywhere. Style the list with
/// Tailwind classes like any component.
///
/// Like [`ListView`](leptoskit_native::components::ListView), the list grows
/// to fill a bounded parent, and as in React Native that growth also opens up
/// a parent column whose height comes from its content. To give the list a
/// fixed size beside other content — the size its virtualizer is told — add
/// `grow-0` with it: `.class("h-[400px] grow-0")`.
///
/// ```
/// use leptoskit_native::components::Text;
/// use leptoskit_native::components::View;
/// use leptoskit_virtual::Axis;
/// use leptoskit_virtual::VirtualList;
/// use leptoskit_virtual::VirtualOptions;
/// use leptoskit_virtual::Virtualizer;
///
/// let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(100, 40.0));
/// virtualizer.set_viewport(200.0);
///
/// let list = VirtualList(
/// 	&virtualizer,
/// 	Axis::Vertical,
/// 	|_| (),
/// 	|item| View().child(Text(format!("Row {}", item.index))),
/// )
/// .expect("a fixed estimate is valid");
/// assert_eq!(list.kind, leptoskit_native::NativeKind::ScrollView);
/// // The content holds the five rows the 200-unit viewport shows.
/// assert_eq!(list.children[0].children.len(), 5);
/// ```
#[track_caller]
pub fn VirtualList(
	virtualizer: &Virtualizer,
	axis: Axis,
	on_scroll: impl Fn(f64) + Send + Sync + 'static,
	item: impl Fn(&VirtualItem) -> Element,
) -> Result<Element, InvalidEstimate> {
	let items = virtualizer.items()?;
	let set_size = aria_count(virtualizer.count());
	let mut content = content_sized(virtualizer.total_size()?, axis);

	for virtual_item in &items {
		content = content.child(
			placed(virtual_item, axis)
				.aria_pos_in_set(aria_count(virtual_item.index + 1))
				.aria_set_size(set_size)
				.child(item(virtual_item)),
		);
	}

	let list = ScrollView()
		.role(Role::List)
		.with_style(|style| {
			// Fill a bounded parent, like the ListView helper: a virtual list
			// that shrinks to its content would render every item at once.
			style.flex_grow = 1.0;
			style.flex_shrink = 1.0;
			style.min_width = Dimension::Points(0.0);
			style.min_height = Dimension::Points(0.0);
		})
		.on(NativeEventSlot::Scroll, move |event| {
			if let NativeEvent::Scroll { offset_x, offset_y } = event {
				on_scroll(f64::from(match axis {
					Axis::Vertical => *offset_y,
					Axis::Horizontal => *offset_x,
				}));
			}

			EventOutcome::Handled
		})
		.child(content);
	Ok(list)
}

/// The content view, which owns the total size along the axis and fills the
/// container across it: its items are all absolutely placed, so nothing else
/// would give it a cross-axis size. It is plain layout, so the items read as
/// the list's own.
fn content_sized(total_size: f64, axis: Axis) -> Element {
	let total = Dimension::Points(total_size as f32);
	let fill = Dimension::Percent(1.0);

	View().with_style(|style| {
		match axis {
			Axis::Vertical => {
				style.flex_direction = FlexDirection::Column;
				style.height = total;
				style.width = fill;
			}
			Axis::Horizontal => {
				style.flex_direction = FlexDirection::Row;
				style.width = total;
				style.height = fill;
			}
		}
	})
}

/// The `listitem` that holds one item at its offset: sized to the item along
/// the axis and stretched across it, so the window math and the layout agree.
fn placed(virtual_item: &VirtualItem, axis: Axis) -> Element {
	let start = Dimension::Points(virtual_item.start as f32);
	let size = Dimension::Points(virtual_item.size as f32);
	let edge = Dimension::Points(0.0);

	View().role(Role::ListItem).with_style(|style| {
		style.position = PositionType::Absolute;
		match axis {
			Axis::Vertical => {
				style.inset = Rect::all_edges(start, edge, Dimension::Auto, edge);
				style.height = size;
			}
			Axis::Horizontal => {
				style.inset = Rect::all_edges(edge, Dimension::Auto, edge, start);
				style.width = size;
			}
		}
	})
}

/// A count as ARIA carries it, saturating past `u32::MAX` items.
fn aria_count(count: usize) -> u32 {
	u32::try_from(count).unwrap_or(u32::MAX)
}
