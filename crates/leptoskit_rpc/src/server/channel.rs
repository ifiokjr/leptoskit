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
	/// The service whose live socket hosts it; `None` for the app's own.
	pub service: Option<&'static str>,
}

/// Decides whether the caller may join, with the join arguments decoded.
///
/// The context is owned, so the check can outlive the call that started it.
pub type Authorize = fn(Value, QueryCx) -> BoxFuture<'static, Result<()>>;

inventory::collect!(ChannelRegistration);

/// The app's channel called `name`: never a service's. See
/// [`registration_in`].
#[must_use]
pub fn registration(name: &str) -> Option<&'static ChannelRegistration> {
	registration_in(None, name)
}

/// The channel called `name` that `service`'s live socket hosts (`None`
/// for the app's own).
#[must_use]
pub fn registration_in(service: Option<&str>, name: &str) -> Option<&'static ChannelRegistration> {
	inventory::iter::<ChannelRegistration>()
		.find(|registration| registration.service == service && registration.name == name)
}
