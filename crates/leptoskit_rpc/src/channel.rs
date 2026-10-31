//! Channels: ephemeral, typed traffic between the people watching the same
//! thing, with presence.
//!
//! A channel is a join argument struct and two types:
//!
//! ```ignore
//! #[rpc::channel(message = Typing, presence = Presence, authorize = may_join)]
//! pub struct Chat {
//! 	pub channel: Id<Channel>,
//! }
//!
//! async fn may_join(cx: &rpc::QueryCx, chat: &Chat) -> rpc::Result<()> {
//! 	let me = cx.require_user::<User>()?;
//! 	Member::query()
//! 		.filter(|member| member.channel.eq(chat.channel) & member.user.eq(me))
//! 		.one(cx.db())
//! 		.await?;
//! 	Ok(())
//! }
//! ```
//!
//! Joining is checked once, as the caller, and re-checked when the
//! connection's identity changes. Signals travel at most once and are never
//! stored: anything that must survive belongs in a live table and a live
//! query. Presence is who is in the channel now, with whatever state each
//! connection joined with.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// One kind of channel: its join arguments, the messages it carries, and
/// the presence each member shows.
///
/// Declare one with `#[rpc::channel(...)]`, which implements this and
/// registers it with a server build.
pub trait Channel: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
	/// The channel's name, used when joining.
	const NAME: &'static str;
	/// What members send each other. Reaches every other member, at most
	/// once, and is never stored.
	type Message: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
	/// What a member shows the others while they are in the channel.
	type Presence: Serialize + DeserializeOwned + Default + Send + Sync + 'static;
}

/// Checks a channel's join arguments, where the macro names them.
#[doc(hidden)]
pub const fn assert_channel_argument<T: Serialize + DeserializeOwned>() {}

/// Checks a channel's message type, where the macro names it.
#[doc(hidden)]
pub const fn assert_channel_message<
	T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
>() {
}

/// Checks a channel's presence type, where the macro names it.
#[doc(hidden)]
pub const fn assert_channel_presence<
	T: Serialize + DeserializeOwned + Default + Send + Sync + 'static,
>() {
}
