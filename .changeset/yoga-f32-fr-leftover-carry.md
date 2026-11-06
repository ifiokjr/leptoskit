---
"leptoskit_yoga": patch
---

# Blink f32 leftover-carry for flexible tracks

Flexible grid tracks now distribute free space with binary32 flex shares, flooring each share to a 1/64 CSS-pixel layout unit and carrying the truncated fraction into the next track—matching Chrome’s `getBoundingClientRect` for equal and weighted `fr` rows/columns. Non-integral percentages such as `w-1/3` floor into a layout unit; whole-number percentages stay exact.
