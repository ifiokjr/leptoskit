---
"release": minor
---

# Add leptoskit_virtual: headless list virtualization

New `leptoskit_virtual` crate porting TanStack Virtual's model to the shared element tree: a `Virtualizer` with fixed or per-index size estimates, measured sizes that shift everything after them, overscan, and clamped aligned scroll targets (`Align::Auto/Start/Center/End`), plus a `VirtualList` scroll container that sizes content to the real total and places each windowed item at its offset. Invalid estimates and measurements are errors naming the index, never silently clamped offsets. Ten unit tests over the windowing math, an integration test mounting ten thousand items over the real headless renderer, and a new virtualized-lists guide. Re-exported behind the `leptoskit` `virtual` feature.
