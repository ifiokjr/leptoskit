//! "Ada is typing…" and the avatars of who's here: a channel, in the
//! `#[rpc::channel]` sense, per Huddle channel.
//!
//! Nothing here touches the database except the check on who may join.
//! Typing signals reach everyone else in the room at most once and are
//! never stored; presence is the set of connections in the room now, each
//! with the [`Here`] it joined with.

use huddle_db::Channel;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;

/// What someone in a room sends the others.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Signal {
	/// They're typing; show it for a few seconds.
	Typing {
		/// Their account, so a screen can leave out its own.
		user: String,
		/// Their name, as the indicator says it.
		name: String,
	},
	/// They sent or cleared what they were typing.
	Stopped {
		/// Their account.
		user: String,
	},
}

/// What each person in a room shows the others.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Here {
	/// Their account.
	pub user: String,
	/// Their name, for the avatar.
	pub name: String,
}

/// The live room beside one channel's messages.
#[rpc::channel(message = Signal, presence = Here, authorize = may_join)]
pub struct Room {
	pub channel: Id<Channel>,
}

/// Only the channel's members may join its room.
#[cfg(feature = "server")]
async fn may_join(cx: &rpc::QueryCx, room: &Room) -> rpc::Result<()> {
	let me = crate::me(cx.get())?;
	let channel = room.channel;
	let member = huddle_db::Membership::query()
		.filter(|member| member.channel_id.eq(channel) & member.user_id.eq(me))
		.exists(cx.db())
		.await?;

	if member {
		Ok(())
	} else {
		Err(rpc::Error::forbidden("join the channel first"))
	}
}
