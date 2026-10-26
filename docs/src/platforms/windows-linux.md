# Windows and Linux

On Windows, your screens are drawn with Win32 common controls: `BUTTON`, `EDIT`, `STATIC`, trackbar, and progress bar windows. On Linux, they are drawn with GTK 4 widgets. Both use leptoskit's layout engine and the platform's own font measurement, and neither embeds a browser.

## Build and run

Each desktop app builds on its own operating system.

<Tabs>
<Tab label="Windows">

You need the MSVC Rust toolchain and the Windows SDK.

```sh
rustup target add wasm32-unknown-unknown
leptoskit build --platform windows
```

The build prints the path of the executable. The app embeds the common-controls manifest, so controls get the current Windows look, and it lays the screen out again when the window is resized or moved to a display with a different DPI.

</Tab>
<Tab label="Linux">

You need GTK 4.6 or newer, its development files, and `pkg-config`. On Debian and Ubuntu:

```sh
sudo apt install libgtk-4-dev pkg-config
rustup target add wasm32-unknown-unknown
leptoskit build --platform linux
```

The build prints the path of the executable. The app runs in the GTK event loop and passes control events to your screens.

</Tab>
</Tabs>

The `windows/` and `linux/` folders are yours: each holds a small Rust program that opens the window and runs your app, plus its `Cargo.toml` for native dependencies. `leptoskit native prepare windows` or `leptoskit native prepare linux` regenerates the build inputs without producing the executable. See [host projects](host-projects.md) for what else lives where.

## How they differ from macOS

- **Windows:** `Switch` is drawn as a checkbox. Rounded corners are cut with a window region, so their edges aren't smoothed, and borders stay rectangular inside them. `ActivityIndicator` is drawn by leptoskit rather than a system control. Accessibility, through UI Automation, has no expanded or collapsed state. A `Vector` with no background takes clicks only where it has ink; clicks on its empty parts reach whatever is underneath.
- **Linux:** live regions update their text but aren't announced, because announcements need GTK 4.14.
- On both, `Image` loads local files, not URLs, and the generated apps don't check for [code push](../guide/05-code-push.md) updates; ship a new build to update them.

Build and test on the operating system you ship to. A build that compiles on another OS says nothing about how the widgets render or respond.
