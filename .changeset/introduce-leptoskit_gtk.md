---
leptoskit_gtk: breaking
---

# leptoskit_gtk: the Linux backend

`leptoskit_gtk` renders the shared element tree as real GTK4 Linux widgets, consuming the same Yoga mutation stream as every other backend.

Initialize GTK on the application thread, construct `GtkBackend`, and hand it to `NativeSurface`. `open_window` hosts a committed root and `run_app` drives the loop; GTK objects stay on their creating thread.
