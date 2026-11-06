---
"release": patch
---

# Fix a rounded-view tap crash; speed up list updates

Tapping a fully rounded view that clips its content, such as a `rounded-full overflow-hidden` avatar, could crash the app when the view sat at a fractional position. Hit testing on every platform, and the macOS `Pressable`, now handles that corner.

Lists and conditional views update faster in native apps:

- A `For` or `Show` that changes now rebuilds only its own children. It used to copy every node on the screen first, so toggling a `Show` beside 1,000 cards took 3 ms and now takes 4 µs.
- Removing many children at once costs their number rather than its square: clearing 1,000 cards takes about half as long.
- Two changes inside the same fixed-size cell, such as a label whose text and style both change, now lay out only that cell instead of the whole screen.
- On Linux, applying a batch of changes reconfigures the widget tree once per batch instead of once per change, so building a large screen no longer slows down with the square of its size.
