//! The leptoskit AppKit backend: renders leptoskit element trees as real
//! macOS views.
//!
//! [`AppKitBackend`] implements
//! [`NativeBackend`](leptoskit_native::NativeBackend),
//! so a [`NativeSurface`](leptoskit_native::NativeSurface) over it diffs,
//! lays out, and commits straight into `NSView`s — no web view anywhere:
//!
//! | Element kind | AppKit widget |
//! | --- | --- |
//! | `View`, `Custom` | a flipped, layer-backed `NSView` |
//! | `Text`, `TextRun` | a non-editable, non-bezeled `NSTextField` label |
//! | `Button` | an `NSButton` whose action dispatches [`NativeEvent::Press`] |
//! | `TextInput` | an editable `NSTextField` |
//! | `PasswordInput` | an `NSSecureTextField` |
//! | `Switch` | an `NSSwitch` |
//! | `Checkbox`, `Radio` | native `NSButton` switch/radio controls |
//! | `Slider` | an `NSSlider` with shared step-grid keyboard increments |
//! | `LinearProgressIndicator`, `ActivityIndicator` | an `NSProgressIndicator` bar/spinner |
//! | `Image` | an `NSImageView` loading `source` as a file path |
//! | `ScrollView` | an `NSScrollView` over a flipped document view, reporting [`NativeEvent::Scroll`] |
//! | `Vector` | a flipped view with one `CAShapeLayer` per path |
//!
//! Every container is flipped, so frames from the layout engine apply as they
//! are: top-left origin, parent-relative coordinates.
//!
//! Text is measured by [`AppKitTextMeasurer`] with the same system font the
//! widgets draw with, so layout matches what AppKit renders.
//!
//! Controls propose typed values to application state. Their action targets
//! restore the latest committed value after each callback, including rejected
//! updates and callbacks that synchronously rebuild or delete the firing
//! widget. Radio arrow keys follow explicit groups across layout wrappers,
//! skipping disabled and hidden members. Native loading animations stop when
//! hidden, stopped, deleted, or when the backend is dropped.
//!
//! [`AppKitBackend::snapshot_png`] renders a view tree offscreen into a PNG,
//! without a window and without Screen Recording permission, and
//! [`AppKitBackend::open_window`] with [`run_application`] shows it for real.
//!
//! ```no_run
//! # #[cfg(target_os = "macos")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use leptoskit_appkit::AppKitBackend;
//! use leptoskit_appkit::MainThreadMarker;
//! use leptoskit_native::Element;
//! use leptoskit_native::NativeSurface;
//!
//! let mtm = MainThreadMarker::new().expect("AppKit runs on the main thread");
//! let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
//! surface.set_viewport(390.0, 844.0);
//! surface.set_root(&Element::view().child(Element::text("hello from AppKit")));
//! let root = surface.root().expect("committed");
//! let png = surface.backend().snapshot_png(root, 2.0)?;
//! std::fs::write("hello.png", png)?;
//! # Ok(())
//! # }
//! # #[cfg(not(target_os = "macos"))]
//! # fn main() {}
//! ```
//!
//! # Other platforms
//!
//! AppKit exists only on macOS. On every other target this crate compiles to
//! an empty library, so a workspace that depends on it still builds and tests
//! on Linux and Windows.
//!
//! [`NativeEvent::Press`]: leptoskit_native::NativeEvent::Press
//! [`NativeEvent::Scroll`]: leptoskit_native::NativeEvent::Scroll
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]
// The workspace denies `unsafe_code`; this crate is the one exception. AppKit
// is an Objective-C framework, and every call into it goes through `objc2`,
// whose message sends, class definitions, and extern statics are `unsafe`
// because the compiler cannot check the Objective-C side of the contract.
// Each unsafe block carries a `SAFETY:` comment saying why it holds.
#![allow(unsafe_code)]

#[cfg(target_os = "macos")]
mod accessibility;
#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod backend;
#[cfg(target_os = "macos")]
mod measure;
#[cfg(target_os = "macos")]
mod pressable;
#[cfg(target_os = "macos")]
mod rive;
#[cfg(target_os = "macos")]
mod snapshot;
#[cfg(target_os = "macos")]
mod text;
#[cfg(target_os = "macos")]
mod vector;
#[cfg(target_os = "macos")]
mod views;

#[cfg(target_os = "macos")]
pub use app::run_application;
#[cfg(target_os = "macos")]
pub use backend::AppKitBackend;
#[cfg(target_os = "macos")]
pub use backend::EventListener;
#[cfg(target_os = "macos")]
pub use measure::AppKitTextMeasurer;
/// The proof of running on the main thread that AppKit requires.
#[cfg(target_os = "macos")]
pub use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
pub use snapshot::SnapshotError;
