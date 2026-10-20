<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_yoga

> A from-scratch flexbox and CSS grid layout engine for the leptoskit native renderer.

A React Native-style layout engine, fully rewritten in Rust with no C dependencies. One `Style` struct drives both flexbox (the React Native layout system) and CSS grid, so a stylesheet maps onto any node without a second system to learn.

## Why another yoga

- **One arena, zero pointers.** A whole interface lives in a single `Vec`; nodes reference each other by index, with no `Rc` and no locking. Every node caches its results by input constraints, so a relayout re-runs only the dirty path (a changed node and its ancestors) and clean nodes whose constraints changed; clean subtrees laid out under the same constraints are reused as they are.
- **Grid is first-class.** Track sizing (`auto`, lengths, percentages, `fr`, `minmax`, including Tailwind's `minmax(0, 1fr)`), line-based placement with spans, auto-placement in row, column, sparse, and dense modes, gaps, and area alignment.
- **One box model.** Content-box and border-box sizing apply consistently to dimensions, min/max constraints, flex basis, and aspect ratios. Parent-resolved flex/grid sizes remain border-box sizes, so padding and borders are never counted twice.
- **Modern CSS where it matters.** `gap`, `aspect_ratio`, `min`/`max` constraints, `display: contents`, RTL row directions, signed margins and insets, relative positioning in flex and grid, and main-axis auto margins that absorb free space.
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
- `position: fixed` is laid out exactly like `position: absolute`, against the parent's padding box rather than the viewport.
- Grid auto-track stretching applies on the block axis; the inline axis packs at the start unless tracks use `fr`.
- Layout supports flex and grid containers; block/inline flow, tables, floats, multicolumn layout, and writing modes beyond horizontal rows are not implemented.
- Intrinsic sizing keywords, named grid lines/areas, subgrid, sticky positioning, CSS `order`, and cross-axis auto margins are not implemented.
- Overflow is computed for layout extents; clipping and scrolling require a platform backend to consume that behavior.
- With `grid_auto_flow: column` and no rows at all, auto-placed items stack down the first column instead of filling a single implicit row.
- Grid line numbers and spans are clamped to 10 000, bounding the implicit grid.
