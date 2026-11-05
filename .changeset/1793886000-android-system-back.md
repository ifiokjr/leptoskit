---
"release": patch
---

# Return through routes with Android system Back

Generated Android apps report the current router's back availability to the Activity and send system Back to the guest. The Android 13+ dispatcher handles Back buttons and completed back gestures; older Android versions use the legacy callback. At the root the system retains its normal exit behavior.
