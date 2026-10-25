//! The scroll container and content window around a virtualizer's items.

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::View;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;

use crate::VirtualItem;

/// A scroll container that renders one virtualized window of items.
///
/// The list is the renderer side of a [`Virtualizer`](crate::Virtualizer):
/// a [`ScrollView`](leptoskit_native::components::ScrollView) whose content
/// is sized to the virtualizer's total, with each returned item placed at its
/// measured offset. Call it with the items and total from the current state;
/// `on_scroll` receives the container's offset along the axis whenever it
/// scrolls, which the screen feeds back into the virtualizer before
/// rebuilding with the new window.
///
/// Items arrive positionally: give each one its own stable identity in the
/// item builder, because a window can start anywhere. Style the list with
/// Tailwind classes like any component.
///
/// ```
/// use leptoskit_native::components::Text;
/// use leptoskit_native::components::View;
/// use leptoskit_virtual::VirtualItem;
/// use leptoskit_virtual::VirtualList;
/// use leptoskit_virtual::VirtualOptions;
/// use leptoskit_virtual::Virtualizer;
///
/// let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(100, 40.0));
/// virtualizer.set_viewport(200.0);
/// let items = virtualizer.items().expect("a fixed estimate is valid");
/// let total = virtualizer.total_size().expect("a fixed estimate is valid");
///
/// let list = VirtualList(
/// 	&items,
/// 	total,
/// 	false,
/// 	|_| (),
/// 	|item| View().child(Text(format!("Row {}", item.index))),
/// );
/// assert_eq!(list.kind, leptoskit_native::NativeKind::ScrollView);
/// assert_eq!(list.children.len(), 1);
/// assert_eq!(list.children[0].children.len(), items.len());
/// ```
pub fn VirtualList(
	items: &[VirtualItem],
	total_size: f64,
	horizontal: bool,
	on_scroll: impl Fn(f64) + Send + Sync + 'static,
	item: impl Fn(&VirtualItem) -> Element,
) -> Element {
	let mut content = content_sized(total_size, horizontal);

	for virtual_item in items {
		let placed = item(virtual_item).with_style(|style| {
			style.position = PositionType::Absolute;
			style.inset = if horizontal {
				Rect::all_edges(
					Dimension::Auto,
					Dimension::Auto,
					Dimension::Points(0.0),
					Dimension::Points(virtual_item.start as f32),
				)
			} else {
				Rect::all_edges(
					Dimension::Points(virtual_item.start as f32),
					Dimension::Points(0.0),
					Dimension::Auto,
					Dimension::Points(0.0),
				)
			};
		});
		content = content.child(placed);
	}

	ScrollView()
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
				on_scroll(if horizontal {
					f64::from(*offset_x)
				} else {
					f64::from(*offset_y)
				});
			}
			EventOutcome::Handled
		})
		.child(content)
}

/// The content view that owns the total size along the scrolling axis.
fn content_sized(total_size: f64, horizontal: bool) -> Element {
	View().with_style(|style| {
		style.flex_direction = if horizontal {
			FlexDirection::Row
		} else {
			FlexDirection::Column
		};
		if horizontal {
			style.width = Dimension::Points(total_size as f32);
		} else {
			style.height = Dimension::Points(total_size as f32);
		}
	})
}
