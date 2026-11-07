#![cfg_attr(docsrs, feature(doc_cfg))]

//! <!-- {=serviceWorkerSummary|trim|linePrefix:"//! ":true} -->
//!
//! Register a worker script, wait for control, exchange messages, open Cache
//! Storage, describe caching strategies, and manage Web Push subscriptions.
//! Outside a browser every call returns `Unavailable`.
//!
//! <!-- {/serviceWorkerSummary} -->
//!
//! This is the page side of a Progressive Web App's service worker. Ship a
//! worker script with your web assets (for example `/sw.js`), then:
//!
//! ```ignore
//! use leptoskit::service_worker::prelude::*;
//!
//! let registration = register("/sw.js", RegisterOptions::default()).await?;
//! let _ = ready().await?;
//!
//! send(&WorkerMessage::SetCacheRules {
//! 	rules: vec![
//! 		CacheRule::new("static-v1", Strategy::cache_first()).prefixes(["/assets/"]),
//! 		CacheRule::new("pages", Strategy::network_first_within(2_000)),
//! 	],
//! })?;
//!
//! let push = registration.push_manager()?;
//! let subscription = push.subscribe(&PushOptions::new(vapid_public_key)).await?;
//! ```
//!
//! Native apps, the terminal, and extension backgrounds have no page-controlled
//! service worker; those calls return [`Error::Unavailable`]. Extension
//! background workers use [`leptoskit_extension`](https://docs.rs/leptoskit_extension)
//! instead.

mod cache;
mod js;
mod messages;
mod push;
mod register;
mod strategy;

pub use cache::Cache;
pub use cache::delete;
pub use cache::has;
pub use cache::keys;
pub use cache::open;
pub use js::Listener;
pub use messages::on_controller_change;
pub use messages::on_message;
pub use messages::post_message;
pub use messages::send;
pub use push::PushKeys;
pub use push::PushManager;
pub use push::PushOptions;
pub use push::PushSubscription;
pub use register::RegisterOptions;
pub use register::Registration;
pub use register::UpdateViaCache;
pub use register::Worker;
pub use register::controller;
pub use register::get_registration;
pub use register::ready;
pub use register::register;
pub use strategy::CacheRule;
pub use strategy::Source;
pub use strategy::Strategy;
pub use strategy::WorkerMessage;

/// Why a service worker API call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// This context has no such API (not a secure browser page, or the
	/// browser lacks it). The value names the API.
	#[error("`{0}` isn't available in this browser")]
	Unavailable(String),
	/// The browser refused the call, with its reason.
	#[error("{0}")]
	Browser(String),
	/// A value couldn't cross between Rust and JavaScript.
	#[error("couldn't convert a value: {0}")]
	Json(String),
}

/// A result from the service worker API.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The names an app typically imports together.
pub mod prelude {
	pub use crate::CacheRule;
	pub use crate::Error;
	pub use crate::PushOptions;
	pub use crate::RegisterOptions;
	pub use crate::Result;
	pub use crate::Strategy;
	pub use crate::WorkerMessage;
	pub use crate::controller;
	pub use crate::on_message;
	pub use crate::ready;
	pub use crate::register;
	pub use crate::send;
}
