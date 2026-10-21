# leptoskit_plugin_haptics

Selection, impact and notification feedback with binary plugin messages.

- iOS uses UIKit feedback generators.
- Android uses `View.performHapticFeedback`, respecting the user's settings without requesting vibration permission. The Android-specific method accepts public `HapticFeedbackConstants` supported by the current API level.
- macOS uses its trackpad haptic performer. Its three patterns approximate the richer mobile styles.
- Browsers use the Vibration API when available and allowed by user activation/device policy.
- Windows and Linux return `Unavailable`; this crate does not assume a game controller or other actuator exists.

Add the crate as a direct app dependency and select `"haptics"` in `native.plugins`. Native registration is automatic. Interpreter guests enable the `guest` feature and construct `guest::Haptics` once to cache its numeric plugin ID.

Success means the request was accepted. The OS may suppress physical feedback because of settings, power state, hardware or foreground restrictions.

The portable binary contract uses method 0 with no payload for selection, method 1 with one little-endian `u32` impact style (light, medium, heavy, rigid, soft), and method 2 with one `u32` notification kind (success, warning, error). Method 3 accepts an Android constant. Successful replies are empty. Truncated, trailing or unknown portable values fail before requesting hardware feedback.

References: [Expo Haptics](https://docs.expo.dev/versions/latest/sdk/haptics/), [Android feedback constants](https://developer.android.com/reference/android/view/HapticFeedbackConstants), [macOS haptic performer](https://developer.apple.com/documentation/appkit/nshapticfeedbackmanager).
