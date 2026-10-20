# LeptosKit Brightness

Mobile screen brightness with explicit restoration. Add the crate with its `guest` feature, select `brightness` in `native.plugins`, and run `leptoskit native prepare` to register the native handler.

```rust,ignore
use leptoskit_plugin_brightness::{Level, guest::Brightness};
let brightness = Brightness::new()?;
brightness.set(Level::from_fraction(0.8)?).await?;
// When the temporary override is no longer needed:
brightness.restore().await?;
```

`get` returns `Setting::Level` or `Setting::System`. The latter means an Android window inherits the OS setting; it does not invent a level for adaptive brightness. Levels use thousandths (0..=1000) with finite fraction validation and exactly four little-endian bytes.

Android changes only the owning activity's `WindowManager.LayoutParams.screenBrightness` and needs no settings permission. The original value, including the system-default sentinel, is captured on the first set and restored by `restore` or `close`. iOS changes the current foreground scene's screen brightness, which is screen-wide and can outlive app focus. The prior brightness is restored by default on `close`; embedding apps can explicitly choose `BrightnessPlugin(restoreOnClose: false)`. Restoration is idempotent. One plugin instance must own these overrides; another component independently changing the same screen/window cannot be reconciled automatically.

Desktop screen controls and the browser are unsupported and report `Unavailable`; CSS dimming is not a screen-brightness implementation. This port does not write persistent system settings, change adaptive brightness mode, request Android `WRITE_SETTINGS`, or subscribe to brightness changes.

Wire methods: `0` get (empty input, four-byte setting), `1` set (four-byte level, empty output), `2` restore (empty input/output). The value `u32::MAX` is a read-only system-default sentinel and is rejected by set. Native close restores the captured value and rejects subsequent requests with `Cancelled`.

Reference: [Expo Brightness](https://docs.expo.dev/versions/latest/sdk/brightness/).
