---
"release": patch
---

# Wrap flex lines without a gap before their first item

A wrapping flex container counted the gap before an item that moved to a new line as part of that line, so lines filled up one gap early and could wrap an item sooner than CSS does. A line now holds only the gaps between its own items.
