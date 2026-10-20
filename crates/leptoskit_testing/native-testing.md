# Native UI testing

Enable `leptoskit_testing`'s `native-ui` feature in your **host-side** test program. `NativeTester` drives the operating system through a real native accessibility driver. It can click native controls, type text, interact with system permission dialogs, and capture their rendered pixels. The app under test runs on the target device; the Rust test runs on a desktop host.

| Target  | Driver                             | Host environment                                                             |
| ------- | ---------------------------------- | ---------------------------------------------------------------------------- |
| iOS     | Appium XCUITest                    | macOS with Xcode and an installed simulator runtime, or a provisioned device |
| Android | Appium UI Automator 2              | Android SDK and an emulator or device                                        |
| Windows | Appium NovaWindows / UI Automation | An interactive Windows desktop; use a dedicated VM or CI runner              |
| Linux   | KDE Selenium AT-SPI                | An accessible graphical session; the provided fixture jobs use nested KWin   |

These drivers are external processes. Install them on the machine that owns the device or desktop and pass its HTTP endpoint. An endpoint can be remote: a macOS test controller can target a Windows VM's driver. Installing a driver does not supply an OS image, emulator runtime, or device signing credentials.

This adds testing support for native applications. It does **not** implement Leptoskit's unfinished iOS, Android, Windows, or Linux rendering backends. The headless `WidgetTester` remains useful for shared widget behavior; it cannot prove those backends render correctly.

## Run a scenario

```rust,no_run
use leptoskit_testing::native::{Application, Locator, NativeTester, Options, Platform};

let mut options = Options::new(
    "http://127.0.0.1:4723",
    Platform::Android,
    Application::Path("/driver-host/app.apk".into()),
);
options.device = Some("emulator-5554".into());
NativeTester::run(options, |tester| {
    tester.tap(&Locator::accessibility_id("enable-notifications"))?;
    let allow = Locator::Id(
        "com.android.permissioncontroller:id/permission_allow_button".into(),
    );
    tester.expect_visible(&allow)?;
    tester.checkpoint("OS permission dialog")?;
    tester.tap(&allow)?;
    tester.expect_visible(&Locator::text("Notifications enabled"))?;
    Ok(())
})?;
# Ok::<(), leptoskit_testing::native::Error>(())
```

For Windows/Linux dialogs in other processes, use a separate `Application::Desktop` session. Start your app in that desktop before connecting. Desktop sessions borrow the existing environment; they do not close unrelated applications. Mobile sessions require an explicit device identifier. XCUITest and UI Automator can query system UI outside your app; automatic permission acceptance is disabled by default.

`tap`, `enter_text`, and assertions resolve selectors afresh, wait for visible controls, and reject ambiguity. Actions execute once: a lost response is an error, not permission to click twice. Read-only stale-element observations can retry until the configured deadline. Text/name selectors depend on the OS language; keep CI devices in a known locale or use stable resource/accessibility IDs.

Driver extensions remain available through `native_command`, and W3C pointer/key actions through `perform_actions`. Their supported operations differ by driver; unsupported commands fail explicitly. Protected surfaces such as the Windows secure desktop or Android secure windows may intentionally deny automation/capture.

Use `NativeTester::run` to retain failure evidence and propagate cleanup errors. Manually managed sessions must call `finish`. The synchronous client belongs outside async runtimes, or in their blocking pool. Every session gets an isolated artifact directory, so parallel tests cannot overwrite each other's images or reports. Use separate device IDs and driver ports for concurrent native sessions.

## Real screenshots and optional layout diagnostics

`tester.screenshot()` returns the native driver's PNG. It validates the image, preserves its original encoded bytes, and never falls back to colored rectangles. Fonts, images, OS compositing, and visible dialogs come from the actual screen. Dimensions are physical pixels; native action coordinates may use logical units.

`tester.checkpoint("label")` saves that PNG, the accessibility XML, and `index.html`. The HTML contains ordinary image elements with links to the full-resolution PNGs. Open it locally or download the complete CI artifact directory; images render directly. Captures can contain OS UI outside your app, so run fixtures in dedicated test environments.

For fast headless diagnosis, `widget_tester.layout_image()` separately produces colored widget bounds. Its Rust type is `LayoutImage`, its PNG metadata says it is synthetic, and it should be saved as `layout-diagnostic.png`. This output helps explain layout; it does not render text, images, or native system UI.

## JSON scenarios

The CLI also runs the same native API:

```sh
leptoskit test native --suite native-test.json
```

```json
{
	"session": {
		"endpoint": "http://127.0.0.1:4723",
		"platform": "android",
		"device": "emulator-5554",
		"application": { "kind": "path", "value": "/driver-host/app.apk" }
	},
	"steps": [
		{
			"action": "tap",
			"locator": { "by": "accessibility_id", "value": "continue" }
		},
		{
			"action": "expect_visible",
			"locator": { "by": "text", "value": "Welcome" }
		},
		{ "action": "screenshot", "label": "Welcome screen" }
	]
}
```

Other steps include `enter_text`, `expect_text`, `expect_absent`, `accept_alert`, `dismiss_alert`, `native_command`, and `perform_actions`. Unknown fields and empty suites fail. Additional driver capabilities go in `session.capabilities`; application identity, platform, and device selection use typed fields.

The fixture conformance jobs exercise real controls and system dialogs on each OS. Protocol tests use a local HTTP server to exercise malformed responses, ambiguous controls, cleanup failures, evidence retention, and prevention of repeated mutations; those tests are not evidence of OS support.
