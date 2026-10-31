---
"release": minor
---

# Expose viewport highlight rectangles for DevTools

`NativeSurface::highlight_rects` returns each committed node's viewport-space frame when the `devtools` feature is on, for the host agent to highlight a node.
