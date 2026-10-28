---
"release": patch
---

# Flatten layout-only views

Containers that only arrange their children no longer get a native view in the generated iOS, Android, macOS, and HarmonyOS apps. That's a `View` with no background, border, rounded corners, opacity, clipping, hiding, accessibility label or role, or id. Its children join the nearest native ancestor at the same position on screen, so each flattened container saves creating, laying out, and compositing one view. React Native's Fabric does the same.

A container that later needs a view, because it gained a background for example, becomes one in place and takes its children back. Taps and accessibility are unchanged: events are dispatched on leptoskit's own tree, and a flattened container had no accessible role to begin with.

A differential test drives a surface through random screens and edits, and checks after every commit that each drawn element keeps its position, properties, and sibling order. It covers more than 8,000 flattened containers and 400 promotions.
