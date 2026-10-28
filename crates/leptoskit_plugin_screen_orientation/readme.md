# leptoskit_plugin_screen_orientation

Requests a scoped orientation policy for an app's owning iOS scene, Android activity, or browser document. Snapshot reports the presented interface; it does not read accelerometer orientation. Register `screen-orientation` in `native.plugins` and add this crate to the app's dependencies.

The native guest `ScreenOrientation::hold(policy)` returns a future that stays pending while the request is held. Retain and poll it alongside your work. Dropping it sends bridge cancellation and restores the previous native policy. Pending means no failure has been reported, **not** that the device rotated or accepted the policy. One hold per owning scene/activity is allowed; overlap returns `Busy`. Background/deactivation, close, or OS rejection ends the hold. Closed instances reject further operations. Snapshot is independent of the requested policy.

Policies are portrait, landscape, their platform-specific primary/secondary orientations, and any. Primary and secondary are not portable physical left/right directions. iOS intersects the request with the application's supported orientations and rejects an empty intersection. Android uses the corresponding ActivityInfo orientation constants. Application manifests, user preferences, fullscreen/multitasking policy, device capabilities and OS policy can override requests. Android 16 large-screen apps may ignore them; resizable iOS apps may do likewise. The plugin does not modify manifest supported orientations or opt the app out of resizing.

## Ownership and restoration

On iOS the generated host supplies a `NativePluginContext` containing a weak reference to the owning controller. The host's `supportedInterfaceOrientations` reads its context mask. The plugin never searches global windows. It restores the exact previous override and requests the previous effective mask; UIKit may reject the geometry change. No physical orientation restoration is promised. Restoration errors are logged. Scene deactivation releases the lease; returning foreground requires a new request.

Android takes the owning Activity, saves its exact `requestedOrientation`, and restores it on cancellation, pause or close. The generated host declares `android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize"` and resizes its rendering surface. Custom hosts must handle those configuration changes too: otherwise an orientation change or folding a foldable can recreate the activity, cancel the lease and restart the guest. The plugin does not restart or recreate activities itself. Restoration errors are returned when a live caller can receive them, otherwise logged. All native policy changes and lifecycle cleanup run on the UI thread. No subscription queue or periodic polling is allocated.

The browser adapter calls `screen.orientation` directly. `web::hold` is a future: poll it to request a lock; drop to unlock. It allows one request per document adapter, releases on hidden/pagehide/fullscreen exit, and removes its listeners on every completion/drop. The browser may require fullscreen/user activation or an installed app; callers must arrange that themselves. The API cannot reveal an existing lock, so release restores the browser default via `unlock()`, not another library's prior lock. Do not mix external orientation locks with this adapter. Explicit browser failures map to permission denied, unavailable, cancelled or native errors. Drop cannot return unlock errors; those are logged to the browser console.

macOS, Windows and Linux native orientation locking is unavailable. Desktop browser support depends on its Screen Orientation implementation. There are no synthetic successful native desktop locks.

## Wire contract

All integers are little-endian u32. Method 0 takes no payload and returns orientation (0 unknown, 1 portrait, 2 landscape), followed by angle (0/90/180/270, or `u32::MAX` unknown). iOS leaves angle unknown; Android uses display rotation; browsers use ScreenOrientation.angle. Their rotation conventions are platform-specific. Method 1 accepts one policy (0 portrait, 1 landscape, 2 portrait-primary, 3 portrait-secondary, 4 landscape-primary, 5 landscape-secondary, 6 any) and never completes successfully. Truncated, trailing, and unknown values are rejected.

## References

- [Expo ScreenOrientation](https://docs.expo.dev/versions/latest/sdk/screen-orientation/)
- [Apple supportedInterfaceOrientations](https://developer.apple.com/documentation/uikit/uiviewcontroller/supportedinterfaceorientations)
- [Apple requestGeometryUpdate](https://developer.apple.com/documentation/uikit/uiwindowscene/requestgeometryupdate(_:errorhandler:))
- [Android orientation and resizability](https://developer.android.com/develop/adaptive-apps/guides/app-orientation-aspect-ratio-resizability)
- [W3C Screen Orientation](https://w3c.github.io/screen-orientation/)
