# leptoskit_plugin_battery

Battery charge, charging state, and system power-saving state, with Swift and Kotlin handlers for generated mobile apps. Unknown readings remain `None`; an iOS simulator is not reported as a fully charged physical battery.

Add the crate to the native host and select `"battery"` in `native.plugins`. The generator discovers `BatteryPlugin` from Cargo package metadata. No mobile permission prompt is needed. This crate requires Rust 1.89 because its desktop battery dependency requires that version.

Guest applications enable the `guest` feature:

```rust,ignore
let battery = leptoskit_plugin_battery::guest::Battery::new()?;
let mut current = battery.snapshot().await?;
loop {
    update_display(current);
    current = battery.wait_for_change(current).await?;
}
```

Dropping `wait_for_change` cancels the native request and removes its observers. Only one waiter may be active per native plugin; another returns `Busy`. Each wait installs observers before sampling, so changes between calls are detected. The result is the latest state: intermediate transitions can be coalesced, and a slow consumer does not accumulate an event backlog. Closing the host cancels outstanding subscriptions before releasing the plugin.

| Platform              | Snapshots                                             | Change observation                                             | Power saving   |
| --------------------- | ----------------------------------------------------- | -------------------------------------------------------------- | -------------- |
| iOS                   | `UIDevice`                                            | Battery and process power-state notifications                  | Low Power Mode |
| Android               | Battery broadcast                                     | Battery and power-save broadcasts                              | Power Saver    |
| macOS Swift host      | IOKit internal battery                                | IOKit run-loop and process power-state notifications           | Low Power Mode |
| macOS, Windows, Linux | `desktop::batteries()` through OS battery enumeration | `desktop::Watcher`, explicit polling at least one second apart | Unknown        |
| Browser               | `web::snapshot()` when Battery Status API exists      | `web::wait_for_change()`, listeners removed on future drop     | Unknown        |

The desktop `Battery` bridge adapter exposes snapshots only. `Watcher::poll` performs synchronous OS I/O and should run off the UI thread; it creates no background thread and does not claim to use OS notifications. Multiple desktop batteries are returned individually. The bridge snapshot uses the first battery. Generated macOS apps use the Swift IOKit implementation, including native notifications. Other desktop Rust hosts can use `desktop::batteries()` and `desktop::Watcher`.

Android additionally provides `guest::Battery::optimization_enabled()` for the application's battery-optimization exemption policy. This is separate from the device-wide Power Saver state. Other platforms return `Unavailable`. Browsers without the API return `Unavailable`; browser security-policy rejection returns `PermissionDenied`, and other native failures remain `Native`.

Snapshot payloads are exactly 16 bytes: four little-endian `u32` fields for presence (`0` unknown, `1` absent, `2` present), charge in thousandths (`u32::MAX` unknown), state (`0` unknown, `1` discharging, `2` charging, `3` full), and power-saving (`0` unknown, `1` off, `2` on). All decoders reject invalid values and trailing bytes. Method `0` reads a snapshot; method `1` waits for a snapshot different from the supplied 16-byte previous value; Android method `2` returns the optimization policy as a four-byte `0` or `1`.

API reference: [Expo Battery](https://docs.expo.dev/versions/latest/sdk/battery/), [Apple battery monitoring](https://developer.apple.com/documentation/uikit/uidevice/isbatterymonitoringenabled), and [Android battery monitoring](https://developer.android.com/training/monitoring-device-state/battery-monitoring).
