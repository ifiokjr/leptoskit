# Virtualized lists

A thousand rows of real native widgets is a heavy screen, and ten thousand does not commit. `leptoskit_virtual` is list virtualization for the shared element tree, ported from [TanStack Virtual](https://tanstack.com/virtual): a `Virtualizer` computes exactly the items a scrolled viewport intersects — never the whole list — and `VirtualList` renders that window inside a `ScrollView` sized to the real total.

```toml
[dependencies]
leptoskit_virtual = "0.1"
```

Or enable `leptoskit`'s `virtual` feature and import `leptoskit::virtual_list::*`.

## The loop

The virtualizer is pure windowing math; the screen owns the loop around it:

1. feed it the viewport size and the scroll offset the surface reports, with `set_viewport` and `scroll_to`;
2. read `items` and `total_size`, and render them with `VirtualList`;
3. when the renderer measures a real item size, hand it back with `measure`.

```rust,ignore
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

fn contacts(offset: f64, virtualizer: &mut Virtualizer) -> leptoskit_native::Element {
    virtualizer.set_viewport(644.0);
    virtualizer.scroll_to(offset);
    let items = virtualizer.items().expect("a fixed estimate is valid");
    let total = virtualizer.total_size().expect("a fixed estimate is valid");

    VirtualList(&items, total, false, |_| (), |item| {
        View().child(Text(format!("Contact {}", item.index)))
    })
}
```

`on_scroll` — the fourth argument — receives the container's offset along the axis whenever it scrolls; write it to state, rebuild, and the next window renders. Items arrive positionally, so give each one a stable identity in the item builder: a window can start anywhere.

## Sizes

Sizes start from an [`Estimate`][readme-estimate]: one fixed number for uniform rows, or a function of the index for mixed content. The window resolves against estimates until a real size arrives through `measure`, and a changed measurement shifts every item after it, so estimated windows settle as the renderer measures. An estimate that returns anything but a finite positive size is an error naming the index — never a silently clamped offset.

Overscan widens the window on both sides cheaply (`with_overscan`), absorbing scroll velocity at the cost of a few extra items.

## Programmatic scrolling

`offset_for_index` computes where to scroll for an item: `Align::Auto` moves the least distance that shows it, `Start`, `Center`, and `End` place it deliberately, and every target is clamped to the scrollable range. The surface still owns the actual scrolling: hand the returned offset to the scroll container.

## On every platform

The core is axis-agnostic arithmetic and the container is the shared `ScrollView`, so one virtualized list runs on the web renderer and every native backend. Horizontal lists pass `true` for the axis; the offsets and content size apply to whichever axis the container scrolls.

## Testing

- The window math is pure: `items()` returns the intersecting indexes and the integration test mounts a ten-thousand-item list over the real headless renderer, asserting the mount commits twelve items and a deep scroll swaps them wholesale.
- `WidgetTester::scroll` drives the scroll event through the real dispatch path, so the on-scroll feedback loop is exercised, not mocked.

[readme-estimate]: https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_virtual#sizes
