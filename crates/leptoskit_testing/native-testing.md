# Native UI testing

Enable `leptoskit_testing`'s `native-ui` feature in your **host-side** test program. `NativeTester` drives the operating system through a real native accessibility driver. It can click native controls, type text, interact with system permission dialogs, and capture their rendered pixels. The app under test runs on the target device; the Rust test runs on a desktop host.

| Target  | Driver                             | Host environment                                                             |
| ------- | ---------------------------------- | ---------------------------------------------------------------------------- |
| iOS     | Appium XCUITest                    | macOS with Xcode and an installed simulator runtime, or a provisioned device |
| Android | Appium UI Automator 2              | Android SDK and an emulator or device                                        |
| macOS   | Appium Mac2 (XCTest for macOS)     | Xcode and an interactive macOS session that allows UI automation             |
| Windows | Appium NovaWindows / UI Automation | An interactive Windows desktop; use a dedicated VM or CI runner              |
| Linux   | KDE Selenium AT-SPI                | An accessible graphical session; the provided fixture jobs use nested KWin   |

These drivers are external processes. Install them on the machine that owns the device or desktop and pass its HTTP endpoint. An endpoint can be remote: a macOS test controller can target a Windows VM's driver. Installing a driver does not supply an OS image, emulator runtime, or device signing credentials.

This adds testing support for native applications. It does **not** implement Leptoskit's unfinished iOS, Android, macOS, Windows, or Linux rendering backends. The headless `WidgetTester` remains useful for shared widget behavior; it cannot prove those backends render correctly.

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

A macOS session launches one application bundle (`Application::Path` to the `.app`, or `Application::Identifier` with its bundle ID) and sees everything that application presents, including its sheets, open and save panels, and alerts; it quits that application when the session ends, so consecutive journeys each start from a clean screen. There is no desktop-root session on macOS. Mac2 resolves `Locator::AccessibilityId` to AppKit's accessibility identifier; `Locator::text` matches an element's title, label, or value, which covers button titles and label text. Before the first session, allow UI automation without a password prompt: `sudo automationmodetool enable-automationmode-without-authentication`. The driver then builds its XCTest runner with Xcode, which takes a few minutes on a fresh machine.

For Windows/Linux dialogs in other processes, use a separate `Application::Desktop` session. Start your app in that desktop before connecting. Desktop sessions borrow the existing environment; they do not close unrelated applications. Mobile sessions require an explicit device identifier. XCUITest and UI Automator can query system UI outside your app; automatic permission acceptance is disabled by default.

`tap`, `enter_text`, and assertions resolve selectors afresh, wait for visible controls, and reject ambiguity. Actions execute once: a lost response is an error, not permission to click twice. Read-only stale-element observations can retry until the configured deadline. Reads inside a wait share the wait's deadline (`wait_seconds`). Any one driver command may take up to the longer of `command_seconds` and `wait_seconds`; iOS sessions ask WebDriverAgent to enforce that bound, and the client waits slightly longer so a slow action is reported by the driver instead of abandoned mid-flight. iOS also limits XCTest's pre-action idle wait to one second, because a focused field's caret keeps the app from ever idling. iOS accessibility trees omit the `visible` and `accessible` attributes: while a system alert is up the tree comes from SpringBoard, where computing them can stall past any deadline. Set `appium:settings[pageSourceExcludedAttributes]` to `""` to include them. Text/name selectors depend on the OS language; keep CI devices in a known locale or use stable resource/accessibility IDs.

Driver extensions remain available through `native_command`, and W3C pointer/key actions through `perform_actions`. Their supported operations differ by driver; unsupported commands fail explicitly. Protected surfaces such as the Windows secure desktop or Android secure windows may intentionally deny automation/capture.

Use `NativeTester::run` to retain failure evidence and propagate cleanup errors. Manually managed sessions must call `finish`. The synchronous client belongs outside async runtimes, or in their blocking pool. Every session gets an isolated artifact directory, so parallel tests cannot overwrite each other's images or reports. Use separate device IDs and driver ports for concurrent native sessions.

