//! The channels a server knows: each one's name and the check that decides
//! who may join.

use leptoskit_query::BoxFuture;
use serde_json::Value;

use super::QueryCx;
use crate::Result;

/// One `#[rpc::channel]`, as a server build registers it.
pub struct ChannelRegistration {
	/// The channel's name.
	pub name: &'static str,
	/// Decodes the join arguments, then runs the channel's `authorize`
	/// function as the connection's identity.
	#[doc(hidden)]
	pub authorize: Authorize,
}

/// Decides whether the caller may join, with the join arguments decoded.
///
/// The context is owned, so the check can outlive the call that started it.
pub type Authorize = fn(Value, QueryCx) -> BoxFuture<'static, Result<()>>;

inventory::collect!(ChannelRegistration);

/// The channel called `name`.
#[must_use]
pub fn registration(name: &str) -> Option<&'static ChannelRegistration> {
	inventory::iter::<ChannelRegistration>().find(|registration| registration.name == name)
}
