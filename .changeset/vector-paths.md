---
"release": minor
---

# Draw filled and stroked paths with `Vector`

`Vector(view_box)` draws SVG-style paths natively: `CAShapeLayer`s on macOS and iOS, an `android.graphics.Path` on Android, cairo on Linux, and anti-aliased GDI+ on Windows, with inline SVG on the web. Paths come from SVG path data (`M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `Z`, absolute or relative) through `PathData::parse`, or from generated `PathCommand`s, and each one takes a fill, a stroke with caps and joins, or both. `VectorScaling::Fit` keeps a drawing's shape, centred in its frame; `VectorScaling::Stretch` fills the frame on both axes. Stroke widths stay in layout units at any scale, so stretched outlines keep an even line.

Native apps had no way to draw a vector shape before: `Image` loads only local files there.
