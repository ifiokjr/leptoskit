# macOS

On macOS, your screens are drawn with AppKit: layer-backed `NSView`s for containers, `NSTextField` for text and inputs, `NSButton`, `NSImageView`, and `NSScrollView`. Text is measured with the same metrics AppKit draws with, so the layout reserves exactly the space each label needs. Nothing runs in a web view.

macOS is leptoskit's reference backend: every screenshot on this site is a leptoskit screen committed as real AppKit views.

## Build and run

You need a Mac with Xcode or the Xcode command line tools, and the WebAssembly target:

```bash
rustup target add wasm32-unknown-unknown
leptoskit build --platform macos
open .leptoskit/macos/App.app
```

The build compiles your app to WebAssembly, builds the AppKit host for your Mac's architecture, and assembles `.leptoskit/macos/App.app`. It signs the app ad hoc, which is enough to run it on your own Mac. To distribute it, sign it with your Developer ID and notarize it.

The `macos/` folder is yours: `App.swift` starts the app and owns its window, `Info.plist` holds its name and version, and `Cargo.toml` lists native Rust dependencies such as [runtime plugins](plugins.md).

## How it differs from the other platforms

- The generated app draws seven components: `View`, `Text`, `Button`, `Pressable`, `TextInput`, `ScrollView`, and `Image`. Others stop it with an "unsupported widget" error. The `leptoskit_appkit` backend crate draws every component, including `NSSwitch`, `NSSlider`, and `NSProgressIndicator`; the [examples](../examples.md) use it. See [generated apps and backend crates](readme.md#generated-apps-and-backend-crates).
- The generated app doesn't report scroll events yet, so a [virtualized list](../guide/17-virtualized-lists.md) stays on its first window there. The backend crate reports them.
- Pressing a button with the keyboard (Space, with Full Keyboard Access on) or through VoiceOver activates it without showing its [pressed style](../guide/07-shared-components.md#style-the-pressed-state). A mouse press shows it.
- `Image` loads local paths and `file://` URLs, not web URLs.
- The generated macOS app doesn't check for [code push](../guide/05-code-push.md) updates; ship a new build to update it. The iOS and Android apps do.
- AppKit follows the system's light or dark appearance until the app calls `NativeSurface::set_dark_mode`. An app with a fixed light theme should call `set_dark_mode(false)` before it shows its first screen.

## See more screens

The [examples](../examples.md) page shows seven screens rendered with AppKit. To open them yourself, clone the leptoskit repository and run the gallery:

```bash
cargo run -p gallery -- window counter            # a live, clickable window
cargo run -p gallery -- snapshot target/gallery   # one 2x PNG per screen
```

Snapshots draw the views offscreen, so they need no window and no Screen Recording permission.
