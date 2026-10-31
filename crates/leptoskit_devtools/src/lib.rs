//! The agents that report a running leptoskit app to the local `DevTools` hub.
//!
//! The `host` agent reports a native surface: it opens a session, sends
//! snapshots, and long-polls for commands. The same protocol serves the web
//! agent and the native guests, which are added in later layers.

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
mod host;

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::HostAgent;
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub use host::snapshot;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub use web::install;
