---
"release": patch
---

# Lay out only what a change can affect

Native layout now stops at a relayout boundary, the way Flutter does. A boundary is an element whose parent sets both its width and its height without measuring its content, like a `flex-1` cell in a fixed-height row. When something inside a boundary changes, only that element's subtree is laid out again, not its siblings or ancestors.

A grid of cells whose labels change on every frame now costs the changed cells, not the whole grid. Two further changes make that possible and speed up every layout:

- A single-line container with a fixed cross size no longer measures the content of children it stretches, because the CSS algorithm never uses that measurement.
- Layout no longer copies each element's style on every pass.

Layouts are unchanged. A differential test compares incremental layout with a fresh layout across 2,000 random trees.
