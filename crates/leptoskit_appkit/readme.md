<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_appkit

> The leptoskit AppKit backend: renders leptoskit element trees as real macOS views.

`AppKitBackend` implements the [`leptoskit_native`] `NativeBackend` contract over AppKit. A `NativeSurface` built on it diffs element trees, lays them out with [`leptoskit_yoga`], and commits the result straight into `NSView`s — there is no web view anywhere.

| Element kind                                   | AppKit widget                                                      |
| ---------------------------------------------- | ------------------------------------------------------------------ |
| `View`, `Custom`                               | a flipped, layer-backed `NSView`                                   |
| `Text`, `TextRun`                              | a non-editable, non-bezeled `NSTextField` label                    |
| `Button`                                       | an `NSButton` whose clicks dispatch `NativeEvent::Press`           |
| `TextInput`                                    | an editable `NSTextField` (change, submit, focus, and blur events) |
| `PasswordInput`                                | an `NSSecureTextField` with the same editing events                |
| `Switch`                                       | an `NSSwitch` proposing typed boolean values                       |
| `Checkbox`, `Radio`                            | native `NSButton` switch/radio controls                            |
| `Slider`                                       | an `NSSlider` proposing values on the shared step grid             |
| `LinearProgressIndicator`, `ActivityIndicator` | native `NSProgressIndicator` bars and spinners                     |
| `Vector`                                       | a flipped view with one `CAShapeLayer` per path                    |
| `Image`                                        | an `NSImageView` loading `source` as a file path or `file://` URL  |
| `ScrollView`                                   | an `NSScrollView` over a flipped document view                     |

Backgrounds, rounded corners (clipping their content), borders, and opacity are applied on each view's layer. Fonts come from the system font at the requested size and weight, and `AppKitTextMeasurer` measures text with the same metrics an `NSTextField` cell uses, so layout reserves exactly the space AppKit draws into.

Switches, checkboxes, radios, sliders, and text fields remain controlled: a callback proposes a value, and the widget restores the latest application-owned value when the callback finishes. Callbacks can synchronously update or delete the firing widget. Radio arrow keys move focus and propose selection within the explicit group across layout wrappers, skipping hidden and disabled options. Slider arrows advance by the declared step, or one percent of the range for a continuous slider. Native progress animations stop when hidden, stopped, deleted, or when their backend is dropped.

## Usage

```rust
use leptoskit_appkit::AppKitBackend;
use leptoskit_appkit::MainThreadMarker;
use leptoskit_native::Element;
use leptoskit_native::NativeSurface;

let mtm = MainThreadMarker::new().expect("AppKit runs on the main thread");
let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
surface.set_viewport(390.0, 844.0);
surface.set_root(&Element::view().child(Element::text("hello from AppKit")));

// Render offscreen at 2x, with no window and no Screen Recording permission…
let root = surface.root().expect("committed");
std::fs::write("hello.png", surface.backend().snapshot_png(root, 2.0)?)?;

// …or show it for real.
let _window = surface.backend().open_window(root, "Hello");
leptoskit_appkit::run_application(mtm);
```

## Events

Controls report interaction through one listener installed with `set_event_listener`. It receives the committed node id and the event, runs on the main thread from inside the AppKit event loop (never during a mutation batch), and usually forwards to `NativeSurface::dispatch_event` before re-rendering. The [`gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery) example's `window` command shows the whole loop.

## Limits

- Text is drawn at the top of its frame and ignores the yoga padding of the `Text` node itself; put padding on a wrapping `View`.
- `Image` sources other than file paths and `file://` URLs are left empty, as are files that are missing or not images.
- Windows opened by `open_window` are fixed-size, matching the surface's viewport.
- Without an explicit color, text uses AppKit's dynamic label colors, which follow the light or dark appearance.

## Other platforms

AppKit exists only on macOS. On every other target the crate compiles to an empty library, so workspaces that depend on it still build and test on Linux and Windows.

[`leptoskit_native`]: https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_native
[`leptoskit_yoga`]: https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga
