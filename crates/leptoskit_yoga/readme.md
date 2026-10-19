# leptoskit_yoga

> A from-scratch flexbox and CSS grid layout engine for the leptoskit native renderer.

A React Native-style layout engine, fully rewritten in Rust with no C dependencies. One `Style` struct drives both flexbox (the React Native layout system) and CSS grid, so a stylesheet maps onto any node without a second system to learn.

## Why another yoga

- **One arena, zero pointers.** A whole interface lives in a single `Vec`; there is no per-node allocation, no `Rc`, and no locking. Relayouts only touch dirty subtrees, and text measurement results are cached per node.
- **Grid is first-class.** Track sizing (`auto`, lengths, percentages, `fr`, `minmax`), line-based placement with spans, auto-placement in row, column, sparse, and dense modes, gaps, and area alignment.
- **Modern CSS where it matters.** `gap`, `aspect_ratio`, `min`/`max` constraints, `box_sizing`, `display: contents`, RTL writing directions, and auto margins that absorb free space.
- **No platform lock-in.** The crate has no dependencies by default; backends for any platform consume the plain `Layout` output.

## Usage

```no_run
use leptoskit_yoga::{AvailableSpace, Dimension, Size, Style, Yoga};

let mut tree = Yoga::new();
let root = tree.create_node();
tree.set_style(root, Style {
    width: Dimension::Points(320.0),
    height: Dimension::Points(240.0),
    ..Style::default()
});
let child = tree.create_node();
tree.set_style(child, Style {
    flex_grow: 1.0,
    ..Style::default()
});
tree.insert_child(root, child, 0);
tree.calculate_layout(root, Size {
    width: AvailableSpace::Definite(320.0),
    height: AvailableSpace::Definite(240.0),
});
assert_eq!(tree.layout(child).height, 240.0);
```

## Documented divergences from CSS

- Defaults follow React Native: `flex_direction: column`, `align_items:
  stretch`, `flex_shrink: 0`.
- Baseline alignment falls back to flex-start.
- `position: fixed` resolves against the root's padding box rather than the nearest scroll container.
- Grid auto-track stretching applies on the block axis; the inline axis packs at the start unless tracks use `fr`.
