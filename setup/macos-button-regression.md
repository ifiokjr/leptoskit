# macOS Button measurement

The recording application uses its existing AppKit text measurer. Buttons now select its native-control operation rather than its label operation. Existing custom text measurers retain their behavior through the default trait method. Caller padding, borders and width constraints remain layout inputs.

The current regular rounded control reserves a half-point-rounded title width, 24 points of horizontal chrome and a 34-point minimum width. Its native height is 24 points, including at oversized fonts. Large glyphs can paint outside that height. A clipping parent must reserve any desired additional space; this change does not promise visibility beyond a clipped parent. Custom background, border or corners replace native chrome, and removing them restores it.

## Actual geometry evidence

An offscreen native control sampled 80 cases: four titles, five system font sizes from 10 to 42 points, regular and bold weights, and bordered and borderless modes. Each case assigned its frame before reading native rectangles and painting into a generous parent bitmap. The title retained its native font and geometry while its foreground changed to red for glyph accounting. That change altered no metrics.

All proposed and native frames matched the generous reference's glyph counts and coverage. The old label allocation lost coverage in 30 nonempty rounded cases. For the unchanged journey title at 14 points, the old 239 by 17 allocation visibly truncated. The native 258.5 by 24 allocation preserved all reference glyphs. This is sampled functional evidence, not a future operating-system guarantee. The offscreen drawing did not launch an installed application or a window.

The concrete Swift compiler and one control execution completed successfully. Compiler stderr retained two sandbox-application diagnostics. Their originating child and the complete intended denial boundary were not independently proved. These qualifications do not establish a performance or installed journey result.

## Maintained checks

The new native tests cover default trait compatibility, Button-only routing, custom-chrome invalidation, retained nodes and explicit widths. The AppKit harness compares actual control metrics and measures off the main thread. The real FFI application tests allocate the unchanged long title, a deliberately narrow button and scaled native/custom controls. The Swift renderer tests compare title pixels against native references and exercise custom/native property transitions.

The full native suite passed 207 tests and two doctests, including absent and empty Button titles.

The original 14 AppKit bodies and three Button bodies passed, including 50 native-cell comparisons.

The three real FFI Button bodies and the unchanged Text body's seven cases passed.

The full Swift package passed, including all five Button bodies and eight parameter cases.

All original Text test bodies remain intact.

A distinct-target negative must retain the real FFI allocation assertion and original label selection.

Generated CLI output and installed application journeys remain separate acceptance checks.
