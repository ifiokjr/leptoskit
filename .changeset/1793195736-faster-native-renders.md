---
"release": patch
---

# Render native updates with a fraction of the work

Native apps run their screens as WebAssembly in an interpreter, so the work a render does inside the guest dominates how fast an update reaches the screen. Three changes cut that work:

- An update sends the host only the elements whose properties changed. Unchanged elements reuse the bytes encoded by the previous render.
- Each distinct Tailwind class string is parsed once, rather than once per element on every render.
- Unset properties, a default layout style, and an empty class list are left out of what the guest sends the host. A typical element now encodes to a few dozen bytes instead of about a kilobyte. Every host and renderer already reads a missing property as unset, so this works in both directions with code push: old bundles keep working with new hosts, and new bundles with old hosts.

In the arena's big-bang scenario on macOS, creating 1,000 cards went from 726 ms to 517 ms, and updating every tenth card from 300 ms to 67 ms.
