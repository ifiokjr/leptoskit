# Pedometer

A bounded, demand-driven CoreMotion/Android step-counter API. Select Cargo crate `leptoskit_plugin_pedometer` and runtime `pedometer`; native registration is automatic.

The app must provide iOS `NSMotionUsageDescription` and Android `android.permission.ACTIVITY_RECOGNITION` in its own plist/manifest. On Android 10+ runtime authorization uses the generated host permission broker. `permission()` never prompts. `request_permission()` is explicit and bounded to 30 seconds; timeout returns Unavailable without guessing authorization. iOS has no standalone CMPedometer permission request: the explicit request briefly starts live updates from now, discards sample data, observes authorization, and stops. It does not query historical activity. Canceling cannot dismiss an OS permission dialog. Unrequested and permanently denied Android access may both be Unknown; query is not a claim that asking again will work.

`next(timeout)` accepts 1–120000 ms and permits one outstanding operation. Drop, native close, deadline, and background stop live updates and remove callbacks. Hardware availability is separate from authorization. iOS reads require observed authorization before acquisition and recheck it on delivery. Android Activity-backed acquisition requires a focused foreground window. Android rechecks runtime permission before registration and delivery. Android sampling is requested at 200 ms; native step-counter latency can be up to ten seconds, and no new steps may mean no fresh event before timeout.

Count scopes deliberately remain distinct:

- iOS `Interval` contains native reported steps and the native start/end Unix millisecond timestamps. Live observations start when requested. Repeated requests have gaps, so adding their results is not a complete daily total.
- Android `BootCounter` preserves the native cumulative count since reboot while the sensor was active and its monotonic event timestamp in seconds. It is not a daily total or a guarantee that all steps since reboot were recorded. Android exposes this count as a float, so large values carry float precision limits. Counts are algorithmic estimates, not measured ground truth.

`history(range, timeout)` is iOS-only. Requests must fall entirely within the past seven days. The OS can only return available stored data. The CMPedometer historical query cannot itself be canceled: timeout/cancel releases the caller but retains the plugin's Busy slot until the native callback returns, preventing unbounded native queries. Android history, browser pedometers and desktop pedometers return Unavailable; no accelerometer-derived counts or invented zero readings substitute for missing support. Genuine reported zero counts are valid.

Physical hardware, real permission dialogs, and activity history have not been read or validated in this implementation session. Tests are synthetic protocol/lifecycle fixtures.

References: [Expo Pedometer](https://docs.expo.dev/versions/latest/sdk/pedometer/), [Android motion sensors](https://developer.android.com/develop/sensors-and-location/sensors/sensors_motion), [Apple CMPedometer](https://developer.apple.com/documentation/coremotion/cmpedometer).
