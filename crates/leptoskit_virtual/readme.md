# leptoskit_virtual

Headless list virtualization for the leptoskit element tree, ported from [TanStack Virtual](https://tanstack.com/virtual): a `Virtualizer` computes exactly the items a scrolled viewport intersects — never the whole list — and `VirtualList` renders that window inside a `ScrollView` sized to the real total.

```rust
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 56.0));
virtualizer.set_viewport(644.0);
virtualizer.scroll_to(0.0);
let items = virtualizer.items().expect("a fixed estimate is valid");
let total = virtualizer.total_size().expect("a fixed estimate is valid");

let list = VirtualList(&items, total, false, |_| (), |item| {
	View().child(Text(format!("Contact {}", item.index)))
});
```

Sizes start from an estimate — one fixed number, or a function of the index — and resolve to real sizes through `measure`, which shifts every item after it. Overscan widens the window cheaply, `offset_for_index` computes aligned scroll targets, and an estimate that returns anything but a finite positive size is an error naming the index, never a silently clamped offset.

The core is axis-agnostic and pure math, so it runs anywhere the shared element tree does: the web renderer and every native backend. Use the crate directly or enable `leptoskit`'s `virtual` feature and import `leptoskit::virtual_list::*`.
