---
"release": minor
---

# More of SVG in `Vector`, and cheaper drawings

`Vector` now covers much more of SVG, on every platform:

- Path data accepts elliptical arcs (`A`), and `PathData` builds rectangles, rounded rectangles, circles, ellipses, lines, polylines, and polygons directly. `Affine` transforms paths exactly and parses SVG transform lists.
- Fills and strokes take `Paint::CurrentColor`, which follows the element's text color, and fills take an even-odd fill rule.
- Strokes take dashes and a dash offset, a miter limit, and `StrokeScaling::Drawing` for widths that scale with the drawing, as SVG's do by default.
- `VectorScaling::Fill` covers the frame like SVG's `slice`, and `.align(...)` places fitted and filled drawings like `xMinYMax` and its siblings.
- With the new `svg` feature, `Drawing::parse_svg` imports SVG documents: shapes, groups, transforms, styles, CSS colors, opacity, and stroke properties. Unsupported features are errors that name them.

Drawings are cheaper too. Paths and drawings are shared, immutable values, so cloning, resolving styles, and diffing an unchanged drawing cost a pointer copy or comparison instead of a walk over every point. Path data is formatted once per path for the web and the wire, guest renders reuse the wire bytes of unchanged nodes, and native backends skip redrawing unchanged drawings.

`NativeProps::drawing` is now an `Option<Drawing>` rather than a boxed one, and `VectorPath::stroke_style` returns a reference.
