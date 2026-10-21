---
"release": minor
---

# Render shared primitives with UIKit on iOS

Add `leptoskit_uikit` and the `leptoskit` facade's `uikit` feature. `UIKitBackend` renders every existing native primitive as real UIKit views with exact Yoga frames, Core Text measurement, controlled inputs, VoiceOver traits and values, scrolling, and animation lifecycle support. `UIKitHost` retains one surface through a native view controller and forwards logical bounds and rotation. iOS 15 is the minimum.

CI renders every primitive in a signed UIApplication on a fresh simulator and fails on any native assertion.
