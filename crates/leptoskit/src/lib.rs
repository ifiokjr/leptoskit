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
//! - [`codepush`] — over-the-air releases, manifests, and binary deltas;
//! - `server` — the Axum server toolkit, with `axum`, `tokio`, `tower`,
//!   `tower_http`, and `leptos_axum` re-exported so a deployment depends on
//!   `leptoskit` alone (the `server` feature);
//! - `leptos`, `leptos_router`, and `leptos_meta` — re-exported behind the
//!   `leptos` feature, which the `csr`, `hydrate`, `ssr`, and `islands`
//!   features forward to, so an app picks its render mode without naming leptos
//!   in its own manifest;
//! - `query` — cached, deduplicated async state in the style of React Query:
//!   queries, infinite queries, and mutations (the `query` feature).
//! - `aria` — headless accessible components: disclosure, toggle buttons, tabs,
//!   labeled fields, and dialogs (the `aria` feature).
//! - `form` — headless form state with sync validation and aria field wiring
//!   (the `form` feature, which implies `aria`).
//! - `virtual_list` — headless list virtualization, ported from `TanStack
//!   Virtual` (the `virtual` feature).
//! - `table` — headless tables in the style of `TanStack Table`: typed columns,
//!   sorting, filtering, and pagination (the `table` feature).
//! - `rive` — bounded assets and browser Rive playback (the `rive` feature;
//!   rendering also needs `web`).
//! - `l10n` — typed translations in the style of Flutter's `gen_l10n`: the
//!   runtime behind the module `leptoskit l10n` writes from ARB files (the
//!   `l10n` feature).
//!
//! Screens are written as markup with [`view!`] and [`component`], or with
//! the builder functions they expand to. Both produce the same element tree,
//! which renders on the web and on every native backend:
//!
//! ```
//! use leptoskit::prelude::*;
//!
//! #[component]
//! pub fn Counter(count: usize, on_increment: impl Fn() + Send + Sync + 'static) -> impl IntoView {
//! 	view! {
//! 		<View class="gap-4 p-6">
//! 			<Text class="text-3xl font-bold">"Count: " {count}</Text>
//! 			<Button class="rounded-lg px-4 py-2" on:press=on_increment>
//! 				"Increment"
//! 			</Button>
//! 		</View>
//! 	}
//! }
//!
//! let screen = view! { <Counter count=3 on_increment=|| println!("increment") /> };
//! let builders = View()
//! 	.class("gap-4 p-6")
//! 	.child(Text("Count: 3").class("text-3xl font-bold"))
//! 	.child(Button("Increment").class("rounded-lg px-4 py-2"));
//! assert_eq!(screen.children[0].props, builders.children[0].props);
//! assert_eq!(screen.children[1].props, builders.children[1].props);
//! ```
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

pub use leptoskit_macros::component;
pub use leptoskit_macros::platform_module;
pub use leptoskit_macros::view;
pub use leptoskit_native::view::Children;
pub use leptoskit_native::view::Fragment;
pub use leptoskit_native::view::IntoChildren;
pub use leptoskit_native::view::IntoView;

/// Support for code that `view!` and `#[component]` generate. Not public API.
#[doc(hidden)]
pub mod __private {
	pub use leptoskit_native::Element;
	pub use leptoskit_native::view::__private::*;
}

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

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	//! Shared application rendering inside a native Wasm host.
	pub use leptoskit_native::guest::*;
}

#[cfg(feature = "web")]
pub mod web {
	//! Leptos rendering for the shared component tree.
	pub use leptoskit_native::web::*;
}

#[cfg(all(feature = "android", target_os = "android"))]
pub mod android {
	//! The Android Views backend, which renders native trees as Android
	//! widgets.
	pub use leptoskit_android::*;
}

#[cfg(all(feature = "appkit", target_os = "macos"))]
pub mod appkit {
	//! The AppKit backend, which renders native trees as macOS views.
	pub use leptoskit_appkit::*;
}

#[cfg(all(feature = "gtk", target_os = "linux"))]
pub mod gtk {
	//! The GTK4 backend, which renders native trees as Linux widgets.
	pub use leptoskit_gtk::*;
}

#[cfg(all(feature = "windows", target_os = "windows"))]
pub mod windows {
	//! The Win32 backend, which renders native trees as Windows controls.
	pub use leptoskit_windows::*;
}

