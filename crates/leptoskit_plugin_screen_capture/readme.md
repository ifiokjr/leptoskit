# leptoskit_plugin_screen_capture

Capture prevention and capture observation have separate capabilities. This plugin never captures pixels, reads a screenshot file, or scans the photo library. Each plugin instance belongs to its host's Activity/window or explicit Apple `NativePluginContext`; it does not search global windows.

| Platform                   | Prevention                       | Recording/mirroring state | Screenshot notification       |
| -------------------------- | -------------------------------- | ------------------------- | ----------------------------- |
| Android                    | Owned window `FLAG_SECURE` lease | Unavailable / unknown     | Android 14+ Activity callback |
| iOS 17+                    | Unavailable                      | Owned scene capture trait | App-wide UIKit notification   |
| macOS, Windows, Linux, web | Unavailable                      | Unavailable               | Unavailable                   |

The iOS screenshot notification cannot identify which app window was captured. It reports app-wide scope, and only a pending observer with a foreground owned scene receives it. Screenshot notifications happen after capture. There is no private text-field/layer workaround pretending to prevent iOS screenshots. Browser screen-sharing permission APIs provide a different contract and are not used as a substitute for prevention or screenshot detection.

Add the crate as a direct dependency and `"screen-capture"` to `native.plugins`. Android registration declares the normal install-time permission `android.permission.DETECT_SCREEN_CAPTURE`. Screenshot observation on Android 14+ returns `PermissionDenied` if the host omits that permission. Earlier Android versions return `Unavailable`; no photo/storage permissions are requested. The Android OS can show a screenshot-detection notice to the user. The host app should explain its use before starting observation.

```rust,ignore
use leptoskit_plugin_screen_capture::{guest::ScreenCapture, Watch};
let capture = ScreenCapture::new()?;
let capabilities = capture.capabilities().await?;
let event = capture.wait(Watch::Screenshot).await?;
```

A prevention request remains pending for its entire lifetime. Pending means no failure has arrived, not an acknowledgement that the secure bit has been set. The host's snapshot can confirm the current window flag. Dropping the future, closing the plugin or destroying its Activity releases the lease. At most 64 leases may share a window, including across plugin instances. The first lease saves the original secure bit; the final release restores that bit exactly while preserving changes to unrelated window flags. Other code changing the same secure bit must coordinate with these leases. A lease may start before the window is shown or while a dialog has focus, so content is protected from its first frame. Screenshot observation needs the Activity's content to be visible, not focused. Android prevention remains active when the Activity stops, protecting recents snapshots until release/destruction. This is an OS window policy, not a guarantee against a compromised OS or an external camera.

Only one observation request may be pending per plugin (`Busy` otherwise). Native subscriptions exist only while that request is pending and are removed on completion, cancellation, registration failure, native background transition or close. Capture-state observation resamples after installing the scene trait observer; intermediate transitions may coalesce. Screenshot events between requests are deliberately not buffered or replayed. The Apple scene must remain the same explicitly owned scene; Android observes only its own Activity.

The binary protocol uses little-endian u32 fields with strict lengths:

| Method         | Request                      | Reply                                                      |
| -------------- | ---------------------------- | ---------------------------------------------------------- |
| 0 capabilities | empty                        | version=1, prevention 0/1, capture scope, screenshot scope |
| 1 snapshot     | empty                        | version=1, capture state, secure flag                      |
| 2 wait         | kind, previous capture state | version=1, kind, capture state                             |
| 3 prevent      | empty                        | stays pending; cancellation/error only                     |

Scope is unavailable/window/scene/application = 0/1/2/3. Capture state is unknown/inactive/active = 0/1/2. Secure flag is unknown/off/on = 0/1/2. Wait kind 0 means capture changed; kind 1 means screenshot and requires previous state 0. Screenshot replies always carry state 0. Rust rejects replies for the wrong requested kind, unchanged capture-change replies, unsupported versions, unknown values, truncation and trailing data.

The implementation follows the capability areas exposed by [Expo ScreenCapture](https://docs.expo.dev/versions/latest/sdk/screen-capture/), using Android's public [secure-window and screenshot callback APIs](https://developer.android.com/about/versions/14/features/screenshot-detection) and Apple's public [scene capture trait](https://developer.apple.com/documentation/uikit/uitraitcollection/scenecapturestate). Native tests use synthetic notifications/subscriptions or an owned fixture Activity. They never capture user apps or data. Actual screenshot/recording notifications and OS compositor enforcement still require device verification.
