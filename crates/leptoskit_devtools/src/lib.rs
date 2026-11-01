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
//!
//! The element tree needs nothing more. What only the app knows, it hands
//! over with a hook, each of which does nothing outside a `leptoskit dev`
//! build:
//!
//! - [`track_router`] reports the current route and navigates for the
//!   `DevTools`;
//! - [`expose`] names a signal, which the `DevTools` show and edit (only state
//!   named this way appears);
//! - `track_form` reports a `leptoskit_form` form, read-only (the `form`
//!   feature);
//! - `track_queries` reports a `leptoskit_query` cache, whose queries the
//!   `DevTools` refetch, invalidate, and edit (the `query` feature).
//!
//! On the app's path a hook only notes that something changed. The agent
//! serializes values in its debounced flush and sends the ones that changed.
//! The web agent reports them today; in a native guest the hooks compile and
//! do nothing yet.

#[cfg(feature = "form")]
mod form;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
mod host;
#[cfg(feature = "query")]
mod query;
mod registry;
#[cfg(any(test, all(feature = "web", target_arch = "wasm32")))]
mod report;
mod router;
mod state;
#[cfg(test)]
mod tests;
mod value;

#[cfg(feature = "form")]
pub use form::track_form;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::HostAgent;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::snapshot;
#[cfg(feature = "query")]
pub use query::track_queries;
pub use router::track_router;
pub use state::expose;

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
