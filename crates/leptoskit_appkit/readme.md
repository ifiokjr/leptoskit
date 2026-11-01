<h1 align="center">leptoskit_appkit</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_appkit</strong> is the macOS backend: it applies leptoskit mutation batches to real `NSView`s and measures text with AppKit's own fonts.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_appkit"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_appkit"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_appkit.svg"></a>
  <a href="https://docs.rs/leptoskit_appkit"><img alt="docs.rs" src="https://docs.rs/leptoskit_appkit/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

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

## How it fits

A `NativeSurface` built on it renders the same tree the web build renders as DOM. AppKit is the reference backend, used by the gallery and the generated macOS app; the other backends follow its contract.

## Use it

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

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_appkit)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [macOS](https://ifiokjr.github.io/leptoskit/platforms/macos.html)
