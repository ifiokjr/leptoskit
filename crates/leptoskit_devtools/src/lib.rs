//! The agents that report a running leptoskit app to the local `DevTools` hub.
//!
//! Apps call [`install`] once at startup, behind their `devtools` feature:
//!
//! ```ignore
//! #[cfg(feature = "devtools")]
//! leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
//! ```
//!
//! `leptoskit dev` compiles the hub's address in, so a build without one
//! (such as `leptoskit build`) starts nothing.
//!
//! The `host` agent reports a native surface: it opens a session, sends
//! snapshots, and long-polls for commands. The `web` agent reports a browser
//! page.

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
mod host;

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::HostAgent;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::snapshot;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web;

/// The hub `leptoskit dev` compiled into this build, if any.
#[must_use]
pub const fn hub() -> Option<&'static str> {
	option_env!("LEPTOSKIT_DEVTOOLS_HUB")
}

/// Start reporting this app, named `app`, to the hub `leptoskit dev` runs.
///
/// It does nothing in a build without a hub address, or on a platform whose
/// agent the host starts instead: native hosts report their surface
/// themselves.
pub fn install(app: &str) {
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	if let Some(hub) = hub() {
		web::install(hub, app);
	}
	let _ = app;
}
