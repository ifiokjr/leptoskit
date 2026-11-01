---
"release": minor
---

# Native apps outline and pick any element

Selecting an element in DevTools now outlines it inside the running macOS, iOS, or Android app, in the same magenta as on the web, and the outline follows the element as you scroll. **Pick** takes your next tap or click anywhere in the app, not only on a control, and selects the element under it, including layout containers that draw nothing of their own; the app never receives that tap. Native elements also show the CSS each class resolves to in the Styles panel, as web elements do.

- **For hosts.** `leptoskit_bridge_ffi` exports `lk_devtools_active` and `lk_devtools_pick`, and `lk_devtools_frame` gains `anchor`, `anchor_x`, and `anchor_y`: the nearest view the platform draws for the node and the node's frame inside it. The Swift and Kotlin renderers add their overlay only while the app reports to a hub, so other builds draw and intercept nothing.
- **For tools.** `leptoskit_devtools::Mirror` gains `anchor` and `hit`, `Change` gains `Drawn` and `Styling`, `Highlight` gains `anchor`, and `Reporter::tap` picks by a tap's position. Mirrored nodes carry `styles`.
- **Viewport frames.** `NativeSurface::devtools_tree` reports frames in viewport space, as the protocol says, rather than relative to the parent, so it agrees with what native apps report. `NativeSurface::style_context` returns what the surface resolves classes against.
