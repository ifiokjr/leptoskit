//! Huddle's operations, shared by the server and every client.
//!
//! Each `#[rpc::query]`, `#[rpc::mutation]`, `#[rpc::action]`,
//! `#[rpc::job]`, and `#[rpc::channel]` here compiles to an argument struct
//! on every target. The server build (`server`) keeps the bodies; the
//! browser (`web`) and the native guest (`guest`) keep only the structs and
//! call them by name, so none of the database code reaches a client.
//!
//! | Module         | What it shows                                                                       |
//! | -------------- | ----------------------------------------------------------------------------------- |
//! | [`people`]     | Who's signed in, everyone's names, and an admin signing someone out everywhere      |
//! | [`channels`]   | A sidebar that follows memberships, with unread counts                              |
//! | [`messages`]   | History, and sending with a key the sender's screen matches its pending copy with  |
//! | [`room`]       | "Ada is typing…" and who's here, as a channel with presence                         |
//! | [`board`]      | A task board whose moves are serializable, so two people moving cards converge      |
//! | [`invites`]    | Invite by email: a job that turns *Sending…* into *Email sent*                      |
//! | [`reminders`]  | "Remind me in 10 minutes": a job enqueued to run later                              |
//! | [`tips`]       | A tip settled in checkpointed steps, with `on_failure`, an inbox notice, and Retry  |
//! | [`summary`]    | "Summarize this channel": an action calling a [`summary::Summarizer`]               |

#[cfg(feature = "server")]
use leptoskit_rpc as rpc;

pub mod board;
pub mod channels;
pub mod invites;
pub mod messages;
pub mod people;
pub mod reminders;
pub mod room;
#[cfg(feature = "server")]
pub mod services;
pub mod summary;
pub mod tips;

pub use board::*;
pub use channels::*;
pub use invites::*;
pub use messages::*;
pub use people::*;
pub use reminders::*;
pub use room::*;
pub use summary::*;
pub use tips::*;

/// The signed-in person, or `Unauthorized`.
#[cfg(feature = "server")]
fn me(
	signed_in: Option<&leptoskit_auth::SignedIn>,
) -> rpc::Result<leptoskit_db::Id<leptoskit_auth::Account>> {
	signed_in
		.map(|me| me.account)
		.ok_or_else(|| rpc::Error::unauthorized("sign in first"))
}

/// Refuses a mutation that only Huddle's own jobs may run, such as the one
/// that records a tip as confirmed. A job runs as its owner, but its caller
/// is the job, so a client calling it directly is turned away.
#[cfg(feature = "server")]
fn from_job(cx: &rpc::MutationCx) -> rpc::Result<()> {
	let caller = cx.identity().caller.as_deref().unwrap_or_default();

	if caller.starts_with("job:") {
		Ok(())
	} else {
		Err(rpc::Error::forbidden(
			"only Huddle's background jobs record this",
		))
	}
}
