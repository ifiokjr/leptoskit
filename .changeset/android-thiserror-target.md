---
"release": patch
---

# Load the Android error type only for Android

`leptoskit_android` declares `thiserror` in its Android target table now, beside the JNI boundary that uses it, so host builds no longer report the dependency as unused.
