---
"release": minor
---

# Render shared primitives with Android Views

Add `leptoskit_android` and the `leptoskit` facade's `android` feature. Compile the crate's `android/WidgetHost.kt` into an Android application and construct `AndroidBackend` from it on the UI thread. Every existing native primitive renders a real Android view with exact Yoga outer frames in dp, `StaticLayout` text measurement that matches the drawn text, controlled inputs, native accessibility, RTL, scrolling, and lifecycle support. Android 9 (API 28) or newer is required.

CI renders every primitive on an Android emulator and fails unless the probe Activity reports a verified result.