#[cfg(all(feature = "uikit", target_os = "ios"))]
pub mod uikit {
	//! The UIKit backend, which renders native trees as iOS views.
	pub use leptoskit_uikit::*;
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

#[cfg(feature = "leptos")]
pub use leptos;
#[cfg(feature = "leptos")]
pub use leptos_meta;
#[cfg(feature = "leptos")]
pub use leptos_router;

#[cfg(feature = "server")]
pub mod server {
	//! The Axum server toolkit, and the crates every deployment is built
	//! from.
	//!
	//! A server binary that depends on `leptoskit` with the `server` feature
	//! needs no `axum`, `tokio`, `tower`, `tower-http`, or `leptos_axum` of
	//! its own: they are all here, at the versions leptoskit was built
	//! against, and `tokio` carries the features `#[tokio::main]` needs.
	//!
	//! ```no_run
	//! use leptoskit::server::axum::Router;
	//! use leptoskit::server::axum::routing::get;
	//! use leptoskit::server::tokio;
	//!
	//! #[tokio::main]
	//! async fn main() -> Result<(), leptoskit::server::ServeError> {
	//! 	let app = Router::new()
	//! 		.route("/_leptoskit/health", get(|| async { "ok" }))
	//! 		.fallback_service(leptoskit::server::static_files("dist".as_ref()));
	//! 	leptoskit::server::serve_router("127.0.0.1:3000".parse().expect("address"), app).await
	//! }
	//! ```
	pub use leptos_axum;
	pub use leptoskit_axum::*;
}

#[cfg(feature = "rpc")]
pub mod rpc {
	//! The server function API.
	pub use leptoskit_rpc::*;
}

#[cfg(feature = "use")]
pub mod hooks {
	//! Cross-platform reactive utilities, enabled by the `use` feature.
	pub use leptoskit_use::*;
}

#[cfg(feature = "query")]
pub mod query {
	//! Async state management: queries, infinite queries, and mutations.
	pub use leptoskit_query::*;
}

#[cfg(feature = "aria")]
pub mod aria {
	//! Headless accessible components: disclosure, toggle buttons, tabs,
	//! labeled fields, and dialogs wired for every platform.
	pub use leptoskit_aria::*;
}

#[cfg(feature = "form")]
pub mod form {
	//! Headless form state: declared fields, sync validation that runs when
	//! the user leaves a field or submits, and inputs wired through the aria
	//! fields.
	pub use leptoskit_form::*;
}

#[cfg(feature = "virtual")]
pub mod virtual_list {
	//! Headless list virtualization: windowing math and a scroll container
	//! that renders only what intersects the viewport.
	pub use leptoskit_virtual::*;
}

#[cfg(feature = "table")]
pub mod table {
	//! Headless tables: typed columns, sorting, filtering, pagination, and a
	//! rendered grid of ARIA roles.
	pub use leptoskit_table::*;
}

#[cfg(feature = "rive")]
pub mod rive {
	//! Bounded Rive assets and lifecycle-managed browser playback.
	pub use leptoskit_rive::*;
}

#[cfg(feature = "db")]
pub mod db {
	//! Database tables declared as Rust structs: typed IDs, client-safe public
	//! types, queries, and migrations.
	pub use leptoskit_db::*;
}

#[cfg(feature = "l10n")]
pub mod l10n {
	//! Typed translations: locales, negotiation, plural rules, and the
	//! reactive handle behind the generated messages module.
	pub use leptoskit_l10n::*;
}

/// The everything-import for apps: markup macros, components, layout
/// styles, elements, and configuration types in one glob.
///
/// leptos's prelude also exports `view` and `component`. In a file that
/// glob-imports both, import the macros you mean by name, such as
/// `use leptoskit::view;`, because a named import takes precedence over a
/// glob. leptoskit's `view!` renders shared components everywhere; leptos's
/// renders HTML on the web.
pub mod prelude {
	pub use leptoskit_config::LeptoskitConfig;
	pub use leptoskit_config::Platform;
	pub use leptoskit_macros::component;
	pub use leptoskit_macros::platform_module;
	pub use leptoskit_macros::view;
	pub use leptoskit_native::Color;
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
	pub use leptoskit_native::view::Children;
	pub use leptoskit_native::view::Fragment;
	pub use leptoskit_native::view::IntoChildren;
	pub use leptoskit_native::view::IntoView;
	pub use leptoskit_yoga::Align;
	pub use leptoskit_yoga::Dimension;
	pub use leptoskit_yoga::FlexDirection;
	pub use leptoskit_yoga::JustifyContent;
	pub use leptoskit_yoga::Style;
}
