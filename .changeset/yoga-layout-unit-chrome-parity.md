---
"leptoskit_yoga": patch
"leptoskit_native": patch
---

# Yoga LayoutUnit rounding for Chrome grid parity

Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow.
