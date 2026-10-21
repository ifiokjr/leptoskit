//! The leptoskit toolkit: one dependency for building, deploying, and
//! shipping leptos apps to the web and to native platforms.
//!
//! Each layer is its own crate, re-exported here so apps depend on
//! `leptoskit` alone:
//!
//! - [`config`] — the `leptoskit.toml` project configuration;
//! - [`plugin`] — the plugin system, including full leptos plugin support;
//! - `hooks` — cross-platform reactive primitives, behind the `use` feature;
//! - [`yoga`] — the from-scratch flexbox and CSS grid layout engine;
//! - [`native`] — the React Native-style native renderer;
//! - `appkit` — the AppKit backend that renders native trees as macOS views
//!   (the `appkit` feature, on macOS only);
//! - [`wasm`] — the on-device wasm runtime host behind code push;
//! - [`codepush`] — over-the-air releases, manifests, and binary deltas.
//!
//! ```no_run
//! use leptoskit::native::Element;
//! use leptoskit::native::HeadlessBackend;
//! use leptoskit::native::NativeSurface;
//!
//! let mut surface = NativeSurface::new(HeadlessBackend::new());
//! surface.set_viewport(390.0, 844.0);
//! surface.set_root(&Element::view().child(Element::text("hello from leptoskit")));
//! ```
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod config {
	//! Project configuration.
	pub use leptoskit_config::*;
}

pub mod plugin {
	//! The plugin system.
	pub use leptoskit_plugin::*;
}

pub mod yoga {
	//! The layout engine.
	pub use leptoskit_yoga::*;
}

pub mod native {
	//! The native renderer.
	pub use leptoskit_native::*;
}

pub mod components {
	//! Built-in components shared by native and web screens.
	pub use leptoskit_native::components::*;
}

#[cfg(feature = "web")]
pub mod web {
	//! Leptos rendering for the shared component tree.
	pub use leptoskit_native::web::*;
}

#[cfg(all(feature = "appkit", target_os = "macos"))]
pub mod appkit {
	//! The AppKit backend, which renders native trees as macOS views.
	pub use leptoskit_appkit::*;
}

#[cfg(feature = "wasm")]
pub mod wasm {
	//! The on-device wasm runtime host.
	pub use leptoskit_wasm::*;
}

#[cfg(feature = "codepush")]
pub mod codepush {
	//! Code push releases and updates.
	pub use leptoskit_codepush::*;
}

#[cfg(feature = "fns")]
pub mod fns {
	//! The server function API.
	pub use leptoskit_fns::*;
}

#[cfg(feature = "use")]
pub mod hooks {
	//! Cross-platform reactive utilities, enabled by the `use` feature.
	pub use leptoskit_use::*;
}

/// The everything-import for apps: layout styles, elements, and
/// configuration types in one glob.
pub mod prelude {
	pub use leptoskit_config::LeptoskitConfig;
	pub use leptoskit_config::Platform;
	pub use leptoskit_native::Element;
	pub use leptoskit_native::EventOutcome;
	pub use leptoskit_native::NativeEvent;
	pub use leptoskit_native::NativeEventSlot;
	pub use leptoskit_native::NativeKind;
	pub use leptoskit_native::NativeProps;
	pub use leptoskit_native::NativeSurface;
	pub use leptoskit_native::components::*;
	pub use leptoskit_native::tailwind::ClassError;
	pub use leptoskit_native::tailwind::StyleContext;
	pub use leptoskit_native::tailwind::Tailwind;
	pub use leptoskit_yoga::Align;
	pub use leptoskit_yoga::Dimension;
	pub use leptoskit_yoga::FlexDirection;
	pub use leptoskit_yoga::JustifyContent;
	pub use leptoskit_yoga::Style;
}
