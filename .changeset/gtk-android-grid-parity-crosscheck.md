---
"leptoskit_gtk": patch
---

# GTK grid parity probe against Yoga frames

Adds a `grid_parity_gtk` example that mounts the same Tailwind grid/flex fixtures as the web probe on real GTK 4 widgets under Xvfb, asserts allocations match Yoga within half a CSS pixel, and writes PNGs for cross-device layout evidence.
