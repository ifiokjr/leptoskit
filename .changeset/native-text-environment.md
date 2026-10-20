---
"release": minor
---

# Refresh native text metrics and carry writing direction

Add `NativeSurface::refresh_text_measurer()` so hosts can replace immutable font metrics after DPI or text-scale changes without recreating controls. Pass each node's layout direction through native props and text measurement, and resolve AppKit leading/trailing alignment against that direction.

`TextStyle` includes a `direction` field. Use `..TextStyle::default()` when constructing a style that should keep the default left-to-right direction.
