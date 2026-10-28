---
"release": minor
---

# Add leptoskit_virtual: headless list virtualization

New `leptoskit_virtual` crate porting TanStack Virtual's model to the shared element tree: a `Virtualizer` with fixed or per-index size estimates, measured sizes that shift everything after them (and re-resolve only those items), overscan, and clamped aligned scroll targets (`Align::Auto/Start/Center/End`). `VirtualList(&virtualizer, Axis::Vertical, on_scroll, item)` renders the window inside a `ScrollView` sized to the real total: each item sits in a `listitem` slot sized to it along the axis and stretched across it, carrying `aria-posinset` and `aria-setsize` for its place in the whole list. Invalid estimates and measurements are errors naming the index — including from `offset_for_index`, through `OffsetError` — never silently clamped offsets, and an offset past the end of a list that shrank resolves to its last window. Re-exported behind the `leptoskit` `virtual` feature. The generated macOS, iOS, and Android apps do not report scroll events yet, so there the window stays on its first page.
