---
"release": patch
---

# Draw macOS labels without a text field

`Text` in the generated macOS app is now a lightweight view that draws its string with AppKit's text system, instead of an `NSTextField`. A label field is a control, a cell, and a private drawing view, and every text change runs through all three. The new view sets the same lines in the same box layout measured, so screens look the same and lay out identically.

In the arena on macOS, a wall of 300 labels that change every frame went from about 100 to about 150 updates in five seconds. Creating 1,000 cards went from about 430 ms to 235 ms, and clearing them from 132 ms to 74 ms.

Two things change:

- Labels can't be selected, as on iOS and Android.
- AppKit's own labels darken glyph edges slightly with font dilation that only AppKit's private drawing applies. The new labels ink exactly the same pixels, but their edges are a shade lighter.
