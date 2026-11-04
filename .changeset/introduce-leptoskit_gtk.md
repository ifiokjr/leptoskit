---
leptoskit_gtk: major
---

# leptoskit_gtk: the Linux backend

`leptoskit_gtk` renders the shared element tree as real GTK 4 Linux widgets, consuming the same Yoga mutation stream as every other backend, and draws every built-in component. It needs GTK 4.6 or newer.

The Linux app `leptoskit new` generates uses it through the `linux` feature. A host initializes GTK on the application thread, hands a `GtkBackend` to `NativeSurface`, shows a committed root with `open_window`, and runs the loop with `run_application`; GTK objects stay on their creating thread.
