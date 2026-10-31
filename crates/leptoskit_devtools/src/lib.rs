//! The agents that report a running leptoskit app to the local `DevTools` hub.
//!
//! The `host` agent reports a native surface: it opens a session, sends
//! snapshots, and long-polls for commands. The same protocol serves the web
//! agent and the native guests, which are added in later layers.

#[cfg(feature = "host")]
mod host;

#[cfg(feature = "host")]
pub use host::HostAgent;
#[cfg(feature = "host")]
pub use host::snapshot;
