# macOS

On macOS, your screens are drawn with AppKit: layer-backed `NSView`s for containers, `NSTextField` for inputs, `NSButton`, `NSImageView`, and `NSScrollView`. Text is measured with the same metrics AppKit draws with, so the layout reserves exactly the space each label needs. In the generated app, a label is a lightweight view that draws its text with AppKit's text system rather than an `NSTextField`: it sets the same lines in the same place for a fraction of the cost per update. Nothing runs in a web view.

macOS is leptoskit's reference backend: every screenshot on this site is a leptoskit screen committed as real AppKit views.

The [shell plugin](plugin-inventory.md) groups each permitted command with its descendants and terminates that group when the invocation ends. Commands that deliberately leave the group need additional containment from the host.

## Build and run

You need a Mac with Xcode or the Xcode command line tools, and the WebAssembly target:

```bash
rustup target add wasm32-unknown-unknown
leptoskit build --platform macos
open .leptoskit/macos/App.app
```

The build compiles your app to WebAssembly, builds the AppKit host for your Mac's architecture, and assembles `.leptoskit/macos/App.app`. It signs the app ad hoc, which is enough to run it on your own Mac. To distribute it, `leptoskit publish macos` signs it with your Developer ID, notarizes it, and makes a `.dmg` and a `.zip`; see [Publishing to the stores](../guide/24-publishing.md#a-notarized-macos-download).

While you work, `leptoskit dev --platform macos` builds and opens the app, and replaces its interface in the same window each time you save. See [building and serving](../guide/04-building.md#leptoskit-dev).

The `macos/` folder is yours: `App.swift` starts the app and owns its window, `Info.plist` holds its name and version, and `Cargo.toml` lists native Rust dependencies such as [runtime plugins](plugins.md). A plugin's third-party Swift dependencies come through Swift Package Manager and resolve automatically; CocoaPods libraries are iOS-only, because the macOS host builds with Swift packages.

## How it differs from the other platforms

- The generated app draws the common controls: `View`, `Text`, `Button`, `Pressable`, `TextInput`, `PasswordInput`, `ScrollView`, `Image`, `Vector`, and `Rive`, plus `Switch`, `Checkbox`, `Radio`, `Slider`, `LinearProgressIndicator`, and `ActivityIndicator`. Switches use `NSSwitch`, choices use `NSButton`, sliders use `NSSlider`, and indicators use `NSProgressIndicator`. Update checked or slider state in your handler to accept a change; otherwise the control restores the previous value. Custom widgets remain unsupported. See [generated apps and backend crates](readme.md#generated-apps-and-backend-crates).
- [Rive](../guide/20-rive.md) displays native vector frames in an `NSImageView`. The generated host uses a display link and follows the system reduced-motion preference. Native Rive rejects text, image, layout, script, and audio assets.
- Pressing a button with the keyboard (Space, with Full Keyboard Access on) or through VoiceOver activates it without showing its [pressed style](../guide/07-shared-components.md#style-the-pressed-state). A mouse press shows it.
- Transforms and [animations](../guide/32-animations.md) run on Core Animation, as on iOS, with the same limits: a widget animating out is drawn above its former siblings until it is gone, and text color changes don't animate.
- `Image` loads local paths and `file://` URLs, not web URLs.
- [Sign-in](../guide/23-sign-in.md#native-apps) works as on iOS: the system's passkey sheet and Sign in with Apple's own sheet once the app has its associated domain, which the generated project doesn't add yet, and other providers in an `ASWebAuthenticationSession`.
- Labels in the generated app can't be selected, as on iOS and Android. AppKit's own labels also darken glyph edges slightly with private font dilation that no public API offers, so text there is a shade lighter than in an `NSTextField`.
- The generated macOS app doesn't check for [code push](../guide/05-code-push.md) updates; ship a new build to update it. The iOS and Android apps do.
- AppKit follows the system's light or dark appearance until the app calls `NativeSurface::set_dark_mode`. An app with a fixed light theme should call `set_dark_mode(false)` before it shows its first screen.

## Application menus

The generated Swift host still ships a small hardcoded Edit menu. A Rust desktop host that owns AppKit itself builds a menu bar with `leptoskit_plugin_menu` and installs it on the main thread. Prefer the chained builders (`MenuBar`, `Submenu`, `Command`, `Check`) so the tree matches leptoskit's usual call style; validation still runs before any `NSMenu` is allocated. Root entries must be submenus, which is AppKit's rule.

```rust
use leptoskit_plugin_menu::{Command, MenuBar, Submenu, macos};
use std::rc::Rc;

let definition = MenuBar()
	.submenu(
		Submenu("file")
			.label("File")
			.item(Command("save").label("Save").accelerator("CmdOrCtrl+S")),
	)
	.build()?;
let application = macos::application(main_thread);
let lease = macos::ApplicationMenu::install(definition, &application)?;
```

Keep the lease for as long as the menu should stay installed; dropping it restores the previous menu when nothing else replaced the slot. Status-item / tray menus reuse the same `DesktopMenu` through `leptoskit_plugin_tray::TrayHost::set_menu` and do not take this lease. Menu is a host-owned desktop API, not a guest plugin: list it as a host dependency, not under `native.plugins`. See the [plugin inventory](plugin-inventory.md) for Windows and Linux parity.

## Desktop widgets

The [`home-widgets`](plugin-inventory.md) plugin works on macOS the same way as on iOS: it publishes a snapshot and asks WidgetKit to reload timelines. Your app still owns the Widget extension and needs an App Group for a shared suite. macOS Notification Center widgets are not generated for you.

## See more screens

The [examples](../examples.md) page shows seven screens rendered with AppKit. To open them yourself, clone the leptoskit repository and run the gallery:

```bash
cargo run -p gallery -- window counter            # a live, clickable window
cargo run -p gallery -- snapshot target/gallery   # one 2x PNG per screen
```

Snapshots draw the views offscreen, so they need no window and no Screen Recording permission.
