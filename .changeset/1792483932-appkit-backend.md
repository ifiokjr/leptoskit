---
"release": minor
---

# Add the AppKit backend and styled native elements

leptoskit renders on a real platform: `leptoskit_appkit` commits mutation batches as AppKit views on macOS, measures text with AppKit's own font metrics, delivers button and text-field events, opens windows, and renders view trees to PNG offscreen.

Elements gain cross-platform styling: corner radius, borders, opacity, font size and weight, and text alignment, with old wire JSON still decoding. `TextMeasurer::measure` now receives the text style, so measurement follows the font.

The layout engine measures column items at the column's width so wrapped text gets its real height, caps content by `max_width`, sizes the cross axis of ratio-bound items from `aspect_ratio`, and resolves percentages inside stretched columns.

A new `gallery` example renders six screens through the AppKit backend, published on the documentation site's examples page.
