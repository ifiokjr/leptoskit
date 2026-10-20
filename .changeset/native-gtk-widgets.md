---
"release": minor
---

# Render shared primitives with GTK4 on Linux

Add `leptoskit_gtk` and the `leptoskit` facade's `gtk` feature. The backend renders every existing native primitive with real GTK widgets, exact Yoga outer frames, controlled inputs, platform text measurement, styling, accessibility, scrolling, and lifecycle support. GTK 4.6 or newer is required.

Linux workspace tests now run real GUI checks under D-Bus and Xvfb when no display is available. GUI tests are serialized to preserve native keyboard focus. The development environment includes GTK and the native verification tools.
