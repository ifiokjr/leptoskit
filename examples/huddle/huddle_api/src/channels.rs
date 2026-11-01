//! The sidebar: the channels you're in, with unread counts, and the people
//! in each.
//!
//! [`channels`] reads `Channel::query()` and lets the table's policy decide
//! what's visible. The policy asks `cx.exists(…)` about memberships, so
//! adding someone to a private channel re-runs their `channels` query and the
//! channel appears; removing them makes it vanish. Nothing here names a
//! subscription.

use huddle_db::Channel;
use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::Person;

/// A channel in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelCard {
	pub id: Id<Channel>,
	pub name: String,
	pub private: bool,
	/// Whether the signed-in person made it, and so decides who's in it.
	pub owned: bool,
	/// Messages from other people since the signed-in person last read it.
	pub unread: u64,
	/// The summary someone pinned, if any.
	pub summary: Option<String>,
}

/// Every channel the signed-in person can see, by name, with what's unread.
///
/// The unread counts and the message list update in the same transition,
/// so the badge never disagrees with the list beside it.
#[rpc::query]
pub async fn channels(cx: &rpc::QueryCx) -> rpc::Result<Vec<ChannelCard>> {
	let me = crate::me(cx.get())?;
	let channels = Channel::query()
		.order_by(|channel| channel.name.asc())
		.all(cx.db())
		.await?;
	let memberships = huddle_db::Membership::query()
		.filter(|member| member.user_id.eq(me))
		.all(cx.db())
		.await?;
	let mut cards = Vec::with_capacity(channels.len());

	for channel in channels {
		let read = memberships
			.iter()
			.find(|member| member.channel_id == channel.id)
			.map(|member| member.last_read_at);
		let unread = match read {
			Some(read) => {
				let id = channel.id;
				huddle_db::Message::query()
					.filter(|message| {
						message.channel_id.eq(id)
							& message.created_at.gt(read)
							& message.author_id.ne(me)
					})
					.count(cx.db())
					.await?
			}
			None => 0,
		};
		cards.push(ChannelCard {
			id: channel.id,
			owned: channel.created_by == me,
			name: channel.name,
			private: channel.private,
			unread,
			summary: channel.summary,
		});
	}

	Ok(cards)
}

/// Who's in a channel.
#[rpc::query]
pub async fn members(cx: &rpc::QueryCx, channel: Id<Channel>) -> rpc::Result<Vec<Person>> {
	let ids: Vec<Id<Account>> = huddle_db::Membership::query()
		.filter(|member| member.channel_id.eq(channel))
		.all(cx.db())
		.await?
		.into_iter()
		.map(|member| member.user_id)
		.collect();
	let users = huddle_db::User::query()
		.filter(|user| user.id.in_list(ids))
		.order_by(|user| user.name.asc())
		.all(cx.db())
		.await?;
	Ok(users
		.into_iter()
		.map(|user| {
			Person {
				id: user.id,
				name: user.name,
				is_admin: user.is_admin,
			}
		})
		.collect())
}

/// Starts a channel with the signed-in person and `members` in it.
#[rpc::mutation]
pub async fn create_channel(
	cx: &rpc::MutationCx,
	name: String,
	private: bool,
	#[serde(default)] members: Vec<Id<Account>>,
) -> rpc::Result<Id<Channel>> {
	let me = crate::me(cx.get())?;
	let name = name
		.trim()
		.trim_start_matches('#')
		.to_lowercase()
		.replace(' ', "-");
	let channel = huddle_db::NewChannel {
		name,
		private,
		created_by: me,
		summary: None,
	}
	.insert(cx.tx())
	.await?;
	let now = leptoskit_db::now();

	for user in std::iter::once(me).chain(members.into_iter().filter(|user| *user != me)) {
		huddle_db::NewMembership {
			channel_id: channel.id,
			user_id: user,
			last_read_at: now,
		}
		.insert(cx.tx())
		.await?;
	}

	Ok(channel.id)
}

/// Adds `user` to a channel the signed-in person made. The channel appears
/// in their sidebar at once.
#[rpc::mutation]
pub async fn add_member(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	user: Id<Account>,
) -> rpc::Result<()> {
	let existing = huddle_db::Membership::query()
		.filter(|member| member.channel_id.eq(channel) & member.user_id.eq(user))
		.exists(cx.tx())
		.await?;

	if !existing {
		huddle_db::NewMembership {
			channel_id: channel,
			user_id: user,
			last_read_at: leptoskit_db::now(),
		}
		.insert(cx.tx())
		.await?;
	}

	Ok(())
}

/// Takes `user` out of a channel. It vanishes from their sidebar, and from
/// every query that showed them its messages.
#[rpc::mutation]
pub async fn remove_member(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	user: Id<Account>,
) -> rpc::Result<()> {
	huddle_db::Membership::query()
		.filter(|member| member.channel_id.eq(channel) & member.user_id.eq(user))
		.delete(cx.tx())
		.await?;
	Ok(())
}

/// Marks a channel read up to now, for the signed-in person.
#[rpc::mutation]
pub async fn read_channel(cx: &rpc::MutationCx, channel: Id<Channel>) -> rpc::Result<()> {
	let me = crate::me(cx.get())?;
	huddle_db::Membership::update()
		.filter(|member| member.channel_id.eq(channel) & member.user_id.eq(me))
		.set(|member| member.last_read_at, leptoskit_db::now())
		.run(cx.tx())
		.await?;
	Ok(())
}