## Real screenshots and optional layout diagnostics

`tester.screenshot()` returns the native driver's PNG. It validates the image, preserves its original encoded bytes, and never falls back to colored rectangles. Fonts, images, OS compositing, and visible dialogs come from the actual screen. Dimensions are physical pixels; native action coordinates may use logical units.

`tester.checkpoint("label")` saves that PNG, the accessibility XML, and `index.html`. The HTML contains ordinary image elements with links to the full-resolution PNGs. Open it locally or download the complete CI artifact directory; images render directly. Captures can contain OS UI outside your app, so run fixtures in dedicated test environments.

`Screenshot::distinct_colors(limit)` counts distinct pixel values and stops once it passes `limit`. A rendered screen with text and controls has many; a blank or placeholder frame has very few. Assert a lower bound (the fixture journeys require more than 32) to reject a driver that returns a substitute image.

For fast headless diagnosis, `widget_tester.layout_image()` separately produces Patrol-style flat-colour widget bounds. Its Rust type is `LayoutImage`, its PNG metadata says it is synthetic, and it should be saved as `layout-diagnostic.png`. It never contains more than `LayoutImage::MAX_COLORS` colours, which `distinct_colors` can assert. This output helps explain layout; it does not render text, images, or native system UI.

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

## Add a native integration test

The conformance fixtures in `crates/leptoskit_testing/fixtures` and the `native UI` workflow (`.github/workflows/native-ui.yml`) are the working template. To cover a new feature on all five platforms:

1. **Write the journey once, in Rust.** Add a host-side program behind the `native-ui` feature, as `examples/native_conformance.rs` does, and run it with `NativeTester::run`. Take the platform, driver endpoint, app path, device, and any variant (such as `allow`/`deny`) as arguments, so the CLI wrapper for each OS stays a thin launcher. Prefer accessibility IDs. Use `Locator::Id` with Android resource IDs for system dialogs, and visible text only in a known locale.
2. **Prove real behaviour, not just presence.** Act through the UI, wait for the visible result, and call `checkpoint` before and after each system dialog so the report shows the actual screen. Finish with `screenshot()` plus a `distinct_colors` lower bound. For shared layout, add a `WidgetTester` check to `examples/widget_suite.rs`: `leptoskit test run` executes it on the iOS simulator, Android emulator, Linux, Windows, and macOS, and its `layout_image()` must stay within `LayoutImage::MAX_COLORS`.
3. **Reuse the platform launchers.**
   - iOS and Android: `setup/scripts/native_mobile_run.sh <ios|android> [device]` builds the fixture and starts the pinned Appium driver from `setup/native-ui`. On Android it waits for the emulator to settle and installs the UiAutomator2 server packages one at a time; concurrent installs right after boot dropped the adb transport on hosted runners. It also keeps logcat in `target/native-ui/logcat-android.log`.
   - macOS: `setup/scripts/native_macos_run.sh` builds the AppKit fixture bundle, wraps the dialog plugin's fixture in a bundle, and starts the pinned Mac2 driver. Put files a journey's open panel should select beside the bundle; panels do not browse inside one.
   - Windows: `setup/scripts/native_windows_run.ps1` starts NovaWindows on an interactive desktop and runs the journey against the desktop root.
   - Linux: `setup/native-ui/Dockerfile` builds an isolated KWin session with the AT-SPI WebDriver. `setup/scripts/native_linux_session.sh` runs inside it.
4. **Add the CI job.** Extend the matching job in `native-ui.yml`, or copy one for a new fixture. Keep the `retain native evidence` upload with `if: always()`, so failures keep their `index.html`, screenshots, accessibility trees, and driver logs. A failed session start also writes a report that records the driver error.
5. **Isolate state.** Uninstall or reset only your fixture between variants, as the mobile script does with `dev.leptoskit.capturelab`. Give concurrent sessions their own device and driver port.

Driver versions are pinned in `setup/native-ui/package-lock.json`. Update them with `npm install --prefix setup/native-ui`, then run all five native jobs before merging.
