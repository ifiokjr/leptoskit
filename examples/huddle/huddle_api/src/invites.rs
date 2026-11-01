//! Invite by email.
//!
//! [`invite`] writes the invite as *Sending…* and enqueues [`SendInvite`] in
//! the same transaction: if the mutation fails, no email goes out, and if it
//! commits, the email is never lost. The job sends the email, marks the row
//! *Email sent*, and leaves the inviter a notice. `invites` is a live
//! table, so the row changes on every screen showing it.
//!
//! When the invited address signs in for the first time, it joins the
//! channel (see `huddle_db`'s sign-up hook).

use huddle_db::Channel;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// An invitation, as the channel's screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InviteView {
	pub id: Id<huddle_db::Invite>,
	pub email: String,
	/// `sending`, `sent`, or `failed`.
	pub status: String,
}

/// The invitations sent for a channel, newest first.
#[rpc::query]
pub async fn invites(cx: &rpc::QueryCx, channel: Id<Channel>) -> rpc::Result<Vec<InviteView>> {
	let invites = huddle_db::Invite::query()
		.filter(|invite| invite.channel_id.eq(channel))
		.order_by(|invite| invite.created_at.desc())
		.limit(20)
		.all(cx.db())
		.await?;
	Ok(invites
		.into_iter()
		.map(|invite| {
			InviteView {
				id: invite.id,
				email: invite.email,
				status: invite.status,
			}
		})
		.collect())
}

/// Invites `email` to a channel. Returns at once; the email follows.
#[rpc::mutation]
pub async fn invite(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	email: String,
) -> rpc::Result<Id<huddle_db::Invite>> {
	let me = crate::me(cx.get())?;
	let email = email.trim().to_lowercase();

	if !email.contains('@') || email.len() < 3 {
		return Err(rpc::Error::invalid("that isn't an email address"));
	}

	let invite = huddle_db::NewInvite {
		channel_id: channel,
		invited_by: me,
		email,
		status: "sending".to_owned(),
	}
	.insert(cx.tx())
	.await?;
	cx.enqueue(SendInvite { invite: invite.id }).await?;
	Ok(invite.id)
}

/// Sends an invitation. Runs on a worker; nobody waits for it.
#[rpc::job(queue = "mail", retries = 8, backoff = exponential(1s, 1m))]
pub async fn send_invite(cx: &rpc::JobCx, invite: Id<huddle_db::Invite>) -> rpc::Result<()> {
	let row = huddle_db::Invite::get(cx.db(), invite).await?;
	let channel = Channel::get(cx.db(), row.channel_id).await?;
	let inviter = huddle_db::User::get(cx.db(), row.invited_by).await?;
	let services = crate::services::get();
	let mail = leptoskit_auth::Mail {
		to: row.email.clone(),
		subject: format!(
			"{} invited you to #{} on Huddle",
			inviter.name, channel.name
		),
		text: format!(
			"{} invited you to #{} on Huddle. Sign in with this address and the channel is \
			 waiting for you.",
			inviter.name, channel.name
		),
	};

	cx.step("send", || {
		async move {
			tokio::time::sleep(services.mail_delay).await;
			services
				.mailer
				.send(mail)
				.await
				.map_err(|error| rpc::Error::retry(error.to_string()))
		}
	})
	.await?;
	cx.mutate(MarkInvite {
		invite,
		status: "sent".to_owned(),
	})
	.await?;
	cx.notify(
		cx.owner(),
		rpc::Notice::new(format!("Invite sent to {}", row.email))
			.body(format!("They'll join #{} when they sign in.", channel.name))
			.kind(rpc::NoticeKind::Success),
	)
	.await?;
	Ok(())
}

/// Records how an invitation's email went. Only [`SendInvite`] runs it.
#[rpc::mutation]
pub async fn mark_invite(
	cx: &rpc::MutationCx,
	invite: Id<huddle_db::Invite>,
	status: String,
) -> rpc::Result<()> {
	crate::from_job(cx)?;
	huddle_db::Invite::update()
		.filter(|row| row.id.eq(invite))
		.set(|row| row.status, status)
		.run(cx.tx())
		.await?;
	Ok(())
}
