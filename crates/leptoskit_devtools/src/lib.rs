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
//! The `host` agent reports a native host: a `Reporter` opens a session,
//! keeps a `Mirror` of each guest's tree from the mutations its renderer
//! applies, sends snapshots, patches, and commits from a thread of its own,
//! and long-polls for commands. The `web` agent reports a browser page.
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
//! On the web that is the web agent. In a native guest it is the `guest`
//! agent, which sends them through the host's `DevTools` plugin
//! (`leptoskit_plugin_devtools`); the host merges them into the session it
//! reports, and passes the hub's commands for them back.

#[cfg(feature = "form")]
mod form;
#[cfg(any(test, all(feature = "guest", target_arch = "wasm32")))]
mod guest;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
mod host;
#[cfg(feature = "query")]
mod query;
mod registry;
#[cfg(any(
	test,
	all(any(feature = "web", feature = "guest"), target_arch = "wasm32")
))]
mod report;
mod router;
mod state;
#[cfg(test)]
mod tests;
mod value;

#[cfg(feature = "form")]
pub use form::track_form;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Batch;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Change;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::CommitRecord;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Guest;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Highlight;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::HostAgent;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Mirror;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Reporter;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::SessionGone;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::Source;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::loopback_hub;
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
/// themselves. A native guest starts its agent here when its host offers the
/// `DevTools` plugin, which only a host `leptoskit dev` built does; the host
/// names the app, so `app` is unused there. The first hook starts it too, so
/// a guest needs this call only to report before it calls any hook.
pub fn install(app: &str) {
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	if let Some(hub) = hub() {
		web::install(hub, app);
		return;
	}
	#[cfg(all(feature = "guest", target_arch = "wasm32"))]
	guest::start();
	let _ = app;
}
