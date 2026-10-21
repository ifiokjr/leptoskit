---
"release": patch
---

# Re-export JNI bindings from the Android backend

`leptoskit_android` (and `leptoskit::android`) now re-exports `jni`, so an Activity's JNI entrypoints can construct `AndroidBackend` with the same `jni` version the backend uses, without declaring the dependency themselves.

The gallery example renders its shared screens with every native backend: AppKit, GTK, Win32, UIKit, and Android Views. CI snapshots every screen on each desktop OS and fails unless the iOS app and the Android instrumentation report every screen as verified.
