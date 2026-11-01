# Virtualized lists

A thousand rows of real native widgets is a heavy screen, and ten thousand does not commit. `leptoskit_virtual` is list virtualization for the shared element tree, ported from [TanStack Virtual](https://tanstack.com/virtual): a `Virtualizer` computes exactly the items a scrolled viewport intersects — never the whole list — and `VirtualList` renders that window inside a `ScrollView` sized to the real total.

```toml
[dependencies]
leptoskit_virtual = "0.1"
```

Or enable `leptoskit`'s `virtual` feature and import `leptoskit::virtual_list::*`.

[`examples/observatory`](https://github.com/ifiokjr/leptoskit/tree/main/examples/observatory) is a complete app built this way: a year of sky observations under a horizontal ribbon of nights.

## The loop

The virtualizer is pure windowing math; the screen owns the loop around it:

1. feed it the viewport size and the scroll offset the surface reports, with `set_viewport` and `scroll_to`;
2. render it with `VirtualList`, which builds only the items in the window, inside a container sized to the whole list;
3. when the renderer measures a real item size, hand it back with `measure`.

```rust,ignore
use leptoskit::component;
use leptoskit::view;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_virtual::Axis;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

#[component]
fn Contacts(
	offset: f64,
	on_scroll: impl Fn(f64) + Send + Sync + 'static,
) -> leptoskit_native::Element {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 56.0));
	virtualizer.set_viewport(644.0);
	virtualizer.scroll_to(offset);

	VirtualList(&virtualizer, Axis::Vertical, on_scroll, |item| {
		view! {
			<View>
				<Text>{format!("Contact {}", item.index)}</Text>
			</View>
		}
	})
	.expect("a fixed estimate is valid")
}
```

`on_scroll` receives the container's offset along the axis whenever it scrolls; write it to state, rebuild, and the next window renders. Items arrive positionally, so give each one a stable identity in the item builder: a window can start anywhere.

The list grows to fill a bounded parent, like `ListView`. Beside other content, give it the size you told the virtualizer and `grow-0`, so it keeps that size instead of opening up its column: `.class("h-[400px] grow-0")`.

Each item sits in a slot sized to its size along the axis and stretched across it, so what the math expects is what the layout draws. The list is a `list` — name it with `aria_label` — and each slot a `listitem` that carries its place in the whole list, so a screen reader announces "4,001 of 10,000" rather than the size of the window. To lay the items out yourself, read `items` and `total_size` and build your own container.

## Sizes

Sizes start from an `Estimate`: one fixed number for uniform rows, or a function of the index for mixed content. The window resolves against estimates until a real size arrives through `measure`, and a changed measurement shifts every item after it, so estimated windows settle as the renderer measures. Only the items after a change resolve again, so measuring near the end of a long list stays cheap. An estimate that returns anything but a finite positive size is an error naming the index — never a silently clamped offset.

Overscan widens the window on both sides cheaply (`with_overscan`), absorbing scroll velocity at the cost of a few extra items.

## Programmatic scrolling

A "jump to today" button, a chat that follows its newest message, or a feed that scrolls itself all need to move the list from code. `offset_for_index` computes where to scroll for an item: `Align::Auto` moves the least distance that shows it, `Start`, `Center`, and `End` place it deliberately, and every target is clamped to the scrollable range. An index past the end and an invalid estimate are both errors, since neither has an offset.

Hand the target to the list with `scroll_offset`, and build the window around it in the same render, so the items are there when the list arrives:

```rust,ignore
use leptoskit_native::ScrollOffset;
use leptoskit_virtual::Align;

fn jump_to(state: &mut Contacts, index: usize) {
	let target = state
		.virtualizer
		.offset_for_index(index, Align::Start)
		.expect("the index is in the list");
	state.virtualizer.scroll_to(target);
	state.target = Some(ScrollOffset::new(0.0, target as f32).animated());
}

// In the screen:
let list = VirtualList(&state.virtualizer, Axis::Vertical, on_scroll, item)?;
let list = match state.target {
	Some(target) => list.scroll_offset(target),
	None => list,
};
```

`.animated()` uses the platform's own scroll animation; without it the list jumps. The list scrolls only when the target changes, so the person's own scrolling isn't undone by the next render. To return to a place you scrolled to before, set the target again from the offset `on_scroll` last reported. The list also reports where a target left it, so `on_scroll` sees programmatic scrolls too.

When the list shrinks under its scroll position, the window resolves against the offset clamped to the new end, so the last items show until the container reports where it settled. The same clamp keeps an elastic overscroll at either edge from emptying the window.

## On each platform

The core is axis-agnostic arithmetic and the container is the shared `ScrollView`, so one list builds on the web renderer and every native backend. Pass `Axis::Horizontal` for a carousel; the offsets and content size apply to whichever axis the container scrolls.

The window only moves when the platform reports scrolling. The web, the apps `leptoskit new` generates, and every native backend crate report it; HarmonyOS doesn't yet, so there a `VirtualList` stays on its first window.

`scroll_offset` moves the list on the web and in the generated macOS, iOS, and Android apps. The native backend crates, and so the generated Windows and Linux apps, HarmonyOS, and the terminal don't apply it yet: there the list stays where the person scrolled it.
