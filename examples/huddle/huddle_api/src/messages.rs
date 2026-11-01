//! A channel's messages, and sending one.
//!
//! The sender's screen shows a message the moment they press Send, greyed,
//! under the `client_key` it made for it. [`send_message`] stores that key
//! with the row, so when the live [`messages`] result includes it, the
//! screen swaps the grey copy for the real one without a flicker: the
//! sender reads their own write. Sending the same key twice, as a client
//! does when the network drops the answer, stores one message.

use huddle_db::Channel;
use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// How many messages a channel shows at most; older ones are history.
pub const PAGE: u32 = 50;

/// One message, as every screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageView {
	pub id: Id<huddle_db::Message>,
	pub author_id: Id<Account>,
	pub author: String,
	pub body: String,
	pub client_key: String,
	/// RFC 3339, in UTC.
	pub sent_at: String,
}

/// The latest `limit` messages in a channel, oldest first, or the `limit`
/// before `before` when scrolling back through history.
#[rpc::query]
pub async fn messages(
	cx: &rpc::QueryCx,
	channel: Id<Channel>,
	limit: u32,
	#[serde(default)] before: Option<Id<huddle_db::Message>>,
) -> rpc::Result<Vec<MessageView>> {
	let limit = u64::from(limit.clamp(1, 200));
	let older_than = match before {
		Some(id) => {
			huddle_db::Message::find(cx.db(), id)
				.await?
				.map(|message| message.created_at)
		}
		None => None,
	};
	let mut rows = huddle_db::Message::query()
		.filter(|message| {
			let here = message.channel_id.eq(channel);
			match older_than {
				Some(at) => here & message.created_at.lt(at),
				None => here,
			}
		})
		.order_by(|message| message.created_at.desc())
		.limit(limit)
		.all(cx.db())
		.await?;
	rows.reverse();

	let mut authors: Vec<Id<Account>> = rows.iter().map(|message| message.author_id).collect();
	authors.sort_unstable();
	authors.dedup();
	let names: std::collections::HashMap<Id<Account>, String> = huddle_db::User::query()
		.filter(|user| user.id.in_list(authors))
		.all(cx.db())
		.await?
		.into_iter()
		.map(|user| (user.id, user.name))
		.collect();

	Ok(rows
		.into_iter()
		.map(|message| {
			MessageView {
				id: message.id,
				author: names
					.get(&message.author_id)
					.cloned()
					.unwrap_or_else(|| "Someone".to_owned()),
				author_id: message.author_id,
				body: message.body,
				client_key: message.client_key,
				sent_at: message.created_at.to_rfc3339(),
			}
		})
		.collect())
}

/// Posts `body` in a channel as the signed-in person.
///
/// `client_key` is the sender's own name for the message: the same key twice
/// stores it once.
#[rpc::mutation]
pub async fn send_message(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	body: String,
	client_key: String,
) -> rpc::Result<Id<huddle_db::Message>> {
	let me = crate::me(cx.get())?;
	let body = body.trim().to_owned();

	if body.is_empty() {
		return Err(rpc::Error::invalid("a message needs some text"));
	}

	let existing = huddle_db::Message::query()
		.filter(|message| message.author_id.eq(me) & message.client_key.eq(client_key.clone()))
		.first(cx.tx())
		.await?;

	if let Some(message) = existing {
		return Ok(message.id);
	}

	let message = huddle_db::NewMessage {
		channel_id: channel,
		author_id: me,
		body,
		client_key,
	}
	.insert(cx.tx())
	.await?;
	// Your own message isn't unread for you.
	huddle_db::Membership::update()
		.filter(|member| member.channel_id.eq(channel) & member.user_id.eq(me))
		.set(|member| member.last_read_at, message.created_at)
		.run(cx.tx())
		.await?;
	Ok(message.id)
}
